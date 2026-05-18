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
