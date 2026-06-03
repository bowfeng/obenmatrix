/// Unified transport dispatcher — selects the right LLM transport implementation
/// based on the ProviderKind in the config.
///
/// Maps to `agent/transports/__init__.py` from Hermes-Agent.
///
/// Architecture:
///
/// ```ignore
/// Transport (enum)
/// ├── from_config_with_tools_via_registry(...) — registry-based creation
/// ├── list_models() — fetch available models
/// └── find_model(id) — find a specific model
/// ```
use std::sync::Arc;

use anyhow::Result;
use oben_models::{
    provider_kind_to_transport,
    providers::{ProviderConfig, TransportProvider, TransportResponse},
    CallMode, Message, ProviderKind, Tool,
};

use super::{
    anthropic_messages::AnthropicMessagesTransport, chat_completions::ChatCompletionsTransport,
    registry,
};

/// Internal transport enum — wraps ChatCompletionsTransport + AnthropicMessagesTransport.
pub enum Transport {
    /// OpenAI-compatible API (Chat Completions).
    OpenAIChat { transport: ChatCompletionsTransport },
    /// Anthropic native Messages API.
    Anthropic {
        transport: AnthropicMessagesTransport,
    },
}

/// Determine whether a provider kind routes to the Anthropic-messages protocol.
///
/// Delegates to `provider_kind_to_transport` from `oben_models::provider_registry`.
fn uses_anthropic_protocol(kind: &ProviderKind) -> bool {
    matches!(
        provider_kind_to_transport(kind.clone()),
        Some(oben_models::TransportType::AnthropicMessages)
    )
}

impl Transport {
    /// Create a transport instance from a ProviderConfig (legacy, non-registry).
    ///
    /// Routes to the correct transport type based on `config.kind`:
    /// - `Anthropic, MiniMax, MiniMaxOAuth, MiniMaxCN` -> `AnthropicMessagesTransport`
    ///   (native /v1/messages protocol)
    /// - Everything else -> `ChatCompletionsTransport` (OpenAI-compatible /v1/chat/completions)
    ///
    /// **NOTE:** Prefer `from_config_with_tools_via_registry()` for new code.
    pub fn from_config(config: &ProviderConfig, system_prompt: impl Into<String>) -> Self {
        let system_prompt = system_prompt.into();

        if uses_anthropic_protocol(&config.kind) {
            Self::Anthropic {
                transport: AnthropicMessagesTransport::from_config(config, system_prompt),
            }
        } else {
            Self::OpenAIChat {
                transport: ChatCompletionsTransport::from_config(config, system_prompt),
            }
        }
    }

    /// Create a transport with tools for structured tool calling (legacy, non-registry).
    pub fn from_config_with_tools(
        config: &ProviderConfig,
        system_prompt: impl Into<String>,
        tools: Vec<Tool>,
    ) -> Self {
        let system_prompt = system_prompt.into();

        if uses_anthropic_protocol(&config.kind) {
            Self::Anthropic {
                transport: AnthropicMessagesTransport::from_config_with_tools(
                    config,
                    system_prompt,
                    tools,
                ),
            }
        } else {
            Self::OpenAIChat {
                transport: ChatCompletionsTransport::from_config_with_tools(
                    config,
                    system_prompt,
                    tools,
                ),
            }
        }
    }

    /// Create a transport via the global registry, with tools for structured tool calling.
    ///
    /// Uses the registry pattern (T.6) — built-in transports are discovered lazily,
    /// and plugins can register custom transports at runtime.
    ///
    /// Routes to the correct transport based on `config.kind`:
    /// - `Anthropic, MiniMax, MiniMaxOAuth, MiniMaxCN` -> `anthropic_messages` registry entry
    /// - Everything else -> `chat_completions` registry entry
    ///
    /// Tools are serialized into the config before creation.
    pub fn from_config_with_tools_via_registry(
        config: &ProviderConfig,
        system_prompt: &str,
        tools: &[Tool],
    ) -> Arc<dyn TransportProvider + Send + Sync> {
        // Serialize tools into the config for transport creation
        let mut config_with_tools = config.clone();
        config_with_tools.tools_json = if tools.is_empty() {
            None
        } else {
            Some(serde_json::to_value(tools).ok())
        }
        .flatten();

        // Clone `kind` before the match to keep `config_with_tools` intact for later.
        let kind = config_with_tools.kind.clone();
        let transport_name = match provider_kind_to_transport(kind.clone()) {
            Some(oben_models::TransportType::AnthropicMessages) => "anthropic_messages",
            Some(oben_models::TransportType::GeminiNative) => "gemini_native",
            _ => "chat_completions",
        };

        let sp = system_prompt.to_string();
        let tools_vec: Vec<Tool> = config_with_tools
            .tools_json
            .as_ref()
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        registry::get_transport(transport_name, &config_with_tools, &sp).unwrap_or_else(|| {
            tracing::warn!(
                "Transport '{}' not found in registry, falling back to direct construction",
                transport_name
            );
            if uses_anthropic_protocol(&kind) {
                Arc::new(AnthropicMessagesTransport::from_config_with_tools(
                    &config_with_tools,
                    sp,
                    tools_vec,
                )) as Arc<dyn TransportProvider + Send + Sync>
            } else {
                Arc::new(ChatCompletionsTransport::from_config_with_tools(
                    &config_with_tools,
                    sp,
                    tools_vec,
                )) as Arc<dyn TransportProvider + Send + Sync>
            }
        })
    }

    /// Get the provider kind this transport is configured for.
    pub fn provider_kind(&self) -> oben_models::ProviderKind {
        match self {
            Transport::OpenAIChat { transport: _ } => ProviderKind::Custom,
            Transport::Anthropic { .. } => oben_models::ProviderKind::Anthropic,
        }
    }
}

#[async_trait::async_trait]
impl TransportProvider for Transport {
    fn name(&self) -> &str {
        match self {
            Transport::OpenAIChat { transport } => transport.name(),
            Transport::Anthropic { transport } => transport.name(),
        }
    }

    async fn chat(&self, messages: &[Message], mode: &CallMode) -> Result<TransportResponse> {
        match self {
            Transport::OpenAIChat { transport } => transport.chat(messages, mode).await,
            Transport::Anthropic { transport } => transport.chat(messages, mode).await,
        }
    }

    async fn stream_chat(
        &self,
        messages: &[Message],
        mode: &CallMode,
        delta_callback: oben_models::StreamDeltaCallback,
    ) -> Result<TransportResponse> {
        match self {
            Transport::OpenAIChat { transport } => {
                transport.stream_chat(messages, mode, delta_callback).await
            }
            Transport::Anthropic { transport } => {
                transport.stream_chat(messages, mode, delta_callback).await
            }
        }
    }

    fn estimate_tokens(&self, messages: &[Message]) -> usize {
        match self {
            Transport::OpenAIChat { transport } => transport.estimate_tokens(messages),
            Transport::Anthropic { transport } => transport.estimate_tokens(messages),
        }
    }
}

// -- Inherent async methods on Transport (no async_trait) --

impl Transport {
    /// Fetch the list of available models from the underlying provider.
    pub async fn list_models(&self) -> Result<oben_models::ModelListResponse> {
        match self {
            Transport::OpenAIChat { transport } => transport.list_models().await,
            Transport::Anthropic { transport } => transport.list_models().await,
        }
    }

    /// Find a specific model by ID from the underlying provider.
    pub async fn find_model(&self, model_id: &str) -> Result<Option<oben_models::ModelInfo>> {
        match self {
            Transport::OpenAIChat { transport } => transport.find_model(model_id).await,
            Transport::Anthropic { transport } => transport.find_model(model_id).await,
        }
    }
}

// -- Tests --

#[cfg(test)]
mod tests {
    use super::*;

    fn make_anthropic_config() -> ProviderConfig {
        ProviderConfig::new(
            oben_models::ProviderKind::Anthropic,
            "claude-sonnet-4-20250514",
        )
    }

    fn make_openai_config() -> ProviderConfig {
        ProviderConfig::new(oben_models::ProviderKind::OpenAI, "gpt-4")
    }

    fn make_custom_config() -> ProviderConfig {
        let mut config = ProviderConfig::new(oben_models::ProviderKind::Custom, "test-model");
        config.base_url = Some("http://localhost:8000/v1".to_string());
        config
    }

    #[test]
    fn build_anthropic_config_creates_anthropic_transport() {
        // given: ProviderConfig with ProviderKind::Anthropic
        // when: Transport::from_config is called
        // then: returns Transport::Anthropic variant
        let config = make_anthropic_config();
        let transport = Transport::from_config(&config, "test prompt");
        assert!(matches!(transport, Transport::Anthropic { .. }));
        assert_eq!(
            transport.provider_kind(),
            oben_models::ProviderKind::Anthropic
        );
    }

    #[test]
    fn build_openai_config_creates_openai_transport() {
        // given: ProviderConfig with ProviderKind::OpenAI
        // when: Transport::from_config is called
        // then: returns Transport::OpenAIChat variant
        let config = make_openai_config();
        let transport = Transport::from_config(&config, "test prompt");
        assert!(matches!(transport, Transport::OpenAIChat { .. }));
    }

    #[test]
    fn build_custom_config_creates_openai_transport() {
        // given: ProviderConfig with ProviderKind::Custom
        // when: Transport::from_config is called
        // then: returns Transport::OpenAIChat variant
        let config = make_custom_config();
        let transport = Transport::from_config(&config, "test prompt");
        assert!(matches!(transport, Transport::OpenAIChat { .. }));
    }

    #[test]
    fn registry_method_returns_trait_object() {
        // given: ProviderConfig with ProviderKind::OpenAI
        // when: Transport::from_config_with_tools_via_registry is called
        // then: returns Arc<dyn TransportProvider> with correct name
        let config = make_openai_config();
        let transport = Transport::from_config_with_tools_via_registry(&config, "test prompt", &[]);
        assert_eq!(transport.name(), "chat-completions");
    }

    #[test]
    fn registry_method_anthropic_returns_trait_object() {
        // given: ProviderConfig with ProviderKind::Anthropic
        // when: Transport::from_config_with_tools_via_registry is called
        // then: returns Arc<dyn TransportProvider> named "anthropic-messages"
        let config = make_anthropic_config();
        let transport = Transport::from_config_with_tools_via_registry(&config, "test prompt", &[]);
        assert_eq!(transport.name(), "anthropic-messages");
    }
}
