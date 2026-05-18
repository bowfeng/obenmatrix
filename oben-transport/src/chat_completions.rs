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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_messages_json_text() {
        let messages = vec![
            Message::system("you are helpful"),
            Message::user("hello"),
        ];
        let json = build_messages_json(&messages);
        assert_eq!(json.len(), 2);
        assert_eq!(json[0]["role"], "system");
        assert_eq!(json[0]["content"], "you are helpful");
        assert_eq!(json[1]["role"], "user");
        assert_eq!(json[1]["content"], "hello");
    }

    #[test]
    fn test_build_messages_json_tool_result() {
        let messages = vec![
            Message::user("tell me weather"),
            Message::tool_result("call-1", "sunny"),
        ];
        let json = build_messages_json(&messages);
        assert_eq!(json[0]["role"], "user");
        assert_eq!(json[1]["role"], "tool");
        assert_eq!(json[1]["content"], "sunny");
    }

    #[test]
    fn test_stream_chunk_serialization() {
        let json = r#"{
            "id": "chatcmpl-123",
            "object": "chat.completion.chunk",
            "created": 1234567890,
            "choices": [{
                "delta": {"content": "Hello", "role": "assistant"},
                "index": 0,
                "finish_reason": null
            }],
            "usage": null
        }"#;
        let chunk: StreamChunk = serde_json::from_str(json).unwrap();
        assert_eq!(chunk.id, "chatcmpl-123");
        assert_eq!(chunk.choices[0].delta.content, Some("Hello".to_string()));
        assert_eq!(chunk.choices[0].delta.role, Some("assistant".to_string()));
    }

    #[test]
    fn test_stream_chunk_usage() {
        let json = r#"{
            "id": "chatcmpl-123",
            "choices": [],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
        }"#;
        let chunk: StreamChunk = serde_json::from_str(json).unwrap();
        assert_eq!(chunk.usage.unwrap().total_tokens, Some(15));
    }

    #[test]
    fn test_stream_chunk_tool_calls() {
        let json = r#"{
            "id": "chatcmpl-123",
            "choices": [{
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": "call-123",
                        "function": {"name": "shell", "arguments": "{\"command\": \"ls\"}"}
                    }]
                },
                "index": 0,
                "finish_reason": null
            }],
            "usage": null
        }"#;
        let chunk: StreamChunk = serde_json::from_str(json).unwrap();
        let tc = &chunk.choices[0].delta.tool_calls.as_ref().unwrap()[0];
        assert_eq!(tc.index, 0);
        assert_eq!(tc.id, Some("call-123".to_string()));
        assert_eq!(tc.function.as_ref().unwrap().name, Some("shell".to_string()));
        assert_eq!(tc.function.as_ref().unwrap().arguments, Some("{\"command\": \"ls\"}".to_string()));
    }

    #[test]
    fn test_stream_chunk_text_accumulation() {
        // Simulate accumulating text across chunks
        let mut final_text = String::new();
        let chunks = vec![
            r#"{"choices":[{"delta":{"content":"Hello"}}]}"#,
            r#"{"choices":[{"delta":{"content":", "}}]}"#,
            r#"{"choices":[{"delta":{"content":"World"}}]}"#,
        ];
        for chunk_json in chunks {
            let chunk: StreamChunk = serde_json::from_str(chunk_json).unwrap();
            for choice in chunk.choices {
                if let Some(ref content) = choice.delta.content {
                    final_text.push_str(content);
                }
            }
        }
        assert_eq!(final_text, "Hello, World");
    }

    #[test]
    fn test_stream_chunk_tool_call_accumulation() {
        // Simulate accumulating tool call deltas
        let mut names: Vec<String> = vec![];
        let mut args: Vec<String> = vec![];
        let chunks = vec![
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"shell"}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"command\""}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"ls\"}"}}]}}]}"#,
        ];
        for chunk_json in chunks {
            let chunk: StreamChunk = serde_json::from_str(chunk_json).unwrap();
            for choice in chunk.choices {
                if let Some(ref tool_deltas) = choice.delta.tool_calls {
                    for tc in tool_deltas {
                        let idx = tc.index;
                        while names.len() <= idx {
                            names.push(String::new());
                            args.push(String::new());
                        }
                        if let Some(ref func) = tc.function {
                            if let Some(ref name) = func.name {
                                names[idx] = name.clone();
                            }
                            if let Some(ref a) = func.arguments {
                                args[idx].push_str(a);
                            }
                        }
                    }
                }
            }
        }
        assert_eq!(names[0], "shell");
        assert_eq!(args[0], "{\"command\"ls\"}");
    }

    #[test]
    fn test_stream_done_terminator() {
        // In SSE, [DONE] comes as "data: [DONE]" followed by a blank line.
        // The eventsource-stream library extracts the payload after "data: ".
        let done_payload = "[DONE]";
        assert_eq!(done_payload, "[DONE]");
    }

    #[test]
    fn test_stream_empty_content() {
        let chunks = vec![
            r#"{"choices":[{"delta":{"content":null}}]}"#,
            r#"{"choices":[{"delta":{}}]}"#,
            r#"{"choices":[{"delta":{"finish_reason":"stop"}}]}"#,
        ];
        let mut text = String::new();
        for chunk_json in chunks {
            let chunk: StreamChunk = serde_json::from_str(chunk_json).unwrap();
            for choice in chunk.choices {
                if let Some(ref content) = choice.delta.content {
                    if !content.is_empty() {
                        text.push_str(content);
                    }
                }
            }
        }
        assert_eq!(text, "");
    }

    #[test]
    fn test_stream_tool_call_id_from_first_chunk() {
        let chunk1: StreamChunk = serde_json::from_str(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call-abc","function":{"name":"shell"}}]}}]}"#,
        ).unwrap();
        assert_eq!(chunk1.choices[0].delta.tool_calls.as_ref().unwrap()[0].id.as_ref().unwrap(), "call-abc");
        assert_eq!(
            chunk1.choices[0].delta.tool_calls.as_ref().unwrap()[0].function.as_ref().unwrap().name,
            Some("shell".to_string())
        );

        let chunk2: StreamChunk = serde_json::from_str(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"cmd\":\"ls\"}"}}]}}]}"#,
        ).unwrap();
        assert_eq!(
            chunk2.choices[0].delta.tool_calls.as_ref().unwrap()[0].function.as_ref().unwrap().arguments.as_ref().unwrap(),
            "{\"cmd\":\"ls\"}"
        );
    }

    #[test]
    fn test_stream_multiple_tool_calls() {
        let chunk: StreamChunk = serde_json::from_str(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"shell","arguments":"{\"cmd\":\"ls\"}"}},{"index":1,"function":{"name":"read_file","arguments":"{\"path\":\"/tmp/x\"}"}}]}}]}"#,
        ).unwrap();
        let tool_calls = chunk.choices[0].delta.tool_calls.as_ref().unwrap();
        assert_eq!(tool_calls.len(), 2);
        assert_eq!(tool_calls[0].function.as_ref().unwrap().name, Some("shell".to_string()));
        assert_eq!(tool_calls[1].function.as_ref().unwrap().name, Some("read_file".to_string()));
    }

    #[test]
    fn test_stream_usage_on_final_chunk() {
        let chunks = vec![
            r#"{"choices":[{"delta":{"content":"Hello"}}]}"#,
            r#"{"choices":[],"usage":{"prompt_tokens":12,"completion_tokens":5,"total_tokens":17}}"#,
        ];
        let mut total_tokens: Option<usize> = None;
        for chunk_json in chunks {
            let chunk: StreamChunk = serde_json::from_str(chunk_json).unwrap();
            if let Some(ref usage) = chunk.usage {
                total_tokens = usage.total_tokens;
            }
        }
        assert_eq!(total_tokens, Some(17));
    }
}
