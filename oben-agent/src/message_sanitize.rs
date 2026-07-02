/// Message sanitization — preprocessing before API calls.
///
/// Mirrors Hermes' message sanitization pipeline:
/// 1. Drop "thinking-only" assistant messages
/// 2. Merge consecutive user/system messages
use oben_models::{Message, MessageContent, MessagePart, MessageRole};

/// Run the full sanitization pipeline on a message list.
///
/// For history messages (all except the last user message), runs thinking-only
/// removal and consecutive user message merging.
/// The **last user message** is preserved as-is — no merging, no content loss.
/// This ensures the most recent user input (potentially containing images/base64
/// data URLs) is never stripped.
pub fn sanitize_messages(messages: &mut Vec<Message>) {
    drop_thinking_only_assistant(messages);

    // Find the index of the last user message.
    let last_user_idx = messages.iter().rposition(|m| m.role == MessageRole::User);

    if let Some(last_user_idx) = last_user_idx {
        // Split into history and remainder (last user message + everything after it)
        let mut history: Vec<Message> = messages.drain(..last_user_idx).collect();
        let remainder: Vec<Message> = messages.drain(..).collect();

        // Merge consecutive users in history only
        merge_consecutive_user_messages(&mut history);

        history.extend(remainder);
        *messages = history;
    } else {
        // No user messages at all — nothing to change
    }

    // Note: merge_consecutive_user_messages is also called after drop_thinking_only_assistant
    // on the history portion, since history messages may now have consecutive users.
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

/// Check whether a user message contains non-text content (images/parts with images).
fn user_message_has_image(msg: &Message) -> bool {
    match &msg.content {
        MessageContent::Image { .. } => true,
        MessageContent::Parts(parts) => {
            parts.iter().any(|p| matches!(p, MessagePart::Image { .. }))
        }
        MessageContent::Text(_) => false,
    }
}

/// Flush the pending text-based user message into the merged list.
fn flush_pending_user(merged: &mut Vec<Message>, pending_text: String) {
    merged.push(Message {
        role: MessageRole::User,
        content: MessageContent::Text(pending_text),
        id: None,
        tool_call_ids: vec![],
        tool_calls: None,
        reasoning: None,
        delegation_id: None,
    });
}

/// Merge consecutive user messages into single messages.
///
/// Merges consecutive user messages that contain only plain text into one
/// to avoid provider-specific issues with message role alternation.
///
/// User messages that contain images (MessageContent::Image or MessageContent::Parts
/// with image parts) are NOT merged — they are emitted as-is to preserve the
/// base64 data URLs. System messages are NOT merged — they are emitted separately
/// to preserve system prompt integrity.
pub fn merge_consecutive_user_messages(messages: &mut Vec<Message>) {
    if messages.len() <= 1 {
        return;
    }

    let mut merged = Vec::with_capacity(messages.len());
    let mut pending_user: Option<String> = None;

    for msg in messages.drain(..) {
        match msg.role {
            MessageRole::User => {
                if user_message_has_image(&msg) {
                    // Flush any accumulated text messages first
                    if let Some(text) = pending_user.take() {
                        flush_pending_user(&mut merged, text);
                    }
                    // Keep image message as-is
                    merged.push(msg);
                } else {
                    // Plain text user message — merge into pending
                    let text = match &msg.content {
                        MessageContent::Text(t) => t.clone(),
                        MessageContent::Parts(parts) => parts
                            .iter()
                            .filter_map(|p| match p {
                                MessagePart::Text(t) => Some(t.clone()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("\n"),
                        _ => String::new(),
                    };

                    if let Some(ref mut pending) = pending_user {
                        pending.push_str("\n\n");
                        pending.push_str(&text);
                    } else {
                        pending_user = Some(text);
                    }
                }
            }
            MessageRole::System => {
                // System messages are NOT merged — emit pending user first,
                // then emit the system message as its own entry.
                if let Some(text) = pending_user.take() {
                    flush_pending_user(&mut merged, text);
                }
                merged.push(msg);
            }
            _ => {
                // Non-user/system message — flush pending user message first
                if let Some(text) = pending_user.take() {
                    flush_pending_user(&mut merged, text);
                }
                merged.push(msg);
            }
        }
    }

    // Flush any remaining user message
    if let Some(text) = pending_user {
        flush_pending_user(&mut merged, text);
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
            reasoning: None,
        }
    }

    fn make_assistant_with_tools() -> Message {
        Message {
            role: MessageRole::Assistant,
            content: MessageContent::Text("Using tool...".to_string()),
            id: None,
            tool_call_ids: vec![],
            tool_calls: Some(vec![]),
            reasoning: None,
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
    fn test_merge_preserves_image_messages() {
        let img_msg = Message {
            role: MessageRole::User,
            content: MessageContent::Image {
                url: "data:image/png;base64,abc123".into(),
                detail: None,
            },
            id: None,
            tool_call_ids: vec![],
            tool_calls: None,
            reasoning: None,
        };

        let text_msg = make_user("Hello");

        let mut messages = vec![text_msg, img_msg];
        merge_consecutive_user_messages(&mut messages);

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].content.to_text(), "Hello");
        assert!(
            matches!(messages[1].content, MessageContent::Image { ref url, .. } if url == "data:image/png;base64,abc123")
        );
    }

    #[test]
    fn test_merge_preserves_parts_with_image() {
        let parts_msg = Message {
            role: MessageRole::User,
            content: MessageContent::Parts(vec![
                MessagePart::Text("分析下这个图片".into()),
                MessagePart::Image {
                    url: "data:image/jpg;base64,xyz".into(),
                    detail: None,
                },
            ]),
            id: None,
            tool_call_ids: vec![],
            tool_calls: None,
            reasoning: None,
        };

        let text_msg = make_user("先看这张");
        let text_msg2 = make_user("再看那张");

        // Image in the middle means text1 and text2 are NOT consecutive
        let mut messages = vec![text_msg, parts_msg, text_msg2];
        merge_consecutive_user_messages(&mut messages);

        assert_eq!(messages.len(), 3);
        // text_msg preserved as-is
        assert!(matches!(messages[0].content, MessageContent::Text(ref t) if t == "先看这张"));
        // parts_msg preserved with image intact
        assert!(
            matches!(messages[1].content, MessageContent::Parts(ref parts) if parts.len() == 2 && matches!(&parts[1], MessagePart::Image { .. }))
        );
        // text_msg2 preserved as-is
        assert!(matches!(messages[2].content, MessageContent::Text(ref t) if t == "再看那张"));
    }

    #[test]
    fn test_sanitize_preserves_latest_user_message() {
        // Mode A: latest user message is preserved intact
        let img_msg = Message {
            role: MessageRole::User,
            content: MessageContent::Image {
                url: "data:image/png;base64,abc123".into(),
                detail: None,
            },
            id: None,
            tool_call_ids: vec![],
            tool_calls: None,
            reasoning: None,
        };

        let mut messages = vec![make_user("previous"), make_assistant("hello"), img_msg];

        sanitize_messages(&mut messages);

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].content.to_text(), "previous");
        assert_eq!(messages[1].content.to_text(), "hello");
        assert!(
            matches!(messages[2].content, MessageContent::Image { ref url, .. } if url == "data:image/png;base64,abc123")
        );
    }

    #[test]
    fn test_sanitize_preserves_latest_user_parts_with_image() {
        let parts_msg = Message {
            role: MessageRole::User,
            content: MessageContent::Parts(vec![
                MessagePart::Text("分析下这个图片".into()),
                MessagePart::Image {
                    url: "data:image/png;base64,screenshot".into(),
                    detail: None,
                },
            ]),
            id: None,
            tool_call_ids: vec![],
            tool_calls: None,
            reasoning: None,
        };

        let mut messages = vec![
            make_user("prev1"),
            make_user("prev2"),
            make_assistant("hi"),
            parts_msg,
        ];

        sanitize_messages(&mut messages);

        // "prev1" and "prev2" should be merged in history
        assert_eq!(messages.len(), 3);
        assert!(
            matches!(messages[0].content, MessageContent::Text(ref t) if t.contains("prev1") && t.contains("prev2"))
        );
        assert_eq!(messages[1].content.to_text(), "hi");
        // Latest user message preserved with image content intact
        assert!(
            matches!(messages[2].content, MessageContent::Parts(ref parts) if parts.len() == 2 && matches!(&parts[1], MessagePart::Image { .. }))
        );
    }

    #[test]
    fn test_sanitize_merging_history_user_messages() {
        let mut messages = vec![make_user("hi"), make_user("there"), make_user("world")];

        sanitize_messages(&mut messages);

        // Only the last message is exempt — history should merge
        // Actually with Mode A: history = [hi, there], last = [world]
        // merge history → user("hi\n\nthere")
        // result → [user("hi\n\nthere"), user("world")]
        assert_eq!(messages.len(), 2);
        assert!(
            matches!(messages[0].content, MessageContent::Text(ref t) if t.contains("hi") && t.contains("there"))
        );
        assert_eq!(messages[1].content.to_text(), "world");
    }

    #[test]
    fn test_merge_image_before_text_messages() {
        let img_msg = Message {
            role: MessageRole::User,
            content: MessageContent::Image {
                url: "data:image/png;base64,abc".into(),
                detail: None,
            },
            id: None,
            tool_call_ids: vec![],
            tool_calls: None,
            reasoning: None,
        };

        let text_msg1 = make_user("先");
        let text_msg2 = make_user("后");

        let mut messages = vec![img_msg, text_msg1, text_msg2];
        merge_consecutive_user_messages(&mut messages);

        assert_eq!(messages.len(), 2);
        // Image should be first
        assert!(matches!(messages[0].content, MessageContent::Image { .. }));
        // text_msg1 and text_msg2 should be merged
        assert!(
            matches!(messages[1].content, MessageContent::Text(ref t) if t.contains("先") && t.contains("后"))
        );
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
