/// Response routing for agent replies across messaging platforms.
///
/// ResponseRouter maintains a registry of platform adapters keyed by name.
/// When the dispatcher produces a response, this routes it to the correct
/// platform adapter for delivery to the end user.
use std::collections::HashMap;

use anyhow::Result;

use crate::platform::{OutgoingMessage, PlatformAdapter, PlatformSessionContext};

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
    /// Uses PlatformSessionContext to parse and reconstruct the context.
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

        let context = PlatformSessionContext::with_thread_id(platform, user_id, thread_id);

        let msg = OutgoingMessage {
            platform: context.platform.clone(),
            user_id: context.user_id.clone(),
            thread_id: context.thread_id.clone(),
            content: resp.content,
        };

        self.send(&context.platform, msg).await
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

    /// Routes a PlatformSessionContext to the correct platform adapter.
    ///
    /// Given: A PlatformSessionContext with platform, user_id, and optional thread_id
    /// When: dispatch_response_with_context is called
    /// Then: Response is routed to the correct platform adapter
    pub async fn dispatch_response_with_context(
        &self,
        context: PlatformSessionContext,
        content: String,
    ) -> Result<()> {
        let msg = OutgoingMessage {
            platform: context.platform.clone(),
            user_id: context.user_id.clone(),
            thread_id: context.thread_id.clone(),
            content,
        };
        self.send(&context.platform, msg).await
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
    use crate::platform::{OutgoingMessage, PlatformSessionContext};

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

    #[tokio::test]
    async fn test_response_router_dispatch_with_context() {
        use super::PlatformSessionContext;

        let router = ResponseRouter::new();

        // Register a test adapter
        let adapters: Vec<(String, Box<dyn PlatformAdapter + Send + Sync>)> = vec![
            ("test_platform".to_string(), Box::new(TestAdapter { name_val: "test_platform".to_string() })),
        ];
        router.register_all(adapters).await;

        // Dispatch with PlatformSessionContext
        let context = PlatformSessionContext::new("test_platform", "user-123");
        let content = "hello from context";

        // Should not error (adapter exists)
        let result = router.dispatch_response_with_context(context, content.to_string()).await;
        assert!(result.is_ok());
    }

    #[test]
    fn test_platform_session_context_isolation() {
        // Verify session keys are unique across platforms
        let telegram_ctx = PlatformSessionContext::new("telegram", "user-123");
        let discord_ctx = PlatformSessionContext::new("discord", "user-123");
        let slack_ctx = PlatformSessionContext::new("slack", "user-123");

        assert_ne!(telegram_ctx.session_key(), discord_ctx.session_key());
        assert_ne!(discord_ctx.session_key(), slack_ctx.session_key());
        assert_ne!(telegram_ctx.session_key(), slack_ctx.session_key());

        assert_eq!(telegram_ctx.session_key(), "telegram/user-123/global");
        assert_eq!(discord_ctx.session_key(), "discord/user-123/global");
        assert_eq!(slack_ctx.session_key(), "slack/user-123/global");
    }

    #[test]
    fn test_platform_session_context_with_thread() {
        let ctx = PlatformSessionContext::with_thread_id("telegram", "user-123", Some("thread-456".to_string()));
        assert_eq!(ctx.session_key(), "telegram/user-123/thread-456");

        let ctx = PlatformSessionContext::with_thread_id("telegram", "user-123", None);
        assert_eq!(ctx.session_key(), "telegram/user-123/global");
    }
}
