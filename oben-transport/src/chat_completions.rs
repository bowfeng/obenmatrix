/// Chat Completions transport — OpenAI-compatible API (OpenRouter, OpenAI, NovitaAI, etc.)
/// Maps to `agent/transports/chat_completions.py`.

use anyhow::Result;
use serde_json::json;
use tracing::debug;
use oben_models::{Message, MessageRole, ProviderKind, TransportResponse, TransportToolCall};

use super::base::{BaseTransport, ChatResponse};

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
        let base_url = match config.kind {
            ProviderKind::OpenRouter => "https://openrouter.ai/api/v1".to_string(),
            ProviderKind::OpenAI => "https://api.openai.com/v1".to_string(),
            ProviderKind::Anthropic => "https://api.anthropic.com/v1".to_string(),
            ProviderKind::Bedrock => "https://bedrock-runtime.us-east-1.amazonaws.com/v1".to_string(),
            ProviderKind::Gemini => "https://generativelanguage.googleapis.com/v1".to_string(),
            ProviderKind::LMStudio => "http://localhost:1234/v1".to_string(),
            ProviderKind::Custom => config.base_url.clone().unwrap_or_default(),
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
        // Build request as raw JSON to avoid serde serialization issues
        let messages_json: Vec<serde_json::Value> = messages
            .iter()
            .map(|m| {
                let role = match m.role {
                    MessageRole::System => "system",
                    MessageRole::User => "user",
                    MessageRole::Assistant => "assistant",
                    MessageRole::Tool => "tool",
                };
                match &m.content {
                    oben_models::MessageContent::Text(t) => {
                        json!({"role": role, "content": t})
                    }
                    oben_models::MessageContent::Image { url, detail } => {
                        let mut img = json!({
                            "type": "image_url",
                            "image_url": { "url": url }
                        });
                        if let Some(d) = detail {
                            img["image_url"]["detail"] = json!(d);
                        }
                        json!({
                            "role": role,
                            "content": [
                                { "type": "text", "text": "I see an image. Let me analyze it." },
                                img
                            ]
                        })
                    }
                    oben_models::MessageContent::Parts(parts) => {
                        let parts_json: Vec<serde_json::Value> = parts
                            .iter()
                            .map(|p| match p {
                                oben_models::MessagePart::Text(t) => {
                                    json!({"type": "text", "text": t})
                                }
                                oben_models::MessagePart::Image { url, detail } => {
                                    let mut img = json!({"type": "image_url", "image_url": {"url": url}});
                                    if let Some(d) = detail {
                                        img["image_url"]["detail"] = json!(d);
                                    }
                                    img
                                }
                            })
                            .collect();
                        json!({"role": role, "content": parts_json})
                    }
                }
            })
            .collect();

        let request = json!({
            "model": self.base.model,
            "messages": messages_json,
            "temperature": 0.7,
            "max_tokens": 8192,
        });

        let url = format!("{}/chat/completions", self.base.base_url);

        debug!("Requesting {}: model={}, messages={}", url, self.base.model, messages.len());

        let mut req = self.base.client.post(&url).json(&request);
        if !self.base.api_key.is_empty() {
            req = req.bearer_auth(&self.base.api_key);
        }
        let response = req.send().await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await?;
            return Err(anyhow::anyhow!("API error {}: {}", status, body));
        }

        let resp: ChatResponse = response.json().await?;

        let choice = resp.choices.first().ok_or_else(|| anyhow::anyhow!("No response choices"))?;
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
            tokens_used: resp.usage.and_then(|u| u.total_tokens),
        })
    }
}
