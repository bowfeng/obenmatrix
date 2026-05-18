/// Gateway — manages platform connections and routes messages to the agent.
/// Maps to `gateway/gateway.py` in Hermes.

use anyhow::Result;
use oben_memory::MemoryManager;
use oben_tools::ToolRegistry;
use tracing::{info, warn};

use crate::platform::{IncomingMessage, MessageHandler, OutgoingMessage, PlatformAdapter};

/// The gateway process — listens on multiple platforms and routes to the agent.
pub struct Gateway {
    memory: MemoryManager,
    tools: ToolRegistry,
}

impl Gateway {
    pub fn new(memory: MemoryManager, tools: ToolRegistry) -> Self {
        Self { memory, tools }
    }

    /// Handle an incoming message (simplified — no platform-specific routing).
    pub async fn handle_message(&self, msg: IncomingMessage) -> Result<String> {
        let preview = if msg.content.len() > 30 {
            &msg.content[..30]
        } else {
            &msg.content
        };
        info!("Gateway received {} message from {} ({})", msg.platform, msg.user_id, preview);

        // TODO: Route through conversation loop
        // For now, just echo
        Ok(format!("Echo: {}", msg.content))
    }

    /// Start the gateway (placeholder — full platform support TBD).
    pub async fn start(&self) -> Result<()> {
        info!("Gateway started. Platform adapters TBD.");
        // In production: spawn platform listeners via tokio::spawn
        loop {
            // Keep running
            tokio::time::sleep(std::time::Duration::from_secs(86400)).await;
        }
    }

    pub fn memory(&self) -> &MemoryManager {
        &self.memory
    }

    pub fn tools(&self) -> &ToolRegistry {
        &self.tools
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::*;

    #[tokio::test]
    async fn test_handle_message_echo() {
        let gateway = Gateway::new(MemoryManager::new(), oben_tools::ToolRegistry::new());
        let msg = IncomingMessage {
            platform: "telegram".to_string(),
            user_id: "user-1".to_string(),
            username: Some("alice".to_string()),
            content: "hello there".to_string(),
            thread_id: None,
        };
        let result = gateway.handle_message(msg).await.unwrap();
        assert_eq!(result, "Echo: hello there");
    }

    #[tokio::test]
    async fn test_handle_message_short_content() {
        let gateway = Gateway::new(MemoryManager::new(), oben_tools::ToolRegistry::new());
        let msg = IncomingMessage {
            platform: "discord".to_string(),
            user_id: "user-2".to_string(),
            username: None,
            content: "hi".to_string(),
            thread_id: Some("thread-1".to_string()),
        };
        let result = gateway.handle_message(msg).await.unwrap();
        assert_eq!(result, "Echo: hi");
    }

    #[tokio::test]
    async fn test_handle_message_long_content_preview() {
        let gateway = Gateway::new(MemoryManager::new(), oben_tools::ToolRegistry::new());
        let long_content = "a".repeat(50);
        let msg = IncomingMessage {
            platform: "slack".to_string(),
            user_id: "user-3".to_string(),
            username: None,
            content: long_content.clone(),
            thread_id: None,
        };
        // Should not panic and should handle long content gracefully
        let result = gateway.handle_message(msg).await.unwrap();
        assert_eq!(result, format!("Echo: {}", long_content));
    }

    #[tokio::test]
    async fn test_handle_message_empty_content() {
        let gateway = Gateway::new(MemoryManager::new(), oben_tools::ToolRegistry::new());
        let msg = IncomingMessage {
            platform: "test".to_string(),
            user_id: "user-0".to_string(),
            username: None,
            content: "".to_string(),
            thread_id: None,
        };
        let result = gateway.handle_message(msg).await.unwrap();
        assert_eq!(result, "Echo: ");
    }
}
