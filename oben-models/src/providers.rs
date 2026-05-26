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

/// Response format option for OpenAI-compatible APIs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponseFormat {
    Text,
    Json,
    JsonSchema { schema: serde_json::Value },
}

/// Tool choice mode for provider APIs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolChoice {
    None,
    Auto,
    Any,
    /// Force a specific tool by name.
    Tool { name: String },
}

/// Reasoning effort level for providers that support extended thinking.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningEffort {
    Low,
    Medium,
    High,
    XHigh,
}

/// Prompt caching configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheControl {
    /// Provider name for cache key (e.g. "openrouter").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Model name for cache key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Caching strategy: "ephemeral" works for most providers.
    #[serde(default = "default_cache_strategy", skip_serializing_if = "is_default_cache")]
    pub strategy: String,
}

fn default_cache_strategy() -> String {
    "ephemeral".to_string()
}

fn is_default_cache(s: &String) -> bool {
    *s == "ephemeral"
}

/// Additional provider-specific fields to merge into the request body.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExtraBody {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anthropic_max_output: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<ReasoningEffort>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_config: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ollama_num_ctx: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
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
    /// Tool definitions as JSON (passed through to transport).
    #[serde(default)]
    pub tools_json: Option<serde_json::Value>,
    // --- API Payload Attributes ---
    /// Top-p sampling cutoff.
    #[serde(default)]
    pub top_p: Option<f64>,
    /// Top-k sampling cutoff.
    #[serde(default)]
    pub top_k: Option<usize>,
    /// Frequency penalty for OpenAI-compatible APIs.
    #[serde(default)]
    pub frequency_penalty: Option<f64>,
    /// Presence penalty for OpenAI-compatible APIs.
    #[serde(default)]
    pub presence_penalty: Option<f64>,
    /// Stop sequences.
    #[serde(default)]
    pub stop_sequences: Option<Vec<String>>,
    /// Response format (JSON mode, structured schema).
    #[serde(default)]
    pub response_format: Option<ResponseFormat>,
    /// Tool choice strategy for providers that support it.
    #[serde(default)]
    pub tool_choice: Option<ToolChoice>,
    /// Per-call reasoning effort level (DeepSeek, LM Studio, Kimi, etc.). Injected into extra_body for providers that support it natively.
    #[serde(default)]
    pub reasoning_effort: Option<ReasoningEffort>,
    /// HTTP request timeout in seconds (for external API clients, not used internally).
    #[serde(default)]
    pub timeout: Option<u64>,
    /// Service tier for providers with tiered pricing (OpenAI: "default", "flex", "auto").
    #[serde(default)]
    pub service_tier: Option<String>,
    /// Provider preferences for routing (AWS Bedrock multi-region).
    #[serde(default)]
    pub provider_preferences: Option<String>,
    /// User ID for provider tracking (OpenRouter).
    #[serde(default)]
    pub user_id: Option<String>,
    /// Per-call metadata for tagging/tracking.
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
    /// Anthropic cache_control marker for prompt caching.
    #[serde(default)]
    pub cache_control: Option<CacheControl>,
    /// Extra arbitrary fields to merge into the request body.
    #[serde(default)]
    pub extra_body: ExtraBody,
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
            tools_json: None,
            top_p: None,
            top_k: None,
            frequency_penalty: None,
            presence_penalty: None,
            stop_sequences: None,
            response_format: None,
            tool_choice: None,
            reasoning_effort: None,
            timeout: None,
            service_tier: None,
            provider_preferences: None,
            user_id: None,
            metadata: None,
            cache_control: None,
            extra_body: ExtraBody::default(),
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

    async fn list_models(&self) -> Result<ModelListResponse> {
        (**self).list_models().await
    }

    async fn find_model(&self, model_id: &str) -> Result<Option<ModelInfo>> {
        (**self).find_model(model_id).await
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

    /// Fetch the list of available models from the provider.
    async fn list_models(&self) -> Result<ModelListResponse> {
        Err(anyhow::anyhow!("list_models not implemented"))
    }

    /// Find a specific model by ID from the provider.
    async fn find_model(&self, _model_id: &str) -> Result<Option<ModelInfo>> {
        Err(anyhow::anyhow!("find_model not implemented"))
    }
}
