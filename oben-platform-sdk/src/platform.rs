/// Platform abstraction for messaging services.
///
/// Each platform (Telegram, Discord, Slack, QQ) implements this trait.
use anyhow::Result;
use async_trait::async_trait;
use serde::{Serialize, Deserialize};
use std::fmt::Display;

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

/// Status of a messaging platform adapter.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PlatformStatus {
    #[default]
    /// Platform has not been started yet.
    Idle,
    /// Platform is in the process of connecting.
    Connecting,
    /// Platform is actively running and receiving messages.
    Running,
    /// Platform has failed with the given error message.
    Failed(String),
}

impl Display for PlatformStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlatformStatus::Idle => write!(f, "Idle"),
            PlatformStatus::Connecting => write!(f, "Connecting"),
            PlatformStatus::Running => write!(f, "Running"),
            PlatformStatus::Failed(reason) => write!(f, "Failed({})", reason),
        }
    }
}

/// Metadata about a platform adapter's current state.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlatformInfo {
    /// Unique platform name (e.g., "qq_bot", "telegram").
    pub name: String,
    /// Current status of the platform.
    pub status: PlatformStatus,
    /// ISO-8601 timestamp when the platform was last started.
    pub started_at: Option<String>,
    /// Optional error message if status is Failed.
    pub error: Option<String>,
}

/// Trait for messaging platform adapters.
#[async_trait]
pub trait PlatformAdapter: Send + Sync {
    /// Platform name (e.g., "telegram", "discord").
    fn name(&self) -> &str;

    /// Start listening for messages. This should block until stop() is called.
    async fn listen(&mut self) -> Result<()>;

    /// Stop the platform adapter.
    async fn stop(&mut self);

    /// Send a message to a user.
    async fn send(&self, msg: OutgoingMessage) -> Result<()>;

    /// Check if the platform is connected and healthy.
    async fn health_check(&self) -> bool;
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

    /// Given: A platform status with Idle variant
    /// When: Display is called via to_string()
    /// Then: Returns "Idle"
    #[test]
    fn test_platform_status_idle() {
        assert_eq!(PlatformStatus::Idle.to_string(), "Idle");
    }

    /// Given: A platform status with Connecting variant
    /// When: Display is called via to_string()
    /// Then: Returns "Connecting"
    #[test]
    fn test_platform_status_connecting() {
        assert_eq!(PlatformStatus::Connecting.to_string(), "Connecting");
    }

    /// Given: A platform status with Running variant
    /// When: Display is called via to_string()
    /// Then: Returns "Running"
    #[test]
    fn test_platform_status_running() {
        assert_eq!(PlatformStatus::Running.to_string(), "Running");
    }

    /// Given: A platform status with Failed variant containing "bad_token"
    /// When: Display is called via to_string()
    /// Then: Returns "Failed(bad_token)"
    #[test]
    fn test_platform_status_failed() {
        assert_eq!(
            PlatformStatus::Failed("bad_token".to_string()).to_string(),
            "Failed(bad_token)"
        );
    }

    /// Given: A PlatformInfo struct constructed with Debug trait
    /// When: It is cloned via Clone derive
    /// Then: The clone matches the original and both implement Debug formatting
    #[test]
    fn test_platform_info_debug_clone() {
        let info = PlatformInfo {
            name: "telegram".to_string(),
            status: PlatformStatus::Running,
            started_at: Some("2025-01-01T00:00:00".to_string()),
            error: None,
        };
        let cloned = info.clone();
        assert_eq!(info.name, cloned.name);
        assert_eq!(info.status, cloned.status);
        assert_eq!(info.started_at, cloned.started_at);
        assert_eq!(info.error, cloned.error);
        assert!(format!("{:?}", info).contains("telegram"));
    }

    /// Given: A PlatformInfo in Default state (no name, Idle, no started_at, no error)
    /// When: PlatformInfo::default() is called
    /// Then: All fields are empty/default values
    #[test]
    fn test_platform_info_default() {
        let info = PlatformInfo::default();
        assert_eq!(info.name, "");
        assert_eq!(info.status, PlatformStatus::Idle);
        assert_eq!(info.started_at, None);
        assert_eq!(info.error, None);
    }
}
