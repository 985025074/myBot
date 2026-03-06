use std::collections::HashMap;
use std::io::{BufRead, BufReader};

use reqwest::header::CONTENT_TYPE;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};

use crate::{
    app::AppResult,
    config::LlmConfig,
    llm::{
        ChatMessage, ErrorEnvelope, TextPart, extract_text_parts, missing_api_key_error,
    },
};

#[derive(Debug, Clone)]
pub struct AnthropicClient {
    http: Client,
    config: LlmConfig,
}

#[derive(Debug, Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    messages: Vec<AnthropicMessage>,
    stream: bool,
}

#[derive(Debug, Serialize)]
struct AnthropicMessage {
    role: String,
    content: Vec<AnthropicContentBlock>,
}

#[derive(Debug, Serialize)]
struct AnthropicContentBlock {
    #[serde(rename = "type")]
    kind: &'static str,
    text: String,
}

#[derive(Debug, Deserialize)]
struct AnthropicResponse {
    content: Vec<TextPart>,
}

#[derive(Debug, Deserialize)]
struct AnthropicStreamEvent {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    index: Option<usize>,
    #[serde(default)]
    delta: Option<AnthropicDelta>,
    #[serde(default)]
    content_block: Option<AnthropicStreamContentBlock>,
}

#[derive(Debug, Deserialize)]
struct AnthropicDelta {
    #[serde(rename = "type")]
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    thinking: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AnthropicStreamContentBlock {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    thinking: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AnthropicErrorEnvelope {
    #[serde(default)]
    error: Option<AnthropicApiError>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    #[serde(rename = "type")]
    kind: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AnthropicApiError {
    message: String,
}

impl AnthropicClient {
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

        let request = AnthropicRequest {
            model: self.config.model.clone(),
            max_tokens: self.config.max_tokens,
            temperature: self.config.temperature,
            system: (!self.config.system_prompt.trim().is_empty())
                .then(|| self.config.system_prompt.clone()),
            messages: history
                .iter()
                .filter(|message| matches!(message.role.as_str(), "user" | "assistant"))
                .map(|message| AnthropicMessage {
                    role: message.role.clone(),
                    content: vec![AnthropicContentBlock {
                        kind: "text",
                        text: message.content.clone(),
                    }],
                })
                .collect(),
            stream: true,
        };

        let url = format!("{}/messages", self.config.base_url.trim_end_matches('/'));
        let response = self
            .http
            .post(url)
            .header("Content-Type", "application/json")
            .header("x-api-key", api_key)
            .header("anthropic-version", self.config.anthropic_version.as_str())
            .json(&request)
            .send()?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text()?;
            if let Ok(error) = serde_json::from_str::<AnthropicErrorEnvelope>(&body) {
                if let Some(error) = error.error {
                    return Err(error.message.into());
                }
                if let Some(message) = error.message {
                    return Err(message.into());
                }
                if let Some(kind) = error.kind {
                    return Err(format!("Anthropic request failed: {kind}").into());
                }
            }
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
            let response: AnthropicResponse = response.json()?;
            let text = extract_text_parts(&response.content);
            if text.trim().is_empty() {
                return Err("LLM response text was empty".into());
            }
            on_chunk(&text)?;
            return Ok(text);
        }

        let mut text = String::new();
        let mut reader = BufReader::new(response);
        let mut line = String::new();
        let mut content_block_kinds = HashMap::new();

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

            let Some(event) = serde_json::from_str::<AnthropicStreamEvent>(data).ok() else {
                continue;
            };

            let mut emitted = String::new();

            match event.kind.as_str() {
                "content_block_start" => {
                    if let Some(block) = event.content_block {
                        if let Some(index) = event.index {
                            content_block_kinds.insert(index, block.kind.clone());
                        }

                        match block.kind.as_str() {
                            "thinking" => {
                                emitted.push_str("<think>\n");
                                if let Some(thinking) = block.thinking.or(block.text)
                                    && !thinking.is_empty()
                                {
                                    emitted.push_str(&thinking);
                                }
                            }
                            "text" => {
                                if let Some(value) = block.text && !value.is_empty() {
                                    emitted.push_str(&value);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                "content_block_delta" => {
                    let block_kind = event
                        .index
                        .and_then(|index| content_block_kinds.get(&index).cloned())
                        .unwrap_or_else(|| "text".to_string());
                    if let Some(delta) = event.delta {
                        match (block_kind.as_str(), delta.kind.as_deref()) {
                            ("thinking", Some("thinking_delta")) | ("thinking", None) => {
                                if let Some(thinking) = delta.thinking.or(delta.text)
                                    && !thinking.is_empty()
                                {
                                    emitted.push_str(&thinking);
                                }
                            }
                            _ => {
                                if let Some(value) = delta.text && !value.is_empty() {
                                    emitted.push_str(&value);
                                }
                            }
                        }
                    }
                }
                "content_block_stop" => {
                    if let Some(index) = event.index
                        && content_block_kinds.remove(&index).as_deref() == Some("thinking")
                    {
                        emitted.push_str("\n</think>");
                    }
                }
                _ => {}
            }

            if emitted.is_empty() {
                continue;
            }

            on_chunk(&emitted)?;
            text.push_str(&emitted);
        }

        for kind in content_block_kinds.into_values() {
            if kind == "thinking" {
                on_chunk("\n</think>")?;
                text.push_str("\n</think>");
            }
        }

        if text.trim().is_empty() {
            return Err("LLM response text was empty".into());
        }

        Ok(text)
    }
}
