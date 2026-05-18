/// Chat Completions transport — OpenAI-compatible API (OpenRouter, OpenAI, NovitaAI, etc.)
/// Maps to `agent/transports/chat_completions.py`.

use anyhow::{anyhow, Result};
use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use serde_json::json;
use tracing::debug;
use oben_models::{Message, MessageRole, ProviderKind, TransportResponse, TransportToolCall};

use super::base::{BaseTransport, ChatResponse};

/// SSE event from streaming response.
#[derive(Debug, serde::Deserialize)]
struct StreamChunk {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub object: String,
    #[serde(default)]
    pub created: u64,
    pub choices: Vec<StreamChoice>,
    #[serde(default)]
    pub usage: Option<StreamUsage>,
}

#[derive(Debug, serde::Deserialize)]
struct StreamChoice {
    pub delta: StreamDelta,
    #[serde(default)]
    pub index: usize,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct StreamDelta {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default, rename = "tool_calls")]
    pub tool_calls: Option<Vec<StreamToolCallDelta>>,
    #[serde(default)]
    pub role: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct StreamToolCallDelta {
    #[serde(default)]
    pub index: usize,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub function: Option<StreamFunctionDelta>,
}

#[derive(Debug, serde::Deserialize)]
struct StreamFunctionDelta {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct StreamUsage {
    pub prompt_tokens: Option<usize>,
    pub completion_tokens: Option<usize>,
    pub total_tokens: Option<usize>,
}

/// Build JSON representation of messages for the API.
fn build_messages_json(messages: &[Message]) -> Vec<serde_json::Value> {
    messages
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
        .collect()
}

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

    /// Fetch the list of available models from the provider.
    pub async fn list_models(&self) -> Result<oben_models::ModelListResponse> {
        self.base.list_models().await
    }

    /// Find a specific model by ID from the provider.
    pub async fn find_model(&self, model_id: &str) -> Result<Option<oben_models::ModelInfo>> {
        self.base.find_model(model_id).await
    }
}

#[async_trait::async_trait]
impl oben_models::providers::TransportProvider for ChatCompletionsTransport {
    fn name(&self) -> &str {
        "chat-completions"
    }

    async fn chat(&self, messages: &[Message]) -> Result<TransportResponse> {
        let messages_json = build_messages_json(messages);

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

    async fn stream_chat(&self, messages: &[Message], mut delta_callback: oben_models::StreamDeltaCallback) -> Result<TransportResponse> {
        let messages_json = build_messages_json(messages);

        let request = json!({
            "model": self.base.model,
            "messages": messages_json,
            "temperature": 0.7,
            "max_tokens": 8192,
            "stream": true,
            "stream_options": {"include_usage": true},
        });

        let url = format!("{}/chat/completions", self.base.base_url);
        debug!("Streaming request to {}", url);

        let mut req = self.base.client.post(&url).json(&request);
        if !self.base.api_key.is_empty() {
            req = req.bearer_auth(&self.base.api_key);
        }
        let response = req.send().await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await?;
            return Err(anyhow!("API error {}: {}", status, body));
        }

        // Parse SSE stream
        let body = response.bytes_stream();
        let mut stream = body.eventsource();

        let mut final_text = String::new();
        let mut tool_call_names: Vec<String> = Vec::new();
        let mut tool_call_args: Vec<String> = Vec::new();
        let mut tool_call_ids: Vec<String> = Vec::new();
        let mut total_tokens: Option<usize> = None;

        while let Some(event_result) = stream.next().await {
            let event = match event_result {
                Ok(e) => e,
                Err(e) => {
                    debug!("SSE stream error: {}", e);
                    break;
                }
            };

            // Check for stream termination
            if event.data.trim() == "[DONE]" {
                break;
            }

            // Deserialize the SSE event data
            let chunk: StreamChunk = match serde_json::from_str(&event.data) {
                Ok(c) => c,
                Err(e) => {
                    debug!("Failed to parse SSE event: {}", e);
                    continue;
                }
            };

            // Capture usage info
            if let Some(usage) = chunk.usage {
                total_tokens = usage.total_tokens;
            }

            // Process choices
            for choice in chunk.choices {
                if let Some(ref finish) = choice.finish_reason {
                    // We don't currently use finish_reason, but it's captured for potential future use
                    let _ = finish;
                }

                let delta = choice.delta;

                // Accumulate text content and fire callback
                if let Some(ref content) = delta.content {
                    if !content.is_empty() {
                        final_text.push_str(content);
                        delta_callback(content);
                    }
                }

                // Accumulate tool call deltas
                if let Some(ref tool_deltas) = delta.tool_calls {
                    for tc in tool_deltas {
                        let idx = tc.index;
                        // Ensure vectors are large enough
                        while tool_call_args.len() <= idx {
                            tool_call_args.push(String::new());
                            tool_call_names.push(String::new());
                            tool_call_ids.push(String::new());
                        }
                        if let Some(ref func) = tc.function {
                            if let Some(ref name) = func.name {
                                tool_call_names[idx] = name.clone();
                            }
                            if let Some(ref args) = func.arguments {
                                tool_call_args[idx].push_str(args);
                            }
                        }
                        if let Some(ref tc_id) = tc.id {
                            tool_call_ids[idx] = tc_id.clone();
                        }
                    }
                }
            }
        }

        // Build final tool_calls from accumulated deltas
        let mut tool_calls: Vec<TransportToolCall> = Vec::new();
        for (idx, (name, args)) in tool_call_names.iter().zip(tool_call_args.iter()).enumerate() {
            let args_json = if args.is_empty() {
                serde_json::Value::Object(serde_json::Map::new())
            } else {
                serde_json::from_str(args).unwrap_or_else(|_| serde_json::json!(args))
            };
            let tc_id = if idx < tool_call_ids.len() && !tool_call_ids[idx].is_empty() {
                tool_call_ids[idx].clone()
            } else {
                format!("call_{}", idx)
            };
            tool_calls.push(TransportToolCall {
                id: tc_id,
                tool_name: name.clone(),
                arguments: args_json,
            });
        }

        Ok(TransportResponse {
            text: final_text,
            tool_calls,
            tokens_used: total_tokens,
        })
    }
}
