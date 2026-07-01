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

    /// Register multiple platform adapters in a single batch.
    ///
    /// Later calls with the same name replace the previous adapter.
    ///
    /// Given: A set of (name, adapter) pairs to register
    /// When: `register_all` is called with an iterator of adapters
    /// Then: All adapters are inserted into the registry at once
    pub async fn register_all(
        &self,
        adapters: impl IntoIterator<Item = (String, Box<dyn PlatformAdapter + Send + Sync>)>,
    ) {
        let mut state = self.adapters.write().await;
        for (name, adapter) in adapters {
            state.insert(name, adapter);
        }
    }

    /// Routes a ResponseMessage to the correct platform adapter.
    ///
    /// The session_key format is `{platform}/{user_id}/{thread_id_or_global}`.
    /// Looks up the adapter by platform name and sends the content.
    pub async fn dispatch_response(
        &self,
        resp: crate::coordinator::ResponseMessage,
    ) -> Result<()> {
        // Parse session_key: platform/user_id/thread
        let mut parts = resp.session_key.splitn(3, '/');
        let platform = parts.next().unwrap_or("unknown");
        let user_id = parts.next().unwrap_or("");
        let thread_id = parts.nth(0).map(|s| {
            if s == "global" || s.is_empty() {
                None
            } else {
                Some(s.to_string())
            }
        }).unwrap_or(None);

        let msg = OutgoingMessage {
            platform: platform.to_string(),
            user_id: user_id.to_string(),
            thread_id,
            content: resp.content,
        };

        self.send(platform, msg).await
    }

    /// Get the names of all registered platforms, sorted alphabetically.
    ///
    /// Given: A set of registered adapters
    /// When: `list_registered` is called
    /// Then: Returns a sorted Vec of platform names
    pub async fn list_registered(&self) -> Vec<String> {
        let adapters = self.adapters.read().await;
        let mut names: Vec<String> = adapters.keys().cloned().collect();
        names.sort();
        names
    }
}

impl Default for ResponseRouter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;

    use super::*;
    use crate::platform::OutgoingMessage;

    struct TestAdapter {
        name_val: String,
    }

    #[async_trait]
    impl PlatformAdapter for TestAdapter {
        fn name(&self) -> &str {
            &self.name_val
        }
        async fn listen(&mut self) -> Result<()> {
            Ok(())
        }
        async fn stop(&mut self) {}
        async fn send(&self, _msg: OutgoingMessage) -> Result<()> {
            Ok(())
        }
        async fn health_check(&self) -> bool {
            true
        }
    }

    #[tokio::test]
    async fn test_response_router_register_all_and_list() {
        let router = ResponseRouter::new();
        assert!(router.list_registered().await.is_empty());

        let adapters: Vec<(String, Box<dyn PlatformAdapter + Send + Sync>)> = vec![
            ("platform_a".to_string(), Box::new(TestAdapter { name_val: "a".to_string() })),
            ("platform_b".to_string(), Box::new(TestAdapter { name_val: "b".to_string() })),
        ];
        router.register_all(adapters).await;

        let names = router.list_registered().await;
        assert_eq!(names, vec!["platform_a", "platform_b"]);

        // Verify send still works
        let msg = OutgoingMessage {
            platform: "platform_a".to_string(),
            user_id: "user-1".to_string(),
            thread_id: None,
            content: "test".to_string(),
        };
        assert!(router.send("platform_a", msg).await.is_ok());
    }
}
