/// Platform abstraction for messaging services.
///
/// Each platform (Telegram, Discord, Slack) implements this trait.

use anyhow::Result;
use async_trait::async_trait;

/// A message received from a platform.
#[derive(Debug, Clone)]
pub struct IncomingMessage {
    pub platform: String,
    pub user_id: String,
    pub username: Option<String>,
    pub content: String,
    pub thread_id: Option<String>,
}

/// A message sent to a platform.
#[derive(Debug, Clone)]
pub struct OutgoingMessage {
    pub platform: String,
    pub user_id: String,
    pub thread_id: Option<String>,
    pub content: String,
}

/// Trait for messaging platform adapters.
#[async_trait]
pub trait PlatformAdapter: Send + Sync {
    /// Platform name (e.g., "telegram", "discord").
    fn name(&self) -> &str;

    /// Start listening for messages. This should block until stop() is called.
    async fn listen(&mut self, handler: Box<dyn MessageHandler>) -> Result<()>;

    /// Stop the platform adapter.
    async fn stop(&mut self);

    /// Send a message to a user.
    async fn send(&self, msg: OutgoingMessage) -> Result<()>;

    /// Check if the platform is connected and healthy.
    async fn health_check(&self) -> bool;
}

/// Handler for incoming messages from a platform.
#[async_trait]
pub trait MessageHandler: Send + Sync {
    async fn handle(&self, msg: IncomingMessage) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_incoming_message_defaults() {
        let msg = IncomingMessage {
            platform: "telegram".to_string(),
            user_id: "user-123".to_string(),
            username: None,
            content: "hello".to_string(),
            thread_id: None,
        };
        assert_eq!(msg.platform, "telegram");
        assert_eq!(msg.user_id, "user-123");
        assert_eq!(msg.username, None);
        assert_eq!(msg.content, "hello");
        assert_eq!(msg.thread_id, None);
    }

    #[test]
    fn test_incoming_message_with_all_fields() {
        let msg = IncomingMessage {
            platform: "discord".to_string(),
            user_id: "user-456".to_string(),
            username: Some("bob".to_string()),
            content: "hey there".to_string(),
            thread_id: Some("thread-789".to_string()),
        };
        assert_eq!(msg.platform, "discord");
        assert_eq!(msg.username, Some("bob".to_string()));
        assert_eq!(msg.thread_id, Some("thread-789".to_string()));
    }

    #[test]
    fn test_incoming_message_clone() {
        let msg1 = IncomingMessage {
            platform: "slack".to_string(),
            user_id: "user-1".to_string(),
            username: None,
            content: "test".to_string(),
            thread_id: None,
        };
        let msg2 = msg1.clone();
        assert_eq!(msg1.platform, msg2.platform);
        assert_eq!(msg1.user_id, msg2.user_id);
        assert_eq!(msg1.content, msg2.content);
    }

    #[test]
    fn test_outgoing_message_defaults() {
        let msg = OutgoingMessage {
            platform: "telegram".to_string(),
            user_id: "user-123".to_string(),
            thread_id: None,
            content: "hello from agent".to_string(),
        };
        assert_eq!(msg.platform, "telegram");
        assert_eq!(msg.content, "hello from agent");
    }

    #[test]
    fn test_outgoing_message_with_thread() {
        let msg = OutgoingMessage {
            platform: "slack".to_string(),
            user_id: "user-456".to_string(),
            thread_id: Some("thread-abc".to_string()),
            content: "reply in thread".to_string(),
        };
        assert_eq!(msg.platform, "slack");
        assert_eq!(msg.thread_id, Some("thread-abc".to_string()));
    }

    #[test]
    fn test_outgoing_message_clone() {
        let msg1 = OutgoingMessage {
            platform: "telegram".to_string(),
            user_id: "user-1".to_string(),
            thread_id: None,
            content: "test".to_string(),
        };
        let msg2 = msg1.clone();
        assert_eq!(msg1.platform, msg2.platform);
        assert_eq!(msg1.user_id, msg2.user_id);
        assert_eq!(msg1.content, msg2.content);
    }

    /// Mock adapter for testing purposes
    struct TestAdapter {
        name_val: String,
        send_count: std::sync::atomic::AtomicUsize,
        health: std::sync::atomic::AtomicBool,
    }

    impl TestAdapter {
        fn new(name: &str) -> Self {
            Self {
                name_val: name.to_string(),
                send_count: std::sync::atomic::AtomicUsize::new(0),
                health: std::sync::atomic::AtomicBool::new(true),
            }
        }
    }

    #[async_trait]
    impl PlatformAdapter for TestAdapter {
        fn name(&self) -> &str {
            &self.name_val
        }

        async fn listen(&mut self, _handler: Box<dyn MessageHandler>) -> Result<()> {
            unimplemented!("listen not supported in test mode")
        }

        async fn stop(&mut self) {}

        async fn send(&self, _msg: OutgoingMessage) -> Result<()> {
            self.send_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }

        async fn health_check(&self) -> bool {
            self.health.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    #[test]
    fn test_mock_adapter_name() {
        let adapter = TestAdapter::new("test-platform");
        assert_eq!(adapter.name(), "test-platform");
    }

    #[tokio::test]
    async fn test_mock_adapter_health_check() {
        let adapter = TestAdapter::new("test-platform");
        assert!(adapter.health_check().await);
    }

    #[tokio::test]
    async fn test_mock_adapter_send() {
        let adapter = TestAdapter::new("test-platform");
        let msg = OutgoingMessage {
            platform: "test".to_string(),
            user_id: "user-1".to_string(),
            thread_id: None,
            content: "test message".to_string(),
        };
        adapter.send(msg.clone()).await.unwrap();
        assert_eq!(adapter.send_count.load(std::sync::atomic::Ordering::SeqCst), 1);
        adapter.send(msg).await.unwrap();
        assert_eq!(adapter.send_count.load(std::sync::atomic::Ordering::SeqCst), 2);
    }
}
