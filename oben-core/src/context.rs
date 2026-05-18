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
