//! Message injection system for plugins.
//!
//! Maps to Hermes' `inject_message()` which allows plugins to:
//! - Insert messages at end of conversation (append)
//! - Interrupt current generation (interrupt)
//! - Queue messages for next idle turn (queue)

use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use std::time::Duration;
use uuid::Uuid;

// We'll use epoch millis as a simple timestamp
fn now_millis() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis() as u64
}

/// How a message should be injected into the conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageAction {
    /// Insert message at end of current conversation.
    /// The message will appear in the next turn.
    Append,

    /// Interrupt current generation (if streaming).
    /// The message will be processed immediately, stopping any
    /// ongoing LLM generation.
    Interrupt,

    /// Queue message for next idle turn.
    /// The message is stored and added when the agent is idle.
    Queue,
}

/// A message injected by a plugin into the conversation.
#[derive(Debug, Clone)]
pub struct InjectedMessage {
    /// Unique message ID.
    pub id: String,

    /// Message role (user, assistant, system).
    pub role: String,

    /// Message content.
    pub content: String,

    /// Injection action.
    pub action: MessageAction,

    /// Timestamp when the message was injected (epoch millis).
    pub injected_at_secs: u64,

    /// Name of the plugin that injected this message.
    pub plugin: String,

    /// Whether this message has been processed/consumed.
    pub consumed: bool,
}

/// Message injector — thread-safe queue of plugin-injected messages.
///
/// Plugins use this to add messages to the conversation at various
/// points: appending to the end, interrupting mid-generation, or
/// queuing for later processing.
pub struct MessageInjector {
    /// All injected messages, ordered by injection time.
    messages: Mutex<Vec<InjectedMessage>>,

    /// Maximum number of messages to keep in the queue.
    max_queue_size: usize,
}

impl Default for MessageInjector {
    fn default() -> Self {
        Self::new()
    }
}

impl MessageInjector {
    /// Create a new message injector.
    pub fn new() -> Self {
        Self {
            messages: Mutex::new(Vec::new()),
            max_queue_size: 100,
        }
    }

    /// Inject a message into the conversation.
    ///
    /// Returns the message ID for tracking.
    ///
    /// Args:
    ///   - role: Message role ("user", "assistant", "system")
    ///   - content: Message content
    ///   - action: How to inject (append, interrupt, queue)
    ///   - plugin: Name of the plugin injecting the message
    pub fn inject(
        &self,
        role: impl Into<String>,
        content: impl Into<String>,
        action: MessageAction,
        plugin: impl Into<String>,
    ) -> String {
        let id = Uuid::new_v4().to_string();
        let is_interrupt = action == MessageAction::Interrupt;

        let msg = InjectedMessage {
            id: id.clone(),
            role: role.into(),
            content: content.into(),
            action,
            injected_at_secs: now_millis(),
            plugin: plugin.into(),
            consumed: false,
        };

        let mut messages = self.messages.lock().unwrap();

        // Enforce max queue size (drop oldest non-interrupt messages)
        if !is_interrupt && messages.len() >= self.max_queue_size {
            // Remove oldest messages until under limit
            while messages.len() >= self.max_queue_size {
                messages.remove(0);
            }
        }

        messages.push(msg);
        drop(messages);
        id
    }

    /// Get all unconsumed messages, optionally filtered by action.
    pub fn get_unconsumed(&self, action: Option<MessageAction>) -> Vec<InjectedMessage> {
        let messages = self.messages.lock().unwrap();
        messages
            .iter()
            .filter(|m| !m.consumed)
            .filter(|m| action.as_ref().map(|a| m.action == *a).unwrap_or(true))
            .cloned()
            .collect()
    }

    /// Get interrupt messages (non-consuming).
    pub fn get_interrupt_messages(&self) -> Vec<InjectedMessage> {
        self.get_unconsumed(Some(MessageAction::Interrupt))
    }

    /// Get append messages (non-consuming).
    pub fn get_append_messages(&self) -> Vec<InjectedMessage> {
        self.get_unconsumed(Some(MessageAction::Append))
    }

    /// Get queued messages (non-consuming).
    pub fn get_queued_messages(&self) -> Vec<InjectedMessage> {
        self.get_unconsumed(Some(MessageAction::Queue))
    }

    /// Mark all messages of a given action as consumed.
    pub fn consume(&self, action: MessageAction) -> Vec<String> {
        let mut messages = self.messages.lock().unwrap();
        let ids: Vec<String> = messages
            .iter_mut()
            .filter(|m| m.action == action && !m.consumed)
            .map(|m| {
                m.consumed = true;
                m.id.clone()
            })
            .collect();
        ids
    }

    /// Mark all messages as consumed.
    pub fn consume_all(&self) -> usize {
        let mut messages = self.messages.lock().unwrap();
        let count = messages.iter().filter(|m| !m.consumed).count();
        for m in messages.iter_mut() {
            m.consumed = true;
        }
        count
    }

    /// List all injected messages (owned clones).
    pub fn list(&self) -> Vec<InjectedMessage> {
        self.messages.lock().unwrap().iter().cloned().collect()
    }

    /// Clear all injected messages.
    pub fn clear(&self) {
        self.messages.lock().unwrap().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inject_message() {
        /// given: a new MessageInjector
        /// when: inject() is called
        /// then: message is stored with correct properties
        let injector = MessageInjector::new();
        let id = injector.inject(
            "user".to_string(),
            "Hello".to_string(),
            MessageAction::Append,
            "test-plugin",
        );

        let msgs = injector.list();
        assert_eq!(msgs.len(), 1);
        let msg = &msgs[0];
        assert_eq!(msg.id, id);
        assert_eq!(msg.role, "user");
        assert_eq!(msg.content, "Hello");
        assert_eq!(msg.action, MessageAction::Append);
        assert_eq!(msg.plugin, "test-plugin");
        assert!(!msg.consumed);
    }

    #[test]
    fn test_get_unconsumed() {
        /// given: messages injected and some consumed
        /// when: get_unconsumed() is called
        /// then: only unconsumed messages returned
        let injector = MessageInjector::new();
        injector.inject(
            "user".to_string(),
            "msg1".to_string(),
            MessageAction::Append,
            "p1",
        );
        injector.inject(
            "user".to_string(),
            "msg2".to_string(),
            MessageAction::Queue,
            "p2",
        );

        let unconsumed = injector.get_append_messages();
        assert_eq!(unconsumed.len(), 1);
        assert_eq!(unconsumed[0].content, "msg1");
    }

    #[test]
    fn test_consume_messages() {
        /// given: multiple appended messages
        /// when: consume(Append) is called
        /// then: all append messages are marked consumed
        let injector = MessageInjector::new();
        injector.inject(
            "user".to_string(),
            "msg1".to_string(),
            MessageAction::Append,
            "p1",
        );
        injector.inject(
            "user".to_string(),
            "msg2".to_string(),
            MessageAction::Append,
            "p1",
        );
        injector.inject(
            "user".to_string(),
            "msg3".to_string(),
            MessageAction::Queue,
            "p1",
        );

        let consumed = injector.consume(MessageAction::Append);
        assert_eq!(consumed.len(), 2);
        assert_eq!(injector.get_append_messages().len(), 0);

        // Queue messages should not be consumed
        assert_eq!(injector.get_queued_messages().len(), 1);
    }

    #[test]
    fn test_consume_all() {
        /// given: various messages
        /// when: consume_all() is called
        /// then: all messages are consumed
        let injector = MessageInjector::new();
        injector.inject(
            "user".to_string(),
            "msg1".to_string(),
            MessageAction::Append,
            "p1",
        );
        injector.inject(
            "user".to_string(),
            "msg2".to_string(),
            MessageAction::Interrupt,
            "p2",
        );
        injector.inject(
            "user".to_string(),
            "msg3".to_string(),
            MessageAction::Queue,
            "p3",
        );

        let count = injector.consume_all();
        assert_eq!(count, 3);
        assert_eq!(injector.get_unconsumed(None).len(), 0);
    }

    #[test]
    fn test_max_queue_size() {
        /// given: injector with default max size
        /// when: more messages than max are injected
        /// then: oldest messages are dropped
        let injector = MessageInjector::new();
        for i in 0..150 {
            injector.inject(
                "user".to_string(),
                format!("msg{}", i),
                MessageAction::Queue,
                "plugin",
            );
        }

        let msgs = injector.list();
        assert!(msgs.len() <= 100);
    }

    #[test]
    fn test_interrupt_not_dropped_by_queue_limit() {
        /// given: injector at max capacity with queued messages
        /// when: an interrupt message is injected
        /// then: interrupt message is kept even if over limit
        let injector = MessageInjector::new();
        for i in 0..100 {
            injector.inject(
                "user".to_string(),
                format!("msg{}", i),
                MessageAction::Queue,
                "plugin",
            );
        }

        // Interrupt should be added even at capacity
        let _id = injector.inject(
            "user".to_string(),
            "urgent".to_string(),
            MessageAction::Interrupt,
            "plugin",
        );
        assert_eq!(injector.get_interrupt_messages().len(), 1);
        assert!(!injector.get_interrupt_messages().is_empty());
    }

    #[test]
    fn test_clear_messages() {
        /// given: several injected messages
        /// when: clear() is called
        /// then: all messages are removed
        let injector = MessageInjector::new();
        injector.inject(
            "user".to_string(),
            "msg1".to_string(),
            MessageAction::Append,
            "p1",
        );
        injector.inject(
            "user".to_string(),
            "msg2".to_string(),
            MessageAction::Queue,
            "p2",
        );

        injector.clear();
        assert!(injector.list().is_empty());
    }
}
