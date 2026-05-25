/// Unified transport dispatcher — selects the right LLM transport implementation
/// based on the ProviderKind in the config.

use anyhow::Result;
use oben_models::{
    CallMode, Message,
    providers::{ProviderConfig, TransportProvider, TransportResponse, TransportToolCall},
    Tool,
};

use super::{
    anthropic_messages::AnthropicMessagesTransport,
    base::BaseTransport,
    chat_completions::ChatCompletionsTransport,
};

/// Unified transport enum — wraps ChatCompletionsTransport + AnthropicMessagesTransport.
pub enum Transport {
    /// OpenAI-compatible API (Chat Completions).
    OpenAIChat {
        transport: ChatCompletionsTransport,
    },
    /// Anthropic native Messages API.
    Anthropic {
        transport: AnthropicMessagesTransport,
    },
}

impl Transport {
    /// Create a transport instance from a ProviderConfig.
    ///
    /// Routes to the correct transport type based on `config.kind`:
    /// - `Anthropic` -> `AnthropicMessagesTransport` (native /v1/messages)
    /// - Everything else -> `ChatCompletionsTransport` (OpenAI-compatible /v1/chat/completions)
    pub fn from_config(
        config: &ProviderConfig,
        system_prompt: impl Into<String>,
    ) -> Self {
        let system_prompt = system_prompt.into();

        match config.kind {
            oben_models::ProviderKind::Anthropic => {
                Self::Anthropic {
                    transport: AnthropicMessagesTransport::from_config(config, system_prompt),
                }
            }
            _ => {
                Self::OpenAIChat {
                    transport: ChatCompletionsTransport::from_config(config, system_prompt),
                }
            }
        }
    }

    /// Create a transport with tools for structured tool calling.
    pub fn from_config_with_tools(
        config: &ProviderConfig,
        system_prompt: impl Into<String>,
        tools: Vec<Tool>,
    ) -> Self {
        let system_prompt = system_prompt.into();

        match config.kind {
            oben_models::ProviderKind::Anthropic => {
                Self::Anthropic {
                    transport: AnthropicMessagesTransport::from_config_with_tools(
                        config, system_prompt, tools,
                    ),
                }
            }
            _ => {
                Self::OpenAIChat {
                    transport: ChatCompletionsTransport::from_config_with_tools(
                        config, system_prompt, tools,
                    ),
                }
            }
        }
    }

    /// Get the provider kind this transport is configured for.
    pub fn provider_kind(&self) -> oben_models::ProviderKind {
        match self {
            Transport::OpenAIChat { transport } => {
                // Infer from base URL of the inner BaseTransport
                let t = transport;
                use oben_models::ProviderKind;
                // Try to infer from the config that was used to create this transport.
                // Since we don't store it, we check internal state.
                // Fallback: this should never be needed in practice since the
                // original ProviderKind was used to construct the right variant.
                ProviderKind::Custom
            }
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
            Transport::Anthropic { .. } => {
                // AnthropicMessagesTransport doesn't expose list_models yet.
                // TODO: implement via base_url/api_key from config
                Err(anyhow::anyhow!("list_models not yet implemented for Anthropic transport"))
            }
        }
    }

    /// Find a specific model by ID from the underlying provider.
    pub async fn find_model(&self, model_id: &str) -> Result<Option<oben_models::ModelInfo>> {
        match self {
            Transport::OpenAIChat { transport } => transport.find_model(model_id).await,
            Transport::Anthropic { .. } => {
                Err(anyhow::anyhow!("find_model not yet implemented for Anthropic transport"))
            }
        }
    }
}

// -- Tests --

#[cfg(test)]
mod tests {
    use super::*;

    fn make_anthropic_config() -> ProviderConfig {
        ProviderConfig::new(oben_models::ProviderKind::Anthropic, "claude-sonnet-4-20250514")
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
        /// given: ProviderConfig with ProviderKind::Anthropic
        /// when: Transport::from_config is called
        /// then: returns Transport::Anthropic variant
        let config = make_anthropic_config();
        let transport = Transport::from_config(&config, "test prompt");
        assert!(matches!(transport, Transport::Anthropic { .. }));
        assert_eq!(transport.provider_kind(), oben_models::ProviderKind::Anthropic);
    }

    #[test]
    fn build_openai_config_creates_openai_transport() {
        /// given: ProviderConfig with ProviderKind::OpenAI
        /// when: Transport::from_config is called
        /// then: returns Transport::OpenAIChat variant
        let config = make_openai_config();
        let transport = Transport::from_config(&config, "test prompt");
        assert!(matches!(transport, Transport::OpenAIChat { .. }));
    }

    #[test]
    fn build_custom_config_creates_openai_transport() {
        /// given: ProviderConfig with ProviderKind::Custom
        /// when: Transport::from_config is called
        /// then: returns Transport::OpenAIChat variant
        let config = make_custom_config();
        let transport = Transport::from_config(&config, "test prompt");
        assert!(matches!(transport, Transport::OpenAIChat { .. }));
    }
}
