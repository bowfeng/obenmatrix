//! Topic-based messaging channels between agents
//!
//! Provides publish/subscribe pattern with support for:
//! - Publishing messages to named topics
//! - Subscribing to topics with async receivers
//! - Broadcasting to all listeners

use dashmap::DashMap;
use std::collections::HashSet;
use tokio::sync::mpsc;

/// Channel for topic-based messaging between agents
pub struct Channel {
    subscribers: DashMap<String, Vec<mpsc::Sender<String>>>,
}

impl Channel {
    /// Create a new empty channel
    pub fn new() -> Self {
        Channel {
            subscribers: DashMap::new(),
        }
    }

    /// Publish a message to a specific topic
    pub fn publish(&self, topic: &str, message: String) {
        if let Some(subs) = self.subscribers.get(topic) {
            let senders: Vec<_> = subs.iter().map(|s| s.clone()).collect();
            for sender in senders {
                // Non-blocking send, drop message if receiver full
                sender.try_send(message.clone()).ok();
            }
        }
    }

    /// Subscribe to a topic, returns a receiver for incoming messages
    pub fn subscribe(&self, topic: &str) -> mpsc::Receiver<String> {
        let (tx, rx) = mpsc::channel::<String>(32);
        self.subscribers
            .entry(topic.to_string())
            .or_insert_with(Vec::new)
            .push(tx);
        rx
    }

    /// Broadcast a message to all agents (all subscribers)
    pub fn broadcast(&self, message: String) {
        let all_senders: Vec<mpsc::Sender<String>> = self
            .subscribers
            .iter()
            .flat_map(|entry| entry.value().clone())
            .collect();

        for sender in all_senders {
            sender.try_send(message.clone()).ok();
        }
    }

    /// Get list of active topics
    pub fn topics(&self) -> HashSet<String> {
        self.subscribers
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }

    /// Get number of subscribers for a topic
    pub fn subscriber_count(&self, topic: &str) -> usize {
        self.subscribers
            .get(topic)
            .map(|entry| entry.len())
            .unwrap_or(0)
    }
}

impl Default for Channel {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_channel_creation() {
        let channel = Channel::new();
        assert_eq!(channel.subscribers.len(), 0);
        assert_eq!(channel.topics().len(), 0);

        let default_channel = Channel::default();
        assert_eq!(default_channel.subscribers.len(), 0);
    }

    #[test]
    fn test_subscribe_initial_state() {
        let channel = Channel::new();
        let receiver = channel.subscribe("test-topic");

        assert_eq!(channel.topics().len(), 1);
        assert!(channel.topics().contains(&"test-topic".to_string()));
    }

    #[test]
    fn test_publish_to_single_subscriber() {
        let channel = Channel::new();
        let mut receiver = channel.subscribe("chat-topic");

        channel.publish("chat-topic", "hello".to_string());

        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let msg = receiver.recv().await;
            assert_eq!(msg, Some("hello".to_string()));
        });
    }

    #[test]
    fn test_publish_to_multiple_subscribers() {
        let channel = Channel::new();
        let mut receiver1 = channel.subscribe("broadcast-topic");
        let mut receiver2 = channel.subscribe("broadcast-topic");

        channel.publish("broadcast-topic", "message".to_string());

        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let msg1 = receiver1.recv().await;
            let msg2 = receiver2.recv().await;
            assert_eq!(msg1, Some("message".to_string()));
            assert_eq!(msg2, Some("message".to_string()));
        });
    }

    #[test]
    fn test_broadcast_all_subscribers() {
        let channel = Channel::new();
        let mut receiver1 = channel.subscribe("topic-a");
        let mut receiver2 = channel.subscribe("topic-b");

        channel.broadcast("global-msg".to_string());

        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let msg1 = receiver1.recv().await;
            let msg2 = receiver2.recv().await;
            assert_eq!(msg1, Some("global-msg".to_string()));
            assert_eq!(msg2, Some("global-msg".to_string()));
        });
    }

    #[test]
    fn test_publish_to_nonexistent_topic() {
        let channel = Channel::new();
        
        channel.publish("nonexistent", "test".to_string());
        
        assert_eq!(channel.topics().len(), 0);
    }

    #[test]
    fn test_subscriber_count() {
        let channel = Channel::new();
        
        channel.subscribe("topic-a");
        channel.subscribe("topic-a");
        
        channel.subscribe("topic-b");

        assert_eq!(channel.subscriber_count("topic-a"), 2);
        assert_eq!(channel.subscriber_count("topic-b"), 1);
        assert_eq!(channel.subscriber_count("nonexistent"), 0);
    }

    #[test]
    fn test_topics_list() {
        let channel = Channel::new();
        channel.subscribe("active");
        
        assert_eq!(channel.topics().len(), 1);
        assert!(channel.topics().contains(&"active".to_string()));
    }
}
