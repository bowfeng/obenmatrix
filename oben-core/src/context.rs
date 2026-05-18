/// Context window management — tracking message count, token estimation.

use oben_models::Message;

pub struct ContextManager {
    messages: Vec<Message>,
    max_messages: usize,
    compression_enabled: bool,
}

impl ContextManager {
    pub fn new(max_messages: usize) -> Self {
        Self {
            messages: Vec::new(),
            max_messages,
            compression_enabled: true,
        }
    }

    pub fn add_message(&mut self, msg: Message) {
        self.messages.push(msg);
    }

    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    /// Check if context is getting full and needs compression.
    pub fn needs_compression(&self) -> bool {
        self.compression_enabled && self.messages.len() > self.max_messages
    }

    /// Get the number of messages.
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// Estimate token count (rough heuristic: ~4 chars per token).
    pub fn estimate_tokens(&self) -> usize {
        self.messages.iter().map(|m| token_estimate(&m)).sum()
    }

    /// Clear all messages.
    pub fn clear_messages(&mut self) {
        self.messages.clear();
    }
}

/// Rough token estimation: ~4 chars per token.
fn token_estimate(msg: &Message) -> usize {
    let text = match &msg.content {
        oben_models::MessageContent::Text(s) => s,
        oben_models::MessageContent::Image { .. } => return 500,
        oben_models::MessageContent::Parts(parts) => {
            return parts.iter().map(|p| match p {
                oben_models::MessagePart::Text(s) => s.len() / 4,
                oben_models::MessagePart::Image { .. } => 500,
            }).sum();
        }
    };
    text.len() / 4 + 5 // per-message overhead
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_starts_empty() {
        let ctx = ContextManager::new(100);
        assert!(ctx.is_empty());
        assert_eq!(ctx.len(), 0);
    }

    #[test]
    fn test_add_message_increments_count() {
        let mut ctx = ContextManager::new(100);
        ctx.add_message(Message::user("hello"));
        assert_eq!(ctx.len(), 1);
        assert!(!ctx.is_empty());
    }

    #[test]
    fn test_needs_compression_false_under_limit() {
        let ctx = ContextManager::new(100);
        assert!(!ctx.needs_compression());
    }

    #[test]
    fn test_needs_compression_true_over_limit() {
        let mut ctx = ContextManager::new(2);
        ctx.add_message(Message::user("a"));
        ctx.add_message(Message::user("b"));
        assert!(!ctx.needs_compression()); // exactly at limit
        ctx.add_message(Message::user("c"));
        assert!(ctx.needs_compression()); // over limit
    }

    #[test]
    fn test_clear_messages() {
        let mut ctx = ContextManager::new(100);
        ctx.add_message(Message::user("a"));
        ctx.add_message(Message::user("b"));
        ctx.clear_messages();
        assert!(ctx.is_empty());
        assert_eq!(ctx.len(), 0);
        assert!(!ctx.needs_compression());
    }

    #[test]
    fn test_estimate_tokens_text() {
        let mut ctx = ContextManager::new(100);
        ctx.add_message(Message::user("a".repeat(400))); // 400 chars ~ 105 tokens
        let tokens = ctx.estimate_tokens();
        // Should be 100 (text) + 5 (overhead)
        assert_eq!(tokens, 105);
    }

    #[test]
    fn test_estimate_tokens_image() {
        let mut ctx = ContextManager::new(100);
        ctx.add_message(Message {
            role: oben_models::MessageRole::User,
            content: oben_models::MessageContent::Image {
                url: "https://example.com/img.jpg".to_string(),
                detail: None,
            },
            id: None,
            tool_call_ids: vec![],
        });
        assert_eq!(ctx.estimate_tokens(), 500);
    }

    #[test]
    fn test_messages_returns_reference() {
        let mut ctx = ContextManager::new(100);
        ctx.add_message(Message::user("test"));
        let msgs = ctx.messages();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, oben_models::MessageRole::User);
    }
}
