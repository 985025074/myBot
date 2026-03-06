use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};

use crate::{
    app::AppResult,
    config::LlmConfig,
    llm::{ChatMessage, ErrorEnvelope, TextPart, extract_text_parts, missing_api_key_error},
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

    pub fn send_chat(&self, history: &[ChatMessage]) -> AppResult<String> {
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
            stream: false,
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

        let response: ChatCompletionResponse = response.json()?;
        let choice = response
            .choices
            .into_iter()
            .next()
            .ok_or("LLM response did not contain any choices")?;

        let content = choice
            .message
            .content
            .ok_or("LLM response did not contain message content")?;

        let text = match content {
            Content::Text(text) => text,
            Content::Parts(parts) => extract_text_parts(&parts),
        };

        if text.trim().is_empty() {
            return Err("LLM response text was empty".into());
        }

        Ok(text)
    }
}
