use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};

use crate::{
    app::AppResult,
    config::LlmConfig,
    llm::{ChatMessage, ErrorEnvelope, TextPart, extract_text_parts, missing_api_key_error},
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

    pub fn send_chat(&self, history: &[ChatMessage]) -> AppResult<String> {
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
            stream: false,
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

        let response: AnthropicResponse = response.json()?;
        let text = extract_text_parts(&response.content);
        if text.trim().is_empty() {
            return Err("LLM response text was empty".into());
        }

        Ok(text)
    }
}
