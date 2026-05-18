use anyhow::Result;
use serde::{Deserialize, Serialize};

/// LLM provider backend.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ProviderKind {
    OpenAI,
    OpenRouter,
    Anthropic,
    Bedrock,
    Gemini,
    LMStudio,
    Custom { base_url: String },
}

/// Configuration for an LLM provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub kind: ProviderKind,
    pub api_key: Option<String>,
    pub model: String,
    /// Default model for this provider.
    pub default_model: String,
    pub base_url: Option<String>,
    pub max_tokens: Option<usize>,
    pub temperature: Option<f64>,
    /// Fallback models, tried in order if the primary fails.
    pub fallback_models: Vec<String>,
}

impl ProviderConfig {
    pub fn new(kind: ProviderKind, model: impl Into<String>) -> Self {
        let model: String = model.into();
        Self {
            kind,
            api_key: None,
            model: model.clone(),
            default_model: model,
            base_url: None,
            max_tokens: None,
            temperature: None,
            fallback_models: vec![],
        }
    }
}

/// LLM response from a transport call.
#[derive(Debug, Clone)]
pub struct TransportResponse {
    pub text: String,
    pub tool_calls: Vec<TransportToolCall>,
    pub tokens_used: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct TransportToolCall {
    pub id: String,
    pub tool_name: String,
    pub arguments: serde_json::Value,
}

/// Trait for LLM transport implementations.
#[async_trait::async_trait]
pub trait TransportProvider: Send + Sync {
    /// Get provider name for logging.
    fn name(&self) -> &str;

    /// Send a chat completion request.
    async fn chat(&self, messages: &[super::Message]) -> Result<TransportResponse>;

    /// Optional: estimate tokens without full API call.
    fn estimate_tokens(&self, messages: &[super::Message]) -> usize {
        // Default: rough heuristic
        messages.iter().map(|m| {
            match &m.content {
                super::MessageContent::Text(s) => s.len() / 4 + 5,
                super::MessageContent::Image { .. } => 500,
                super::MessageContent::Parts(parts) => {
                    parts.iter().map(|p| match p {
                        super::MessagePart::Text(s) => s.len() / 4 + 5,
                        super::MessagePart::Image { .. } => 500,
                    }).sum::<usize>()
                }
            }
        }).sum()
    }
}
