use anyhow::Result;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

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
///
/// Supports `ProviderAlias` deserialization so config values like
/// `claude`, `gpt`, `deepseek`, `qwen` etc. are normalized to canonical providers.
#[derive(Debug, Clone, PartialEq)]
pub enum ProviderKind {
    OpenAI,
    OpenRouter,
    Anthropic,
    Bedrock,
    Gemini,
    LMStudio,
    DeepSeek,
    Alibaba,
    AlibabaCodingPlan,
    StepFun,
    MiniMax,
    MiniMaxOAuth,
    MiniMaxCN,
    Zai,
    Kimi,
    Baidu,
    TencentTokenHub,
    XAI,
    XAIOAuth,
    NVIDIA,
    Nous,
    Vercel,
    OpenCode,
    OpenCodeGo,
    Kilo,
    HuggingFace,
    Novita,
    Xiaomi,
    Arcee,
    GMI,
    OllamaCloud,
    Local,
    Custom,
}

impl Serialize for ProviderKind {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ProviderKind {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let name = String::deserialize(deserializer)?;
        Ok(Self::from_alias(&name))
    }
}

impl ProviderKind {
    /// Canonical name string.
    pub fn as_str(&self) -> &'static str {
        match self {
            ProviderKind::OpenAI => "openai",
            ProviderKind::OpenRouter => "openrouter",
            ProviderKind::Anthropic => "anthropic",
            ProviderKind::Bedrock => "bedrock",
            ProviderKind::Gemini => "gemini",
            ProviderKind::LMStudio => "lmstudio",
            ProviderKind::DeepSeek => "deepseek",
            ProviderKind::Alibaba => "alibaba",
            ProviderKind::AlibabaCodingPlan => "alibaba-coding-plan",
            ProviderKind::StepFun => "stepfun",
            ProviderKind::MiniMax => "minimax",
            ProviderKind::MiniMaxOAuth => "minimax-oauth",
            ProviderKind::MiniMaxCN => "minimax-cn",
            ProviderKind::TencentTokenHub => "tencent-tokenhub",
            ProviderKind::Zai => "zai",
            ProviderKind::Kimi => "kimi-for-coding",
            ProviderKind::Baidu => "baidu",
            ProviderKind::XAI => "xai",
            ProviderKind::XAIOAuth => "xai-oauth",
            ProviderKind::NVIDIA => "nvidia",
            ProviderKind::Nous => "nous",
            ProviderKind::Vercel => "vercel",
            ProviderKind::OpenCode => "opencode",
            ProviderKind::OpenCodeGo => "opencode-go",
            ProviderKind::Kilo => "kilo",
            ProviderKind::HuggingFace => "huggingface",
            ProviderKind::Novita => "novita",
            ProviderKind::Xiaomi => "xiaomi",
            ProviderKind::Arcee => "arcee",
            ProviderKind::GMI => "gmi",
            ProviderKind::OllamaCloud => "ollama-custom",
            ProviderKind::Local => "local",
            ProviderKind::Custom => "custom",
        }
    }

    /// Resolve an alias (or canonical name) to its canonical ProviderKind.
    ///
    /// Supports 50+ aliases including: `claude`, `gpt`, `deepseek`, `qwen`,
    /// `glm`, `grok`, `minimax`, `tokenhub`, `mimo`, `arcee`, `gmi`, `mimo`, etc.
    pub fn from_alias(name: &str) -> Self {
        let normalized = name.trim().to_lowercase();
        if normalized.is_empty() {
            return ProviderKind::Custom;
        }

        // Check alias table (from provider_registry.rs)
        for (alias, canonical) in crate::provider_registry::ALIAS_CANONICAL_PAIRS {
            if *alias == normalized {
                return canonical_to_kind(canonical);
            }
        }

        ProviderKind::Custom
    }

    /// Check if this provider supports streaming chat.
    pub fn supports_streaming(&self) -> bool {
        matches!(
            self,
            ProviderKind::OpenAI
                | ProviderKind::OpenRouter
                | ProviderKind::Anthropic
                | ProviderKind::Bedrock
                | ProviderKind::Gemini
                | ProviderKind::LMStudio
                | ProviderKind::DeepSeek
                | ProviderKind::Alibaba
                | ProviderKind::AlibabaCodingPlan
                | ProviderKind::StepFun
                | ProviderKind::MiniMax
                | ProviderKind::MiniMaxOAuth
                | ProviderKind::MiniMaxCN
                | ProviderKind::TencentTokenHub
                | ProviderKind::XAI
                | ProviderKind::XAIOAuth
                | ProviderKind::NVIDIA
                | ProviderKind::Nous
                | ProviderKind::Vercel
                | ProviderKind::OpenCode
                | ProviderKind::OpenCodeGo
                | ProviderKind::Kilo
                | ProviderKind::HuggingFace
                | ProviderKind::Novita
                | ProviderKind::Xiaomi
                | ProviderKind::Arcee
                | ProviderKind::GMI
                | ProviderKind::OllamaCloud
                | ProviderKind::Local
                | ProviderKind::Zai
                | ProviderKind::Kimi
                | ProviderKind::Baidu
        )
    }

    /// Check if this provider uses Anthropic-compatible protocol.
    pub fn is_anthropic_protocol(&self) -> bool {
        matches!(
            self,
            ProviderKind::Anthropic
                | ProviderKind::MiniMax
                | ProviderKind::MiniMaxOAuth
                | ProviderKind::MiniMaxCN
        )
    }

    /// Check if this provider is a domestic Chinese provider.
    pub fn is_domestic_china(&self) -> bool {
        matches!(
            self,
            ProviderKind::Alibaba
                | ProviderKind::AlibabaCodingPlan
                | ProviderKind::StepFun
                | ProviderKind::MiniMax
                | ProviderKind::MiniMaxOAuth
                | ProviderKind::MiniMaxCN
                | ProviderKind::TencentTokenHub
                | ProviderKind::Xiaomi
                | ProviderKind::Zai
                | ProviderKind::Kimi
                | ProviderKind::Baidu
        )
    }

    /// Returns the default base URL for this provider from the registry.
    ///
    /// Returns `None` for providers with no hardcoded default (e.g. OpenAI).
    /// The caller should additionally check env vars, then fall back
    /// to `config.base_url`.
    pub fn default_base_url(&self) -> Option<&'static str> {
        use crate::provider_registry::PROVIDER_META;
        PROVIDER_META
            .iter()
            .find(|(id, _, _)| *id == self.as_str())
            .map(|(_, _, base_url)| *base_url)
            .filter(|url| !url.is_empty())
    }

    /// Returns None if this is a local/overridable provider that does not auto-detect.
    pub fn base_url_env_var(&self) -> Option<&'static str> {
        use crate::provider_registry::resolve_base_url_env_var;
        resolve_base_url_env_var(self.as_str())
    }
}

fn canonical_to_kind(canonical: &str) -> ProviderKind {
    match canonical {
        "openai" => ProviderKind::OpenAI,
        "openrouter" => ProviderKind::OpenRouter,
        "anthropic" => ProviderKind::Anthropic,
        "bedrock" => ProviderKind::Bedrock,
        "gemini" | "google-gemini-cli" => ProviderKind::Gemini,
        "lmstudio" => ProviderKind::LMStudio,
        "deepseek" => ProviderKind::DeepSeek,
        "alibaba" => ProviderKind::Alibaba,
        "alibaba-coding-plan" => ProviderKind::AlibabaCodingPlan,
        "stepfun" => ProviderKind::StepFun,
        "minimax" => ProviderKind::MiniMax,
        "minimax-oauth" => ProviderKind::MiniMaxOAuth,
        "minimax-cn" => ProviderKind::MiniMaxCN,
        "tencent-tokenhub" => ProviderKind::TencentTokenHub,
        "zai" => ProviderKind::Zai,
        "kimi" | "kimi-for-coding" => ProviderKind::Kimi,
        "baidu" | "baidu-wenxin" => ProviderKind::Baidu,
        "xai" => ProviderKind::XAI,
        "xai-oauth" => ProviderKind::XAIOAuth,
        "nvidia" => ProviderKind::NVIDIA,
        "nous" => ProviderKind::Nous,
        "vercel" | "ai-gateway" | "aigateway" | "vercel-ai-gateway" => ProviderKind::Vercel,
        "opencode" | "opencode-zen" | "zen" => ProviderKind::OpenCode,
        "opencode-go" | "go" | "opencode-go-sub" => ProviderKind::OpenCodeGo,
        "kilo" | "kilocode" | "kilo-code" | "kilo-gateway" => ProviderKind::Kilo,
        "huggingface" | "hf" | "hugging-face" | "huggingface-hub" => ProviderKind::HuggingFace,
        "novita" | "novita-ai" | "novitaai" => ProviderKind::Novita,
        "xiaomi" | "mimo" | "xiaomi-mimo" => ProviderKind::Xiaomi,
        "arcee" | "arcee-ai" | "arceeai" => ProviderKind::Arcee,
        "gmi" | "gmi-cloud" | "gmicloud" => ProviderKind::GMI,
        "ollama-custom" | "ollama" | "ollama-cloud" => ProviderKind::OllamaCloud,
        "local" | "vllm" | "llamacpp" | "llama.cpp" | "llama-cpp" => ProviderKind::Local,
        _ => ProviderKind::Custom,
    }
}

/// A model returned from `/v1/models` API.
/// Supports both OpenAI-compatible format and Qwen/Ollama format.
#[derive(Debug, Clone, Serialize)]
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
    pub max_model_len: Option<usize>,
    /// Root path for the model (vLLM-specific).
    pub root: Option<String>,
    /// Parent model (for model groups).
    pub parent: Option<String>,
}

impl<'de> Deserialize<'de> for ModelInfo {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // First, deserialize into a JSON Value to inspect the structure
        let value = Value::deserialize(deserializer)?;

        // Try to extract fields from OpenAI-compatible format first
        let id = value.get("id")
            .and_then(|v| v.as_str())
            .or_else(|| value.get("model").and_then(|v| v.as_str()))
            .or_else(|| value.get("name").and_then(|v| v.as_str()))
            .map(|s| s.to_string())
            .ok_or_else(|| serde::de::Error::missing_field("id"))?;

        let object = value.get("object")
            .and_then(|v| v.as_str())
            .unwrap_or("model")
            .to_string();

        let created = value.get("created")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        let owned_by = value.get("owned_by")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        // Try max_model_len first (OpenAI), then meta.n_ctx (Qwen/Ollama)
        let max_model_len = value.get("max_model_len")
            .and_then(|v| v.as_u64())
            .or_else(|| {
                value.get("meta")
                    .and_then(|m| m.get("n_ctx"))
                    .and_then(|v| v.as_u64())
            })
            .map(|v| v as usize);

        let root = value.get("root")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let parent = value.get("parent")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        Ok(ModelInfo {
            id,
            object,
            created,
            owned_by,
            max_model_len,
            root,
            parent,
        })
    }
}

/// Response from `/v1/models` API.
/// Supports both OpenAI-compatible format (data array) and Qwen format (models array).
#[derive(Debug, Clone, Serialize)]
pub struct ModelListResponse {
    pub object: String,
    pub data: Vec<ModelInfo>,
}

impl<'de> Deserialize<'de> for ModelListResponse {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;

        let object = value.get("object")
            .and_then(|v| v.as_str())
            .unwrap_or("list")
            .to_string();

        let data_array = value.get("data").and_then(|v| v.as_array());
        let models_array = value.get("models").and_then(|v| v.as_array());

        let data = match (data_array, models_array) {
            (Some(data_arr), Some(models_arr)) => {
                let mut models_by_id: std::collections::HashMap<String, ModelInfo> = std::collections::HashMap::new();

                for item in data_arr.iter().chain(models_arr.iter()) {
                    let id = item.get("id")
                        .and_then(|v| v.as_str())
                        .or_else(|| item.get("model").and_then(|v| v.as_str()))
                        .or_else(|| item.get("name").and_then(|v| v.as_str()))
                        .map(|s| s.to_string())
                        .unwrap_or_default();

                    if let Ok(info) = ModelInfo::deserialize(item.clone()) {
                        if let Some(existing) = models_by_id.get_mut(&id) {
                            if existing.max_model_len.is_none() && info.max_model_len.is_some() {
                                *existing = info;
                            }
                        } else {
                            models_by_id.insert(id, info);
                        }
                    }
                }

                models_by_id.into_values().collect()
            }
            (Some(arr), None) | (None, Some(arr)) => {
                arr.iter()
                    .map(|item| ModelInfo::deserialize(item.clone()))
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap_or_default()
            }
            (None, None) => Vec::new(),
        };

        Ok(ModelListResponse { object, data })
    }
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
    Tool {
        name: String,
    },
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
    #[serde(
        default = "default_cache_strategy",
        skip_serializing_if = "is_default_cache"
    )]
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
    /// Logit bias adjustment for provider-specific token weighting.
    #[serde(default)]
    pub logit_bias: Option<serde_json::Value>,
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
            logit_bias: None,
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
    /// Reasoning/thinking chain-of-thought from models that support it.
    pub reasoning: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TransportToolCall {
    pub id: String,
    pub tool_name: String,
    pub arguments: serde_json::Value,
}

/// Callback type invoked with each text delta during streaming.
pub type StreamDeltaCallback = Box<dyn FnMut(&str) + Send>;

/// Callback type invoked with each reasoning delta during streaming.
///
/// Emits reasoning content (e.g., OpenAI-style `reasoning_content`) that is
/// already separated from `content` at the API level. Distinct from inline
/// `<thinking>` tags (handled by scrub_thinking_blocks() in the text path).
pub type StreamReasoningCallback = Box<dyn FnMut(&str) + Send>;

/// Blanket impl: any `Arc<T: TransportProvider>` is also a `TransportProvider`.
#[async_trait::async_trait]
impl<T: TransportProvider + ?Sized + Send + Sync> TransportProvider for std::sync::Arc<T> {
    fn name(&self) -> &str {
        (**self).name()
    }

    async fn chat(
        &self,
        messages: &[super::Message],
        mode: &super::CallMode,
    ) -> Result<TransportResponse> {
        (**self).chat(messages, mode).await
    }

    async fn stream_chat(
        &self,
        messages: &[super::Message],
        mode: &super::CallMode,
        delta_callback: StreamDeltaCallback,
        reasoning_callback: Option<StreamReasoningCallback>,
    ) -> Result<TransportResponse> {
        (**self).stream_chat(messages, mode, delta_callback, reasoning_callback).await
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
    async fn chat(
        &self,
        messages: &[super::Message],
        mode: &super::CallMode,
    ) -> Result<TransportResponse>;

    /// Fires `delta_callback` with each text delta as it arrives.
    /// Fires `reasoning_callback` (if Some) with each reasoning content delta.
    /// Returns the accumulated response with full text and tool calls.
    async fn stream_chat(
        &self,
        messages: &[super::Message],
        mode: &super::CallMode,
        delta_callback: StreamDeltaCallback,
        reasoning_callback: Option<StreamReasoningCallback>,
    ) -> Result<TransportResponse>;

    /// Optional: estimate tokens without full API call.
    fn estimate_tokens(&self, messages: &[super::Message]) -> usize {
        // Default: rough heuristic
        messages
            .iter()
            .map(|m| match &m.content {
                super::MessageContent::Text(s) => s.len() / 4 + 5,
                super::MessageContent::Image { .. } => 500,
                super::MessageContent::Parts(parts) => parts
                    .iter()
                    .map(|p| match p {
                        super::MessagePart::Text(s) => s.len() / 4 + 5,
                        super::MessagePart::Image { .. } => 500,
                    })
                    .sum::<usize>(),
            })
            .sum()
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
