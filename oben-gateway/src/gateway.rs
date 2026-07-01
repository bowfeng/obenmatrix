/// Gateway — manages platform connections and routes messages to the agent.
/// Maps to `gateway/gateway.py` in Hermes.
use anyhow::Result;
use std::collections::HashMap;

use crate::platform::{IncomingMessage, PlatformInfo, PlatformStatus};

use oben_sessions::DBSessionManager;
use tracing::info;

/// Thread-safe registry of platform adapter states.
pub struct PlatformRegistry {
    /// Map from platform name to its info + status.
    state: tokio::sync::RwLock<HashMap<String, PlatformInfo>>,
}

impl PlatformRegistry {
    pub fn new() -> Self {
        Self {
            state: tokio::sync::RwLock::new(HashMap::new()),
        }
    }

    /// Register a platform with its initial info.
    pub async fn register(&self, info: PlatformInfo) {
        let mut state = self.state.write().await;
        state.insert(info.name.clone(), info);
    }

    /// Update platform status. Transitions to the given status.
    pub async fn set_status(&self, name: &str, status: PlatformStatus, error: Option<String>) {
        let mut state = self.state.write().await;
        if let Some(info) = state.get_mut(name) {
            info.status = status;
            info.error = error;
        }
    }

    /// Get a snapshot of all platform names.
    pub async fn names(&self) -> Vec<String> {
        let state = self.state.read().await;
        state.keys().cloned().collect()
    }

    /// Get a snapshot of all platform info.
    pub async fn snapshot(&self) -> HashMap<String, PlatformInfo> {
        let state = self.state.read().await;
        state.clone()
    }

    /// Check if a platform is registered.
    pub async fn has(&self, name: &str) -> bool {
        let state = self.state.read().await;
        state.contains_key(name)
    }
}

impl Default for PlatformRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// The gateway process — listens on multiple platforms and routes to the agent.
pub struct Gateway {
    session_manager: DBSessionManager,
    /// Handles running platform listeners keyed by name; dropped to abort.
    platform_handles: std::sync::Mutex<HashMap<String, tokio::task::AbortHandle>>,
    /// Factory handles from startup pipeline — dropped to stop platforms on drop.
    _factory_handles: HashMap<String, crate::platform::PlatformHandle>,
    /// Thread-safe registry of platform adapter states.
    platform_registry: std::sync::Arc<PlatformRegistry>,
}

impl Gateway {
    pub fn new(
        session_manager: DBSessionManager,
        platform_handles: HashMap<String, crate::platform::PlatformHandle>,
    ) -> Self {
        let mut abort_handles = HashMap::new();
        for (name, handle) in &platform_handles {
            abort_handles.insert(name.clone(), handle.abort_handle());
        }
        Self {
            session_manager,
            platform_handles: std::sync::Mutex::new(abort_handles),
            _factory_handles: platform_handles,
            platform_registry: std::sync::Arc::new(PlatformRegistry::new()),
        }
    }

    /// Handle an incoming message (simplified — no platform-specific routing).
    pub async fn handle_message(&self, msg: IncomingMessage) -> Result<String> {
        let preview = if msg.content.len() > 30 {
            msg.content.chars().take(30).collect::<String>()
        } else {
            msg.content.clone()
        };
        info!(
            "Gateway received {} message from {} ({})",
            msg.platform, msg.user_id, preview
        );

        // TODO: Route through conversation loop
        // For now, just echo
        Ok(format!("Echo: {}", msg.content))
    }

    /// Start the gateway — run all enabled platform adapter listeners.
    /// Block until Ctrl-C. Called from #[tokio::main], so no nested runtime.
    ///
    /// Platforms are started via the factory startup pipeline (see main.rs).
    /// The factory handles are held in _factory_handles and dropped on shutdown.
    pub async fn start_blocking(&self) -> Result<()> {
        let handles = self.platform_handles.lock().unwrap();
        info!(
            platforms_active = handles.len(),
            "Gateway starting — platforms active via factory pipeline"
        );
        drop(handles);

        // Keep running until Ctrl-C; platforms are maintained by factory pipeline
        tokio::signal::ctrl_c().await?;
        info!("Shutting down gateway...");
        Ok(())
    }

    pub fn session_manager(&self) -> &DBSessionManager {
        &self.session_manager
    }

    pub fn platform_handles(
        &self,
    ) -> std::sync::MutexGuard<'_, HashMap<String, tokio::task::AbortHandle>> {
        self.platform_handles.lock().unwrap()
    }

    /// Register a platform adapter in the registry.
    pub async fn register_platform(&self, info: PlatformInfo) {
        self.platform_registry.register(info).await;
    }

    /// Update a platform's status.
    pub async fn update_platform_status(
        &self,
        name: &str,
        status: PlatformStatus,
        error: Option<String>,
    ) {
        self.platform_registry
            .set_status(name, status, error)
            .await;
    }

    /// Get a snapshot of all platform statuses.
    /// Uses async snapshot to avoid blocking the runtime.
    pub async fn platform_status(&self) -> HashMap<String, PlatformInfo> {
        self.platform_registry.snapshot().await
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::*;

    fn make_gateway() -> Gateway {
        Gateway::new(
            DBSessionManager::new().unwrap(),
            HashMap::new(),
        )
    }

    #[tokio::test]
    async fn test_handle_message_echo() {
        let gateway = make_gateway();
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
        let gateway = make_gateway();
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
        let gateway = make_gateway();
        let long_content = "a".repeat(50);
        let msg = IncomingMessage {
            platform: "slack".to_string(),
            user_id: "user-3".to_string(),
            username: None,
            content: long_content.clone(),
            thread_id: None,
        };
        let result = gateway.handle_message(msg).await.unwrap();
        assert_eq!(result, format!("Echo: {}", long_content));
    }

    #[tokio::test]
    async fn test_handle_message_empty_content() {
        let gateway = make_gateway();
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

    /// Given: A newly created gateway with an empty platform registry
    /// When: A platform is registered with Running status via register_platform()
    /// Then: platform_status() returns a map containing that platform
    #[tokio::test]
    async fn test_platform_registry_register() {
        let gateway = make_gateway();
        let info = PlatformInfo {
            name: "test".to_string(),
            status: PlatformStatus::Running,
            started_at: None,
            error: None,
        };
        gateway.register_platform(info).await;
        let status = gateway.platform_status().await;
        assert!(status.contains_key("test"));
        assert_eq!(status["test"].status, PlatformStatus::Running);
    }

    /// Given: A gateway with a registered platform in Idle status
    /// When: The platform status is updated to Failed with an error message
    /// Then: platform_status() reflects the new status and error text
    #[tokio::test]
    async fn test_platform_registry_update_status() {
        let gateway = make_gateway();
        let info = PlatformInfo {
            name: "test".to_string(),
            status: PlatformStatus::Idle,
            started_at: None,
            error: None,
        };
        gateway.register_platform(info).await;
        gateway
            .update_platform_status(
                "test",
                PlatformStatus::Failed("auth_expired".to_string()),
                Some("auth_expired".to_string()),
            )
            .await;
        let status = gateway.platform_status().await;
        assert_eq!(status["test"].status, PlatformStatus::Failed("auth_expired".to_string()));
        assert_eq!(status["test"].error, Some("auth_expired".to_string()));
    }

}
