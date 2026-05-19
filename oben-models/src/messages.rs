use serde::{Deserialize, Serialize};

use super::ToolCall;

/// A single message in a conversation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: MessageRole,
    pub content: MessageContent,
    pub id: Option<String>,
    /// Tool call IDs this message references (for tool results).
    pub tool_call_ids: Vec<String>,
    /// Tool calls made by the assistant in this message.
    pub tool_calls: Option<Vec<ToolCall>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MessageContent {
    Text(String),
    Image { url: String, detail: Option<String> },
    /// Multiple parts (text + image) in one message.
    Parts(Vec<MessagePart>),
}

impl MessageContent {
    /// Get the text content if this is a Text variant.
    pub fn to_text(&self) -> String {
        match self {
            MessageContent::Text(t) => t.clone(),
            _ => String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessagePart {
    Text(String),
    Image { url: String, detail: Option<String> },
}

impl Message {
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: MessageRole::User,
            content: MessageContent::Text(text.into()),
            id: None,
            tool_call_ids: vec![],
            tool_calls: None,
        }
    }

    pub fn assistant(text: impl Into<String>) -> Self {
        Self {
            role: MessageRole::Assistant,
            content: MessageContent::Text(text.into()),
            id: None,
            tool_call_ids: vec![],
            tool_calls: None,
        }
    }

    pub fn assistant_tool_calls(tool_calls: Vec<ToolCall>) -> Self {
        Self {
            role: MessageRole::Assistant,
            content: MessageContent::Text(String::new()),
            id: None,
            tool_call_ids: vec![],
            tool_calls: Some(tool_calls),
        }
    }

    pub fn system(text: impl Into<String>) -> Self {
        Self {
            role: MessageRole::System,
            content: MessageContent::Text(text.into()),
            id: None,
            tool_call_ids: vec![],
            tool_calls: None,
        }
    }

    pub fn tool_result(tool_call_id: impl Into<String>, output: impl Into<String>) -> Self {
        Self {
            role: MessageRole::Tool,
            content: MessageContent::Text(output.into()),
            id: None,
            tool_call_ids: vec![tool_call_id.into()],
            tool_calls: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json;

    #[test]
    fn test_message_roundtrip_json() {
        let msg = Message::user("hello world");
        let json = serde_json::to_string(&msg).unwrap();
        let back: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(back, msg);
    }

    #[test]
    fn test_assistant_message_roundtrip_json() {
        let msg = Message::assistant("i can help with that");
        let json = serde_json::to_string(&msg).unwrap();
        let back: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(back, msg);
    }

    #[test]
    fn test_system_message_roundtrip_json() {
        let msg = Message::system("you are a helpful assistant");
        let json = serde_json::to_string(&msg).unwrap();
        let back: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(back, msg);
    }

    #[test]
    fn test_tool_result_message_roundtrip_json() {
        let msg = Message::tool_result("call-123", "output data");
        let json = serde_json::to_string(&msg).unwrap();
        let back: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(back, msg);
    }
}
