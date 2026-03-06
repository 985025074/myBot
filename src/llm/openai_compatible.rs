use std::io::{BufRead, BufReader};

use reqwest::header::CONTENT_TYPE;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};

use crate::{
    app::AppResult,
    config::LlmConfig,
    llm::{
        ChatMessage, ErrorEnvelope, TextPart, extract_text_parts, missing_api_key_error,
        wrap_thinking_text,
    },
};

#[derive(Debug, Clone)]
pub struct OpenAiCompatibleClient {
    http: Client,
    config: LlmConfig,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: AssistantMessage,
}

#[derive(Debug, Deserialize)]
struct AssistantMessage {
    content: Option<Content>,
    #[serde(default)]
    reasoning_content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StreamChatCompletionResponse {
    choices: Vec<StreamChoice>,
}

#[derive(Debug, Deserialize)]
struct StreamChoice {
    delta: StreamDelta,
}

#[derive(Debug, Deserialize)]
struct StreamDelta {
    content: Option<Content>,
    #[serde(default)]
    reasoning_content: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Content {
    Text(String),
    Parts(Vec<TextPart>),
}

#[derive(Debug, Serialize)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<ChatMessage>,
    temperature: f32,
    max_tokens: u32,
    stream: bool,
}

impl OpenAiCompatibleClient {
    pub fn new(http: Client, config: LlmConfig) -> Self {
        Self { http, config }
    }

    #[allow(dead_code)]
    pub fn send_chat(&self, history: &[ChatMessage]) -> AppResult<String> {
        self.send_chat_streaming(history, |_| Ok(()))
    }

    pub fn send_chat_streaming(
        &self,
        history: &[ChatMessage],
        mut on_chunk: impl FnMut(&str) -> AppResult<()>,
    ) -> AppResult<String> {
        let api_key = self
            .config
            .resolve_api_key()
            .ok_or_else(|| missing_api_key_error(&self.config.api_key_env))?;

        let mut messages = Vec::with_capacity(history.len() + 1);
        if !self.config.system_prompt.trim().is_empty() {
            messages.push(ChatMessage {
                role: "system".to_string(),
                content: self.config.system_prompt.clone(),
            });
        }
        messages.extend(history.iter().cloned());

        let request = ChatCompletionRequest {
            model: self.config.model.clone(),
            messages,
            temperature: self.config.temperature,
            max_tokens: self.config.max_tokens,
            stream: true,
        };

        let url = format!("{}/chat/completions", self.config.base_url.trim_end_matches('/'));
        let response = self
            .http
            .post(url)
            .bearer_auth(api_key)
            .header("Content-Type", "application/json")
            .json(&request)
            .send()?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text()?;
            if let Ok(error) = serde_json::from_str::<ErrorEnvelope>(&body) {
                return Err(error.error.message.into());
            }
            return Err(format!("LLM request failed with status {status}: {body}").into());
        }

        let is_sse = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.contains("text/event-stream"))
            .unwrap_or(false);

        if !is_sse {
            let response: ChatCompletionResponse = response.json()?;
            let choice = response
                .choices
                .into_iter()
                .next()
                .ok_or("LLM response did not contain any choices")?;

            let text = assistant_message_to_text(choice.message);
            if text.trim().is_empty() {
                return Err("LLM response text was empty".into());
            }

            on_chunk(&text)?;
            return Ok(text);
        }

        let mut text = String::new();
        let mut reader = BufReader::new(response);
        let mut line = String::new();
    let mut in_reasoning_block = false;

        loop {
            line.clear();
            if reader.read_line(&mut line)? == 0 {
                break;
            }

            let trimmed = line.trim();
            let Some(data) = trimmed.strip_prefix("data:") else {
                continue;
            };
            let data = data.trim();
            if data.is_empty() {
                continue;
            }
            if data == "[DONE]" {
                break;
            }

            let Some(choice) = serde_json::from_str::<StreamChatCompletionResponse>(data)
                .ok()
                .and_then(|response| response.choices.into_iter().next())
            else {
                continue;
            };

            let reasoning_chunk = choice.delta.reasoning_content.unwrap_or_default();
            let content_chunk = choice.delta.content.map(content_to_text).unwrap_or_default();

            let mut emitted = String::new();
            if !reasoning_chunk.is_empty() {
                if !in_reasoning_block {
                    emitted.push_str("<think>\n");
                    in_reasoning_block = true;
                }
                emitted.push_str(&reasoning_chunk);
            }

            if !content_chunk.is_empty() {
                if in_reasoning_block {
                    emitted.push_str("\n</think>\n");
                    in_reasoning_block = false;
                }
                emitted.push_str(&content_chunk);
            }

            if emitted.is_empty() {
                continue;
            }

            on_chunk(&emitted)?;
            text.push_str(&emitted);
        }

        if in_reasoning_block {
            on_chunk("\n</think>")?;
            text.push_str("\n</think>");
        }
        if text.trim().is_empty() {
            return Err("LLM response text was empty".into());
        }

        Ok(text)
    }
}

fn content_to_text(content: Content) -> String {
    match content {
        Content::Text(text) => text,
        Content::Parts(parts) => extract_text_parts(&parts),
    }
}

fn assistant_message_to_text(message: AssistantMessage) -> String {
    let mut output = String::new();

    if let Some(reasoning) = message.reasoning_content.as_deref() {
        output.push_str(&wrap_thinking_text(reasoning));
    }

    if let Some(content) = message.content {
        if !output.is_empty() && !output.ends_with('\n') {
            output.push('\n');
        }
        output.push_str(&content_to_text(content));
    }

    output
}
