/// Response routing for agent replies across messaging platforms.
///
/// ResponseRouter maintains a registry of platform adapters keyed by name.
/// When the dispatcher produces a response, this routes it to the correct
/// platform adapter for delivery to the end user.
use std::collections::HashMap;

use anyhow::Result;

use crate::platform::{OutgoingMessage, PlatformAdapter};

/// Routes agent responses to the correct platform adapter.
pub struct ResponseRouter {
    adapters: tokio::sync::RwLock<HashMap<String, Box<dyn PlatformAdapter + Send + Sync>>>,
}

impl ResponseRouter {
    /// Create a new empty ResponseRouter.
    pub fn new() -> Self {
        Self {
            adapters: tokio::sync::RwLock::new(HashMap::new()),
        }
    }

    /// Register a platform adapter under a name.
    ///
    /// Later calls with the same name replace the previous adapter.
    pub async fn register(
        &self,
        name: &str,
        adapter: Box<dyn PlatformAdapter + Send + Sync>,
    ) {
        let mut adapters = self.adapters.write().await;
        adapters.insert(name.to_string(), adapter);
    }

    /// Send a message through the named platform adapter.
    ///
    /// Looks up the adapter by name and calls its `send` method.
    /// Returns an error if the adapter is not registered or send fails.
    pub async fn send(&self, name: &str, msg: OutgoingMessage) -> Result<()> {
        let adapters = self.adapters.read().await;
        match adapters.get(name) {
            Some(adapter) => adapter.send(msg).await,
            None => Err(anyhow::anyhow!("No adapter registered for platform: {name}")),
        }
    }
}

impl Default for ResponseRouter {
    fn default() -> Self {
        Self::new()
    }
}
