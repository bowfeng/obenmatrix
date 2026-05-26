/// Base types for transport implementations.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use tracing::debug;

/// Common API request structure (OpenAI-compatible).
#[derive(Debug, Serialize, Deserialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub temperature: Option<f64>,
    pub max_tokens: Option<usize>,
    pub tools: Option<Vec<ChatTool>>,
    pub stream: Option<bool>,
}

/// A message in a chat request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[derive(PartialEq)]
pub struct ChatMessage {
    pub role: String,
    pub content: MessageContent,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Parts(Vec<ChatMessagePart>),
}

impl MessageContent {
    pub fn text(text: impl Into<String>) -> Self {
        MessageContent::Text(text.into())
    }

    pub fn parts(parts: Vec<ChatMessagePart>) -> Self {
        MessageContent::Parts(parts)
    }
}

/// A part in a multi-part message.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChatMessagePart {
    #[serde(rename = "type")]
    pub part_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_url: Option<ImageUrl>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImageUrl {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// A tool definition for the API.
#[derive(Debug, Serialize, Deserialize)]
pub struct ChatTool {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: ToolFunction,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ToolFunction {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// API response structure (OpenAI-compatible).
#[derive(Debug, Deserialize)]
pub struct ChatResponse {
    pub choices: Vec<ChatChoice>,
    pub usage: Option<Usage>,
}

#[derive(Debug, Deserialize)]
pub struct ChatChoice {
    pub message: ChatResponseMessage,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ChatResponseMessage {
    pub role: String,
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_message_content")]
    pub content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<ToolCall>>,
}

fn deserialize_message_content<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value: serde_json::Value = serde::Deserialize::deserialize(deserializer)?;
    match value {
        serde_json::Value::String(s) => Ok(Some(s)),
        serde_json::Value::Null => Ok(None),
        _ => Ok(Some(value.to_string())),
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: FunctionCall,
}

#[derive(Debug, Deserialize, Clone)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

/// Token usage info.
#[derive(Debug, Deserialize)]
pub struct Usage {
    pub prompt_tokens: Option<usize>,
    pub completion_tokens: Option<usize>,
    pub total_tokens: Option<usize>,
}

/// Base transport that handles HTTP calls.
pub struct BaseTransport {
    pub client: reqwest::Client,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

impl BaseTransport {
    /// Fetch model list from `/v1/models`.
    pub async fn list_models(&self) -> Result<oben_models::ModelListResponse> {
        let url = format!("{}/models", self.base_url.trim_end_matches('/'));
        debug!("Fetching models from: {}", url);
        let mut req = self.client.get(&url);
        if !self.api_key.is_empty() {
            req = req.bearer_auth(&self.api_key);
        }
        let response = req.send().await?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await?;
            return Err(anyhow!("API error {}: {}", status, body));
        }
        let resp: oben_models::ModelListResponse = response.json().await?;
        Ok(resp)
    }

    /// Find a specific model by ID from the API.
    pub async fn find_model(&self, model_id: &str) -> Result<Option<oben_models::ModelInfo>> {
        let list = self.list_models().await?;
        Ok(list.data.into_iter().find(|m| m.id == model_id))
    }
}

impl BaseTransport {
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>, model: impl Into<String>) -> Self {
        // Optimized HTTP client: connection pooling, keep-alive, Nagle disabled
        // for lower-latency small-message workloads (LLM API calls).
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .pool_max_idle_per_host(20)
            .pool_idle_timeout(std::time::Duration::from_secs(90))
            .tcp_nodelay(true)
            .http2_keep_alive_interval(std::time::Duration::from_secs(30))
            .http2_keep_alive_timeout(std::time::Duration::from_secs(10))
            .http2_keep_alive_while_idle(true)
            .build()
            .expect("Failed to build HTTP client");
        Self {
            client,
            base_url: base_url.into(),
            api_key: api_key.into(),
            model: model.into(),
        }
    }

    pub async fn send_request(&self, request: ChatRequest) -> Result<ChatResponse> {
        let url = format!("{}/chat/completions", self.base_url);

        debug!("Requesting {}: model={}, messages={}", url, request.model, request.messages.len());

        let mut req = self.client.post(&url).json(&request);
        if !self.api_key.is_empty() {
            req = req.bearer_auth(&self.api_key);
        }
        let response = req.send().await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await?;
            return Err(anyhow!("API error {}: {}", status, body));
        }

        let resp: ChatResponse = response.json().await?;
        Ok(resp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oben_models::ProviderKind;

    #[test]
    fn test_chat_request_serialization() {
        let req = ChatRequest {
            model: "test-model".to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: MessageContent::text("hello"),
            }],
            temperature: Some(0.7),
            max_tokens: Some(512),
            tools: None,
            stream: Some(false),
        };
        let json = serde_json::to_string(&req).unwrap();
        let parsed: ChatRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.model, "test-model");
        assert_eq!(parsed.temperature, Some(0.7));
        assert_eq!(parsed.max_tokens, Some(512));
    }

    #[test]
    fn test_message_content_text_roundtrip() {
        let msg = ChatMessage {
            role: "user".to_string(),
            content: MessageContent::text("test content"),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: ChatMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back.content, MessageContent::text("test content"));
    }

    #[test]
    fn test_message_content_parts_roundtrip() {
        let msg = ChatMessage {
            role: "user".to_string(),
            content: MessageContent::parts(vec![ChatMessagePart {
                part_type: "text".to_string(),
                text: Some("hello".to_string()),
                image_url: None,
            }]),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: ChatMessage = serde_json::from_str(&json).unwrap();
        assert!(matches!(back.content, MessageContent::Parts(_)));
    }

    #[test]
    fn test_tool_call_deserialization() {
        let json = r#"{
            "id": "call-123",
            "type": "function",
            "function": {"name": "shell", "arguments": "{\"command\": \"ls\"}"}
        }"#;
        let tc: ToolCall = serde_json::from_str(json).unwrap();
        assert_eq!(tc.id, "call-123");
        assert_eq!(tc.tool_type, "function");
        assert_eq!(tc.function.name, "shell");
        assert_eq!(tc.function.arguments, "{\"command\": \"ls\"}");
    }

    #[test]
    fn test_chat_response_parsing() {
        let json = r#"{
            "choices": [{
                "message": {"role": "assistant", "content": "Hello world", "tool_calls": null},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
        }"#;
        let resp: ChatResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.choices.len(), 1);
        assert_eq!(resp.choices[0].message.content, Some("Hello world".into()));
        assert_eq!(resp.usage.unwrap().total_tokens, Some(15));
    }

    #[test]
    fn test_chat_response_tool_calls() {
        let json = r#"{
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call-1",
                        "type": "function",
                        "function": {"name": "shell", "arguments": "{\"command\": \"echo hi\"}"}
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": null
        }"#;
        let resp: ChatResponse = serde_json::from_str(json).unwrap();
        let tcs = resp.choices[0].message.tool_calls.as_ref().unwrap();
        assert_eq!(tcs.len(), 1);
        assert_eq!(tcs[0].function.name, "shell");
    }

    #[test]
    fn test_chat_response_null_content() {
        let json = r#"{
            "choices": [{
                "message": {"role": "assistant", "content": null, "tool_calls": null},
                "finish_reason": null
            }],
            "usage": null
        }"#;
        let resp: ChatResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.choices[0].message.content, None);
    }

    #[test]
    fn test_chat_response_null_content_string_variant() {
        let json = r#"{
            "choices": [{
                "message": {"role": "assistant", "content": 123, "tool_calls": null},
                "finish_reason": null
            }],
            "usage": null
        }"#;
        let resp: ChatResponse = serde_json::from_str(json).unwrap();
        // Non-string content should be coerced via deserialize_message_content
        assert_eq!(resp.choices[0].message.content, Some("123".to_string()));
    }

    #[test]
    fn test_model_response_roundtrip() {
        let json = r#"{
            "object": "list",
            "data": [{"id": "gpt-4", "object": "model", "created": 12345, "owned_by": "openai"}]
        }"#;
        let resp: oben_models::ModelListResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.object, "list");
        assert_eq!(resp.data.len(), 1);
        assert_eq!(resp.data[0].id, "gpt-4");
    }

    #[test]
    fn test_empty_message_parts() {
        let msg = ChatMessage {
            role: "user".to_string(),
            content: MessageContent::parts(vec![]),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: ChatMessage = serde_json::from_str(&json).unwrap();
        if let MessageContent::Parts(parts) = back.content {
            assert!(parts.is_empty());
        } else {
            panic!("expected Parts variant");
        }
    }

    fn resolve_provider_url(kind: &ProviderKind, base_url: Option<String>) -> String {
        match kind {
            ProviderKind::OpenAI => "https://api.openai.com/v1".to_string(),
            ProviderKind::OpenRouter => "https://openrouter.ai/api/v1".to_string(),
            ProviderKind::Anthropic => "https://api.anthropic.com/v1".to_string(),
            ProviderKind::Bedrock => "https://bedrock-runtime.us-east-1.amazonaws.com/v1".to_string(),
            ProviderKind::Gemini => "https://generativelanguage.googleapis.com/v1".to_string(),
            ProviderKind::LMStudio => "http://localhost:1234/v1".to_string(),
            // MiniMax variants use AnthropicMessagesTransport (dispatched in dispatch.rs)
            // Included here for enum exhaustiveness.
            ProviderKind::MiniMax
            | ProviderKind::MiniMaxOAuth
            | ProviderKind::MiniMaxCN => base_url.clone().unwrap_or_default(),
            // All other new providers: use base_url parameter
            ProviderKind::DeepSeek
            | ProviderKind::Alibaba
            | ProviderKind::AlibabaCodingPlan
            | ProviderKind::StepFun
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
            | ProviderKind::Custom => base_url.unwrap_or_default(),
        }
    }


    #[test]
    fn test_from_config_url_resolution() {
        assert_eq!(
            resolve_provider_url(&ProviderKind::OpenAI, None),
            "https://api.openai.com/v1"
        );
        assert_eq!(
            resolve_provider_url(&ProviderKind::OpenRouter, None),
            "https://openrouter.ai/api/v1"
        );
        assert_eq!(
            resolve_provider_url(&ProviderKind::Anthropic, None),
            "https://api.anthropic.com/v1"
        );
        assert_eq!(
            resolve_provider_url(&ProviderKind::Bedrock, None),
            "https://bedrock-runtime.us-east-1.amazonaws.com/v1"
        );
        assert_eq!(
            resolve_provider_url(&ProviderKind::Gemini, None),
            "https://generativelanguage.googleapis.com/v1"
        );
        assert_eq!(
            resolve_provider_url(&ProviderKind::LMStudio, None),
            "http://localhost:1234/v1"
        );
    }

    #[test]
    fn test_from_config_custom_provider() {
        let url = resolve_provider_url(&ProviderKind::Custom, Some("http://my.local:8080/v1".into()));
        assert_eq!(url, "http://my.local:8080/v1");
    }

    #[test]
    fn test_from_config_custom_no_url() {
        let url = resolve_provider_url(&ProviderKind::Custom, None);
        assert_eq!(url, "");
    }
}
