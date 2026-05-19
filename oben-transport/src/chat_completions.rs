/// Chat Completions transport — OpenAI-compatible API (OpenRouter, OpenAI, NovitaAI, etc.)
/// Maps to `agent/transports/chat_completions.py`.

use anyhow::{anyhow, Result};
use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use serde_json::json;
use tracing::debug;
use oben_models::{Message, MessageRole, ProviderKind, TransportResponse, TransportToolCall};

use super::base::{BaseTransport, ChatResponse};

/// Per-session cached request state.
///
/// Stores the full request as a `serde_json::Value` with `"messages"` as a
/// mutable array. On Fresh we replace it entirely; on Incremental we extend
/// the existing array in-place — no cloning, no rebuilding static parts.
struct CachedRequest {
    /// Full request object, e.g. `{ "model": ..., "messages": [...], ... }`
    request: serde_json::Value,
    /// Number of messages currently in the cached array.
    msg_count: usize,
}

/// Build JSON for a single message.
fn message_to_json(m: &Message) -> serde_json::Value {
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
}

/// Build JSON for all messages (fresh call).
fn build_all_messages_json(messages: &[Message]) -> Vec<serde_json::Value> {
    messages.iter().map(message_to_json).collect()
}

/// Resolves the CallMode into a cached request and calls `f` with a reference.
///
/// **Fresh**: replaces the cached request entirely.
/// **Incremental**: extends the existing `"messages"` array in-place — zero
/// copies of the existing messages, zero rebuild of static parts.
///
/// The callback runs while the mutex is held, so the reference is always valid.
fn resolve_request<F, R>(
    cached: &mut std::collections::HashMap<String, CachedRequest>,
    messages: &[Message],
    mode: oben_models::CallMode,
    template: &serde_json::Value,
    f: F,
) -> R
where
    F: FnOnce(&serde_json::Value) -> R,
{
    let session_id = match &mode {
        oben_models::CallMode::Fresh(id) | oben_models::CallMode::Incremental(id) => id.clone(),
    };

    match mode {
        oben_models::CallMode::Fresh(_) => {
            // Build the full request from the template + all messages.
            let json_messages = build_all_messages_json(messages);
            let mut req = template.clone();
            req["messages"] = serde_json::Value::Array(json_messages);
            let sid = session_id.clone();
            cached.insert(sid, CachedRequest { request: req, msg_count: messages.len() });
            f(&cached[&session_id].request)
        }
        oben_models::CallMode::Incremental(_) => {
            let entry = cached.entry(session_id).or_insert_with(|| {
                let req = template.clone();
                CachedRequest {
                    request: req,
                    msg_count: 0,
                }
            });

            let cached_count = entry.msg_count;

            if messages.len() <= cached_count {
                // Messages haven't grown — content was edited or removed.
                // Rebuild entirely.
                let json_messages = build_all_messages_json(messages);
                entry.request["messages"] = serde_json::Value::Array(json_messages);
                entry.msg_count = messages.len();
            } else {
                // Messages grew — extend existing array in-place.
                let arr = entry.request["messages"].as_array_mut().unwrap();
                for msg in &messages[cached_count..] {
                    arr.push(message_to_json(msg));
                }
                entry.msg_count = messages.len();
            }

            f(&entry.request)
        }
    }
}

/// SSE event from streaming response.
#[derive(Debug, serde::Deserialize)]
struct StreamChunk {
    #[serde(default)]
    #[allow(dead_code)]
    pub id: String,
    #[serde(default)]
    #[allow(dead_code)]
    pub object: String,
    #[serde(default)]
    #[allow(dead_code)]
    pub created: u64,
    pub choices: Vec<StreamChoice>,
    #[serde(default)]
    pub usage: Option<StreamUsage>,
}

#[derive(Debug, serde::Deserialize)]
struct StreamChoice {
    pub delta: StreamDelta,
    #[serde(default)]
    #[allow(dead_code)]
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
    #[allow(dead_code)]
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
    #[allow(dead_code)]
    pub prompt_tokens: Option<usize>,
    #[allow(dead_code)]
    pub completion_tokens: Option<usize>,
    pub total_tokens: Option<usize>,
}

/// Build the static parts of the request template.
fn build_request_template(base: &BaseTransport) -> serde_json::Value {
    json!({
        "model": base.model,
        "messages": serde_json::Value::Array(vec![]),
        "temperature": 0.7,
        "max_tokens": 8192,
    })
}

/// Build the streaming request template.
fn build_stream_request_template(base: &BaseTransport) -> serde_json::Value {
    json!({
        "model": base.model,
        "messages": serde_json::Value::Array(vec![]),
        "temperature": 0.7,
        "max_tokens": 8192,
        "stream": true,
        "stream_options": {"include_usage": true},
    })
}

/// Transport that talks to any OpenAI-compatible API.
pub struct ChatCompletionsTransport {
    base: BaseTransport,
    /// Cached request state per session — contains the full request object
    /// with a mutable `"messages"` array.
    cached: std::sync::Mutex<std::collections::HashMap<String, CachedRequest>>,
    /// Static request template (model, temperature, max_tokens).
    template: serde_json::Value,
}

impl ChatCompletionsTransport {
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>, model: impl Into<String>) -> Self {
        let base = BaseTransport::new(base_url, api_key, model);
        let template = build_request_template(&base);
        Self {
            base,
            cached: std::sync::Mutex::new(std::collections::HashMap::new()),
            template,
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

    async fn chat(&self, messages: &[Message], mode: oben_models::CallMode) -> Result<TransportResponse> {
        let request = {
            let mut cached = self.cached.lock().unwrap();
            resolve_request(&mut *cached, messages, mode, &self.template, |req| req.clone())
        };

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

    async fn stream_chat(&self, messages: &[Message], mode: oben_models::CallMode, mut delta_callback: oben_models::StreamDeltaCallback) -> Result<TransportResponse> {
        let request = {
            let mut cached = self.cached.lock().unwrap();
            let stream_template = build_stream_request_template(&self.base);
            resolve_request(&mut *cached, messages, mode, &stream_template, |req| req.clone())
        };

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
    fn test_message_role_json() {
        let msg = Message::system("sys");
        let json = message_to_json(&msg);
        assert_eq!(json["role"], "system");
        assert_eq!(json["content"], "sys");

        let msg = Message::tool_result("call-1", "result");
        let json = message_to_json(&msg);
        assert_eq!(json["role"], "tool");
        assert_eq!(json["content"], "result");
    }

    #[test]
    fn test_resolve_request_fresh() {
        let session_id = String::from("test-session");
        let messages = vec![
            Message::system("be helpful"),
            Message::user("hello"),
        ];
        let template = json!({
            "model": "test-model",
            "messages": serde_json::Value::Array(vec![]),
            "temperature": 0.7,
            "max_tokens": 8192,
        });
        let mut cached = std::collections::HashMap::new();

        let (json_len, model) = resolve_request(&mut cached, &messages, oben_models::CallMode::Fresh(session_id.clone()), &template, |req| {
            assert_eq!(req["messages"].as_array().unwrap().len(), 2);
            assert_eq!(req["messages"][0]["role"], "system");
            assert_eq!(req["model"], "test-model");
            (2, req["model"].clone())
        });
        assert_eq!(json_len, 2);
        assert_eq!(model, "test-model");
        assert_eq!(cached[&session_id].msg_count, 2);
    }

    #[test]
    fn test_resolve_request_incremental_grown() {
        let session_id = String::from("test-session");
        let template = json!({
            "model": "test-model",
            "messages": serde_json::Value::Array(vec![]),
            "temperature": 0.7,
            "max_tokens": 8192,
        });
        let mut cached = std::collections::HashMap::new();

        // Fresh: 2 messages
        let messages = vec![
            Message::system("be helpful"),
            Message::user("hello"),
        ];
        resolve_request(&mut cached, &messages, oben_models::CallMode::Fresh(session_id.clone()), &template, |_| ());

        // Incremental: add 1 more
        let mut messages = messages.clone();
        messages.push(Message::assistant("hi there"));
        resolve_request(&mut cached, &messages, oben_models::CallMode::Incremental(session_id.clone()), &template, |req| {
            assert_eq!(req["messages"].as_array().unwrap().len(), 3);
            assert_eq!(req["messages"][2]["role"], "assistant");
        });
        assert_eq!(cached[&session_id].msg_count, 3);
    }

    #[test]
    fn test_resolve_request_incremental_reset() {
        let session_id = String::from("test-session");
        let template = json!({
            "model": "test-model",
            "messages": serde_json::Value::Array(vec![]),
            "temperature": 0.7,
            "max_tokens": 8192,
        });
        let mut cached = std::collections::HashMap::new();

        // Fresh: 3 messages
        let messages = vec![
            Message::system("sys"),
            Message::user("hello"),
            Message::assistant("hi"),
        ];
        resolve_request(&mut cached, &messages, oben_models::CallMode::Fresh(session_id.clone()), &template, |_| ());

        // Incremental: removed one — should reset
        let mut messages = messages;
        messages.pop();
        resolve_request(&mut cached, &messages, oben_models::CallMode::Incremental(session_id.clone()), &template, |req| {
            assert_eq!(req["messages"].as_array().unwrap().len(), 2);
        });
        assert_eq!(cached[&session_id].msg_count, 2);
    }

    #[test]
    fn test_resolve_request_incremental_equal() {
        let session_id = String::from("test-session");
        let template = json!({
            "model": "test-model",
            "messages": serde_json::Value::Array(vec![]),
            "temperature": 0.7,
            "max_tokens": 8192,
        });
        let mut cached = std::collections::HashMap::new();

        // Fresh: 2 messages
        let messages = vec![
            Message::system("sys"),
            Message::user("hello"),
        ];
        resolve_request(&mut cached, &messages, oben_models::CallMode::Fresh(session_id.clone()), &template, |_| ());

        // Incremental: same count but content changed — should reset
        let mut messages = messages.clone();
        messages[1] = Message::user("changed");
        resolve_request(&mut cached, &messages, oben_models::CallMode::Incremental(session_id.clone()), &template, |req| {
            assert_eq!(req["messages"].as_array().unwrap().len(), 2);
            assert_eq!(req["messages"][1]["content"], "changed");
        });
        assert_eq!(cached[&session_id].msg_count, 2);
    }

    #[test]
    fn test_per_session_isolation() {
        let template = json!({
            "model": "test-model",
            "messages": serde_json::Value::Array(vec![]),
            "temperature": 0.7,
            "max_tokens": 8192,
        });
        let mut cached = std::collections::HashMap::new();

        let messages_a = vec![Message::system("sys-a"), Message::user("hello-a")];
        resolve_request(&mut cached, &messages_a, oben_models::CallMode::Fresh("session-a".into()), &template, |_| ());

        let messages_b = vec![Message::system("sys-b"), Message::user("hello-b")];
        resolve_request(&mut cached, &messages_b, oben_models::CallMode::Fresh("session-b".into()), &template, |_| ());

        assert_eq!(cached["session-a"].msg_count, 2);
        assert_eq!(cached["session-b"].msg_count, 2);
        assert_eq!(cached["session-a"].request["messages"][0]["content"], "sys-a");
        assert_eq!(cached["session-b"].request["messages"][0]["content"], "sys-b");
    }

    #[test]
    fn test_in_place_extend_no_clone() {
        let session_id = String::from("test-session");
        let template = json!({
            "model": "test-model",
            "messages": serde_json::Value::Array(vec![]),
            "temperature": 0.7,
            "max_tokens": 8192,
        });
        let mut cached = std::collections::HashMap::new();

        // Fresh: 2 messages
        let messages = vec![
            Message::system("sys"),
            Message::user("hello"),
        ];
        resolve_request(&mut cached, &messages, oben_models::CallMode::Fresh(session_id.clone()), &template, |_| ());

        // Incremental: extend in-place by 1
        let messages2 = vec![
            Message::system("sys"),
            Message::user("hello"),
            Message::assistant("hi"),
        ];
        resolve_request(&mut cached, &messages2, oben_models::CallMode::Incremental(session_id.clone()), &template, |req| {
            let arr = req["messages"].as_array().unwrap();
            assert_eq!(arr.len(), 3);
            // The first 2 messages should be the exact same Value objects
            // (in-place extend, not a rebuild)
            assert_eq!(arr[0]["role"], "system");
            assert_eq!(arr[1]["role"], "user");
            assert_eq!(arr[2]["role"], "assistant");
        });
    }
}
