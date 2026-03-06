mod anthropic;
mod openai_compatible;

use std::time::Duration;

use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};

use crate::{
    app::AppResult,
    config::{LlmConfig, ProviderKind},
};

pub use anthropic::AnthropicClient;
pub use openai_compatible::OpenAiCompatibleClient;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct LlmClient {
    provider: ProviderClient,
    config: LlmConfig,
}

#[derive(Debug, Clone)]
enum ProviderClient {
    OpenAiCompatible(OpenAiCompatibleClient),
    Anthropic(AnthropicClient),
}

impl LlmClient {
    pub fn new(config: LlmConfig) -> AppResult<Self> {
        let http = Client::builder()
            .timeout(Duration::from_secs(config.timeout_seconds))
            .build()?;

        let provider = match config.provider {
            ProviderKind::OpenAiCompatible => {
                ProviderClient::OpenAiCompatible(OpenAiCompatibleClient::new(http, config.clone()))
            }
            ProviderKind::Anthropic | ProviderKind::AliyunCodingPlan => {
                ProviderClient::Anthropic(AnthropicClient::new(http, config.clone()))
            }
        };

        Ok(Self { provider, config })
    }

    pub fn model_name(&self) -> &str {
        &self.config.model
    }

    pub fn provider_name(&self) -> &'static str {
        match self.config.provider {
            ProviderKind::OpenAiCompatible => "openai-compatible",
            ProviderKind::Anthropic => "anthropic",
            ProviderKind::AliyunCodingPlan => "aliyun-coding-plan",
        }
    }

    #[allow(dead_code)]
    pub fn send_chat(&self, history: &[ChatMessage]) -> AppResult<String> {
        self.send_chat_streaming(history, |_| Ok(()))
    }

    pub fn send_chat_streaming(
        &self,
        history: &[ChatMessage],
        on_chunk: impl FnMut(&str) -> AppResult<()>,
    ) -> AppResult<String> {
        match &self.provider {
            ProviderClient::OpenAiCompatible(client) => client.send_chat_streaming(history, on_chunk),
            ProviderClient::Anthropic(client) => client.send_chat_streaming(history, on_chunk),
        }
    }
}

fn missing_api_key_error(env_name: &str) -> String {
    format!(
        "Missing API key. Set the {env_name} environment variable or place it in the active scope .mybot/.env file."
    )
}

fn extract_text_parts(parts: &[TextPart]) -> String {
    let mut output = String::new();

    for part in parts {
        let kind = part.kind.as_deref().unwrap_or("text");
        match kind {
            "thinking" => {
                if let Some(thinking) = part
                    .thinking
                    .as_deref()
                    .or(part.reasoning_content.as_deref())
                    .or(part.text.as_deref())
                {
                    output.push_str(&wrap_thinking_text(thinking));
                }
            }
            "reasoning" | "reasoning_content" => {
                if let Some(reasoning) = part
                    .reasoning_content
                    .as_deref()
                    .or(part.thinking.as_deref())
                    .or(part.text.as_deref())
                {
                    output.push_str(&wrap_thinking_text(reasoning));
                }
            }
            _ => {
                if let Some(text) = part.text.as_deref() {
                    output.push_str(text);
                } else if let Some(reasoning) = part.reasoning_content.as_deref() {
                    output.push_str(&wrap_thinking_text(reasoning));
                }
            }
        }
    }

    output
}

fn wrap_thinking_text(text: &str) -> String {
    if text.trim().is_empty() {
        return String::new();
    }

    format!("<think>\n{}\n</think>", text.trim_end())
}

#[derive(Debug, serde::Deserialize)]
struct ErrorEnvelope {
    error: ProviderError,
}

#[derive(Debug, serde::Deserialize)]
struct ProviderError {
    message: String,
}

#[derive(Debug, serde::Deserialize, Clone)]
struct TextPart {
    #[serde(rename = "type")]
    kind: Option<String>,
    text: Option<String>,
    #[serde(default)]
    thinking: Option<String>,
    #[serde(default)]
    reasoning_content: Option<String>,
}
