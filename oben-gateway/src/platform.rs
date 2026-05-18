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
