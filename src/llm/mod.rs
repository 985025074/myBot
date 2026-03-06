mod anthropic;
mod openai_compatible;

use std::time::Duration;

use reqwest::blocking::Client;
use serde::Serialize;

use crate::{
    app::AppResult,
    config::{LlmConfig, ProviderKind},
};

pub use anthropic::AnthropicClient;
pub use openai_compatible::OpenAiCompatibleClient;

#[derive(Debug, Clone, Serialize)]
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

    pub fn send_chat(&self, history: &[ChatMessage]) -> AppResult<String> {
        match &self.provider {
            ProviderClient::OpenAiCompatible(client) => client.send_chat(history),
            ProviderClient::Anthropic(client) => client.send_chat(history),
        }
    }
}

fn missing_api_key_error(env_name: &str) -> String {
    format!(
        "Missing API key. Set the {env_name} environment variable or add api_key in config/llm.toml."
    )
}

fn extract_text_parts(parts: &[TextPart]) -> String {
    parts
        .iter()
        .filter(|part| part.kind.as_deref().unwrap_or("text") == "text")
        .filter_map(|part| part.text.clone())
        .collect::<Vec<_>>()
        .join("")
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
}
