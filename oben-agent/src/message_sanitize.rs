/// Message sanitization — preprocessing before API calls.
///
/// Mirrors Hermes' message sanitization pipeline:
/// 1. Drop "thinking-only" assistant messages
/// 2. Merge consecutive user/system messages
use oben_models::{Message, MessageContent, MessagePart, MessageRole};

/// Run the full sanitization pipeline on a message list.
pub fn sanitize_messages(messages: &mut Vec<Message>) {
    drop_thinking_only_assistant(messages);
    merge_consecutive_user_messages(messages);
}

/// Drop assistant messages that are "thinking-only" — they have reasoning
/// (empty content) but no visible text and no tool calls.
///
/// These cause API errors on providers that convert reasoning into thinking blocks.
pub fn drop_thinking_only_assistant(messages: &mut Vec<Message>) {
    messages.retain(|msg| !is_thinking_only_assistant(msg));
}

/// Check if a message is a "thinking-only" assistant message.
///
/// An assistant message is thinking-only when:
/// - Role is Assistant
/// - No tool calls
/// - Content is empty or whitespace only
pub fn is_thinking_only_assistant(msg: &Message) -> bool {
    if msg.role != MessageRole::Assistant {
        return false;
    }

    // Must have no tool calls
    if msg.tool_calls.as_ref().map_or(false, |tc| !tc.is_empty()) {
        return false;
    }

    // Check if content is empty or only whitespace
    match &msg.content {
        MessageContent::Text(text) => text.trim().is_empty(),
        MessageContent::Parts(parts) => {
            parts.iter().all(|p| {
                match p {
                    MessagePart::Text(t) => t.trim().is_empty(),
                    _ => false, // non-text parts mean it's not thinking-only
                }
            })
        }
        MessageContent::Image { .. } => false,
    }
}

/// Merge consecutive user messages into single messages.
///
/// Merges consecutive user/developer messages into one to avoid provider-specific
/// issues with message role alternation. System messages are NOT merged — they
/// are emitted separately to preserve system prompt integrity.
pub fn merge_consecutive_user_messages(messages: &mut Vec<Message>) {
    if messages.len() <= 1 {
        return;
    }

    let mut merged = Vec::with_capacity(messages.len());
    let mut pending_user: Option<String> = None;

    for msg in messages.drain(..) {
        match msg.role {
            MessageRole::User => {
                let text = match &msg.content {
                    MessageContent::Text(t) => t.clone(),
                    MessageContent::Parts(parts) => parts
                        .iter()
                        .filter_map(|p| {
                            if let MessagePart::Text(t) = p {
                                Some(t.clone())
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                    MessageContent::Image { .. } => "[image]".to_string(),
                };

                if let Some(ref mut pending) = pending_user {
                    pending.push_str("\n\n");
                    pending.push_str(&text);
                } else {
                    pending_user = Some(text);
                }
            }
            MessageRole::System => {
                // System messages are NOT merged — emit pending user first,
                // then emit the system message as its own entry.
                if let Some(text) = pending_user.take() {
                    merged.push(Message::user(text));
                }
                merged.push(msg);
            }
            _ => {
                // Non-user/system message — flush pending user message first
                if let Some(text) = pending_user.take() {
                    merged.push(Message::user(text));
                }
                merged.push(msg);
            }
        }
    }

    // Flush any remaining user message
    if let Some(text) = pending_user {
        merged.push(Message::user(text));
    }

    *messages = merged;
}

/// Strip surrogate characters from a string.
pub fn strip_surrogates(text: &str) -> String {
    text.chars()
        .filter(|c| {
            let code = *c as u32;
            !(0xD800..=0xDFFF).contains(&code)
        })
        .collect()
}

/// Strip non-ASCII characters from a string.
pub fn strip_non_ascii(text: &str) -> String {
    text.chars().filter(|c| c.is_ascii()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_assistant(text: &str) -> Message {
        Message {
            role: MessageRole::Assistant,
            content: MessageContent::Text(text.to_string()),
            id: None,
            tool_call_ids: vec![],
            tool_calls: None,
        }
    }

    fn make_assistant_with_tools() -> Message {
        Message {
            role: MessageRole::Assistant,
            content: MessageContent::Text("Using tool...".to_string()),
            id: None,
            tool_call_ids: vec![],
            tool_calls: Some(vec![]),
        }
    }

    fn make_user(text: &str) -> Message {
        Message::user(text)
    }

    fn make_system(text: &str) -> Message {
        Message::system(text)
    }

    #[test]
    fn test_is_thinking_only_assistant_empty() {
        let msg = make_assistant("");
        assert!(is_thinking_only_assistant(&msg));
    }

    #[test]
    fn test_is_thinking_only_assistant_whitespace() {
        let msg = make_assistant("   \n  ");
        assert!(is_thinking_only_assistant(&msg));
    }

    #[test]
    fn test_is_thinking_only_assistant_with_text() {
        let msg = make_assistant("Hello, how can I help?");
        assert!(!is_thinking_only_assistant(&msg));
    }

    #[test]
    fn test_is_thinking_only_assistant_with_tools() {
        let msg = make_assistant_with_tools();
        assert!(!is_thinking_only_assistant(&msg));
    }

    #[test]
    fn test_drop_thinking_only_messages() {
        let mut messages = vec![
            make_user("Hello"),
            make_assistant(""),         // thinking-only, drop
            make_assistant("Response"), // keep
            make_user("Follow up"),
        ];

        drop_thinking_only_assistant(&mut messages);

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].role, MessageRole::User);
        assert_eq!(messages[1].role, MessageRole::Assistant);
        assert_eq!(messages[1].content.to_text(), "Response");
        assert_eq!(messages[2].role, MessageRole::User);
    }

    #[test]
    fn test_merge_consecutive_user_messages() {
        let mut messages = vec![
            make_user("First"),
            make_user("Second"),
            make_user("Third"),
            make_system("System prompt"),
            make_user("After system"),
        ];

        merge_consecutive_user_messages(&mut messages);

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].role, MessageRole::User);
        let combined = messages[0].content.to_text();
        assert!(combined.contains("First"));
        assert!(combined.contains("Second"));
        assert!(combined.contains("Third"));
        assert_eq!(messages[1].role, MessageRole::System);
        assert_eq!(messages[2].role, MessageRole::User);
    }

    #[test]
    fn test_strip_surrogates() {
        let input = "Hello, world!";
        let output = strip_surrogates(input);
        assert_eq!(output, "Hello, world!");
    }

    #[test]
    fn test_strip_non_ascii() {
        let input = "Hello 世界 🌍";
        let output = strip_non_ascii(input);
        // Only ASCII chars kept: "Hello " + space before 世 (kept) + space before 🌍 (kept)
        assert_eq!(output, "Hello  ");
    }

    #[test]
    fn test_no_merge_when_alternating() {
        let mut messages = vec![
            make_user("User 1"),
            make_assistant("Assistant 1"),
            make_user("User 2"),
        ];

        merge_consecutive_user_messages(&mut messages);

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].content.to_text(), "User 1");
        assert_eq!(messages[2].content.to_text(), "User 2");
    }

    #[test]
    fn test_merge_preserves_order() {
        let mut messages = vec![
            make_user("A"),
            make_user("B"),
            make_system("S"),
            make_user("C"),
            make_user("D"),
        ];

        merge_consecutive_user_messages(&mut messages);

        assert_eq!(messages.len(), 3);
        assert!(messages[0].content.to_text().contains("A"));
        assert!(messages[0].content.to_text().contains("B"));
        assert_eq!(messages[1].role, MessageRole::System);
        assert!(messages[2].content.to_text().contains("C"));
        assert!(messages[2].content.to_text().contains("D"));
    }

    #[test]
    fn test_sanitize_messages_runs_full_pipeline() {
        let mut messages = vec![
            make_user("Hello"),
            make_assistant(""), // thinking-only
            make_assistant("Response"),
            make_user("Follow up"),
        ];

        sanitize_messages(&mut messages);

        // Thinking-only dropped, others preserved
        assert_eq!(messages.len(), 3);
    }
}
