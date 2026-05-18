/// Chat Completions transport — OpenAI-compatible API (OpenRouter, OpenAI, NovitaAI, etc.)
/// Maps to `agent/transports/chat_completions.py`.

use anyhow::Result;
use oben_models::{Message, MessageRole, ProviderKind, TransportResponse, TransportToolCall};
use tracing::debug;

use super::base::{BaseTransport, ChatRequest, ChatTool, ToolFunction};

/// Transport that talks to any OpenAI-compatible API.
pub struct ChatCompletionsTransport {
    base: BaseTransport,
}

impl ChatCompletionsTransport {
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            base: BaseTransport::new(base_url, api_key, model),
        }
    }

    /// Create from a ProviderConfig.
    pub fn from_config(config: &oben_models::ProviderConfig) -> Self {
        let kind = &config.kind;
        let base_url = match kind {
            ProviderKind::OpenRouter => "https://openrouter.ai/api/v1".to_string(),
            ProviderKind::OpenAI => "https://api.openai.com/v1".to_string(),
            ProviderKind::Anthropic => "https://api.anthropic.com/v1".to_string(),
            ProviderKind::Bedrock => "https://bedrock-runtime.us-east-1.amazonaws.com/v1".to_string(),
            ProviderKind::Gemini => "https://generativelanguage.googleapis.com/v1".to_string(),
            ProviderKind::LMStudio => "http://localhost:1234/v1".to_string(),
            ProviderKind::Custom { base_url } => base_url.clone(),
        };
        let api_key = config.api_key.clone().unwrap_or_default();
        Self::new(base_url, api_key, &config.model)
    }
}

#[async_trait::async_trait]
impl oben_models::providers::TransportProvider for ChatCompletionsTransport {
    fn name(&self) -> &str {
        "chat-completions"
    }

    async fn chat(&self, messages: &[Message]) -> Result<TransportResponse> {
        let chat_messages: Vec<super::base::ChatMessage> = messages
            .iter()
            .map(|m| {
                let role = match m.role {
                    MessageRole::System => "system".to_string(),
                    MessageRole::User => "user".to_string(),
                    MessageRole::Assistant => "assistant".to_string(),
                    MessageRole::Tool => "tool".to_string(),
                };
                let content = match &m.content {
                    oben_models::MessageContent::Text(t) => {
                        super::base::MessageContent::text(t)
                    }
                    oben_models::MessageContent::Image { url, detail } => {
                        let parts = vec![
                            super::base::ChatMessagePart {
                                part_type: "text".to_string(),
                                text: Some("I see an image. Let me analyze it.".to_string()),
                                image_url: None,
                            },
                            super::base::ChatMessagePart {
                                part_type: "image_url".to_string(),
                                text: None,
                                image_url: Some(super::base::ImageUrl {
                                    url: url.clone(),
                                    detail: detail.clone(),
                                }),
                            },
                        ];
                        super::base::MessageContent::parts(parts)
                    }
                    oben_models::MessageContent::Parts(parts) => {
                        let chat_parts: Vec<super::base::ChatMessagePart> = parts
                            .iter()
                            .map(|p| match p {
                                oben_models::MessagePart::Text(t) => super::base::ChatMessagePart {
                                    part_type: "text".to_string(),
                                    text: Some(t.clone()),
                                    image_url: None,
                                },
                                oben_models::MessagePart::Image { url, detail } => super::base::ChatMessagePart {
                                    part_type: "image_url".to_string(),
                                    text: None,
                                    image_url: Some(super::base::ImageUrl {
                                        url: url.clone(),
                                        detail: detail.clone(),
                                    }),
                                },
                            })
                            .collect();
                        super::base::MessageContent::parts(chat_parts)
                    }
                };
                super::base::ChatMessage { role, content }
            })
            .collect();

        let request = ChatRequest {
            model: self.base.model.clone(),
            messages: chat_messages,
            temperature: Some(0.7),
            max_tokens: Some(8192),
            tools: None,
            stream: None,
        };

        let response = self.base.send_request(request).await?;

        let choice = response.choices.first().ok_or_else(|| anyhow::anyhow!("No response choices"))?;
        let text = choice.message.content.clone().unwrap_or_default();
        let tool_calls: Vec<TransportToolCall> = choice
            .message
            .tool_calls
            .iter()
            .flatten()
            .map(|tc| TransportToolCall {
                id: tc.id.clone(),
                tool_name: tc.function.name.clone(),
                arguments: serde_json::from_str(&tc.function.arguments).unwrap_or_default(),
            })
            .collect();

        Ok(TransportResponse {
            text,
            tool_calls,
            tokens_used: response.usage.and_then(|u| u.total_tokens),
        })
    }
}
