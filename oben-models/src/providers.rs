use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Controls how the transport builds the message JSON for a call.
///
/// The transport maintains a per-session cache. Each CallMode carries a
/// session_id so different sessions don't share caches.
#[derive(Debug, Clone)]
pub enum CallMode {
    /// Rebuild all message JSON from scratch (new turn or session).
    Fresh(String),
    /// Append messages starting from cache.len() (continuation of current turn).
    /// If messages.len() <= cached.len(), the cache resets.
    Incremental(String),
}

/// LLM provider backend.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ProviderKind {
    OpenAI,
    OpenRouter,
    Anthropic,
    Bedrock,
    Gemini,
    LMStudio,
    Custom,
}

/// A model returned from `/v1/models` API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    /// The model's unique identifier.
    pub id: String,
    /// Object type, typically "model".
    pub object: String,
    /// Unix timestamp when the model was created.
    pub created: u64,
    /// Organization that owns the model.
    pub owned_by: String,
    /// Max model context length (for OpenAI-compatible API).
    #[serde(default)]
    pub max_model_len: Option<usize>,
    /// Root path for the model (vLLM-specific).
    #[serde(default)]
    pub root: Option<String>,
    /// Parent model (for model groups).
    #[serde(default)]
    pub parent: Option<String>,
}

/// Response from `/v1/models` API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelListResponse {
    pub object: String,
    pub data: Vec<ModelInfo>,
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
    #[serde(default)]
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

/// Callback type invoked with each text delta during streaming.
pub type StreamDeltaCallback = Box<dyn FnMut(&str) + Send>;

/// Blanket impl: any `Arc<T: TransportProvider>` is also a `TransportProvider`.
#[async_trait::async_trait]
impl<T: TransportProvider + ?Sized + Send + Sync> TransportProvider for std::sync::Arc<T> {
    fn name(&self) -> &str {
        (**self).name()
    }

    async fn chat(&self, messages: &[super::Message], mode: &super::CallMode) -> Result<TransportResponse> {
        (**self).chat(messages, mode).await
    }

    async fn stream_chat(&self, messages: &[super::Message], mode: &super::CallMode, delta_callback: StreamDeltaCallback) -> Result<TransportResponse> {
        (**self).stream_chat(messages, mode, delta_callback).await
    }

    fn estimate_tokens(&self, messages: &[super::Message]) -> usize {
        (**self).estimate_tokens(messages)
    }
}

/// Trait for LLM transport implementations.
#[async_trait::async_trait]
pub trait TransportProvider: Send + Sync {
    /// Get provider name for logging.
    fn name(&self) -> &str;

    /// Send a chat completion request.
    ///
    /// See [`CallMode`] for semantics. `mode` is borrowed to avoid
    /// cloning the session ID in hot paths (e.g. multi-tool loops).
    async fn chat(&self, messages: &[super::Message], mode: &super::CallMode) -> Result<TransportResponse>;

    /// Send a streaming chat completion request.
    ///
    /// Fires `delta_callback` with each text delta as it arrives.
    /// Returns the accumulated response with full text and tool calls.
    async fn stream_chat(&self, messages: &[super::Message], mode: &super::CallMode, delta_callback: StreamDeltaCallback) -> Result<TransportResponse>;

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
