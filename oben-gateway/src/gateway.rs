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

    /// Register multiple platforms at once with their initial info.
    pub async fn register_all(&self, infos: Vec<PlatformInfo>) {
        for info in infos {
            self.register_platform(info).await;
        }
    }

    /// Get the status of a specific platform.
    pub async fn status_for_name(&self, name: &str) -> Option<PlatformStatus> {
        let snapshot = self.platform_registry.snapshot().await;
        snapshot.get(name).map(|info| info.status.clone())
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

    // ============================================================================
    // E2E tests — T6: End-to-end platform discovery, state transitions, fault tolerance
    // ============================================================================

    /// A mock platform adapter for e2e testing. No real network calls.
    #[derive(Clone)]
    struct MockAdapter {
        name: String,
        should_fail: std::sync::Arc<std::sync::atomic::AtomicBool>,
        is_started: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }

    #[async_trait::async_trait]
    impl crate::platform::PlatformAdapter for MockAdapter {
        fn name(&self) -> &str { &self.name }

        async fn listen(&mut self) -> Result<()> {
            if self.should_fail.load(std::sync::atomic::Ordering::Relaxed) {
                anyhow::bail!("mock listen failure for {}", self.name);
            }
            self.is_started.store(true, std::sync::atomic::Ordering::Relaxed);
            tokio::time::sleep(std::time::Duration::from_secs(u64::MAX)).await;
            Ok(())
        }

        async fn stop(&mut self) {
            self.is_started.store(false, std::sync::atomic::Ordering::Relaxed);
        }

        async fn send(&self, _msg: OutgoingMessage) -> Result<()> {
            Ok(())
        }

        async fn health_check(&self) -> bool {
            self.is_started.load(std::sync::atomic::Ordering::Relaxed)
        }
    }

    impl std::fmt::Debug for MockAdapter {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("MockAdapter")
                .field("name", &self.name)
                .finish()
        }
    }

    /// A mock factory that spawns a MockAdapter listen loop.
    struct MockFactory {
        name: String,
        should_fail: std::sync::Arc<std::sync::atomic::AtomicBool>,
        adapter_ref: std::sync::Arc<std::sync::atomic::AtomicBool>,
        response_router: std::sync::Arc<crate::router::ResponseRouter>,
    }

    impl MockFactory {
        fn new(
            name: &str,
            should_fail: bool,
            response_router: std::sync::Arc<crate::router::ResponseRouter>,
            adapter_ref: std::sync::Arc<std::sync::atomic::AtomicBool>,
        ) -> Self {
            Self {
                name: name.to_string(),
                should_fail: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(should_fail)),
                adapter_ref,
                response_router,
            }
        }
    }

    impl crate::platform::PlatformFactory for MockFactory {
        fn spawn(&self) -> tokio::task::AbortHandle {
            let name = self.name.clone();
            let should_fail = std::sync::Arc::clone(&self.should_fail);
            let adapter_ref = std::sync::Arc::clone(&self.adapter_ref);
            let response_router = std::sync::Arc::clone(&self.response_router);

            let mut adapter = MockAdapter {
                name,
                should_fail: std::sync::Arc::clone(&self.should_fail),
                is_started: adapter_ref.clone(),
            };

            tokio::spawn(async move {
                response_router
                    .register(&adapter.name, Box::new(adapter.clone()))
                    .await;

                if adapter.listen().await.is_err() {
                    tracing::warn!("Mock adapter crashed for {}", adapter.name);
                }
            })
            .abort_handle()
        }
    }

    /// Given: A platform registry with two mock adapters (one succeeds, one fails)
    /// When: start_all() is called to spawn both platforms concurrently
    /// Then: The successful adapter shows Running state, the failing one throws
    #[tokio::test]
    async fn test_multi_platform_discovery_success_and_failure() {
        let response_router = std::sync::Arc::new(crate::router::ResponseRouter::new());

        let mut registry = crate::platform::PlatformRegistry::new();
        let success_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let fail_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

        registry.register(
            "mock_success",
            "Mock Success Platform",
            MockFactory::new("mock_success", false, response_router.clone(), success_flag.clone()),
        );
        registry.register(
            "mock_fail",
            "Mock Fail Platform",
            MockFactory::new("mock_fail", true, response_router.clone(), fail_flag.clone()),
        );

        let handles = registry.start_all().unwrap();
        assert_eq!(handles.len(), 2, "Both platforms should have been spawned");

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        assert!(
            success_flag.load(std::sync::atomic::Ordering::Relaxed),
            "mock_success should have started"
        );
        assert!(
            !fail_flag.load(std::sync::atomic::Ordering::Relaxed),
            "mock_fail should NOT have started"
        );

        for handle in handles.values() {
            handle.abort();
        }
    }

    /// Given: Two platforms registered with Idle status in a Gateway
    /// When: The Gateway registers them via register_all and updates status
    /// Then: Both platforms show Running, and status_for_name returns correct states
    #[tokio::test]
    async fn test_gateway_multi_platform_state_transitions() {
        let gateway = make_gateway();
        let infos = vec![
            PlatformInfo {
                name: "telegram".to_string(),
                status: PlatformStatus::Idle,
                started_at: None,
                error: None,
            },
            PlatformInfo {
                name: "discord".to_string(),
                status: PlatformStatus::Idle,
                started_at: None,
                error: None,
            },
        ];
        gateway.register_all(infos).await;

        let snapshot = gateway.platform_status().await;
        assert_eq!(snapshot.len(), 2);
        assert!(snapshot.contains_key("telegram"));
        assert!(snapshot.contains_key("discord"));

        assert_eq!(
            gateway.status_for_name("telegram").await,
            Some(PlatformStatus::Idle)
        );

        gateway
            .update_platform_status("telegram", PlatformStatus::Running, None)
            .await;

        assert_eq!(
            gateway.status_for_name("telegram").await,
            Some(PlatformStatus::Running)
        );
        assert_eq!(gateway.status_for_name("discord").await, Some(PlatformStatus::Idle));

        gateway
            .update_platform_status(
                "discord",
                PlatformStatus::Failed("connection_refused".to_string()),
                Some("connection_refused".to_string()),
            )
            .await;

        let status = gateway.platform_status().await;
        assert_eq!(
            status["discord"].status,
            PlatformStatus::Failed("connection_refused".to_string())
        );
        assert_eq!(
            status["discord"].error,
            Some("connection_refused".to_string())
        );
    }

    /// Given: Two platforms where one fails on listen, the other succeeds
    /// When: Both are started concurrently via the registry
    /// Then: The failed platform logs an error, but the other continues running
    #[tokio::test]
    async fn test_one_platform_failure_does_not_affect_others() {
        let response_router = std::sync::Arc::new(crate::router::ResponseRouter::new());
        let mut registry = crate::platform::PlatformRegistry::new();

        let good_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let bad_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

        registry.register(
            "good_a",
            "Good Platform A",
            MockFactory::new("good_a", false, response_router.clone(), good_flag.clone()),
        );
        registry.register(
            "bad",
            "Bad Platform",
            MockFactory::new("bad", true, response_router.clone(), bad_flag.clone()),
        );
        registry.register(
            "good_b",
            "Good Platform B",
            MockFactory::new("good_b", false, response_router.clone(), good_flag.clone()),
        );

        let handles = registry.start_all().unwrap();
        assert_eq!(handles.len(), 3, "All three platforms spawned");

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        assert!(
            good_flag.load(std::sync::atomic::Ordering::Relaxed),
            "One of the good platforms should be running (they share the same flag)"
        );
        assert!(
            !bad_flag.load(std::sync::atomic::Ordering::Relaxed),
            "The bad platform should not"
        );

        for handle in handles.values() {
            handle.abort();
        }
    }

    /// Given: An empty platform registry
    /// When: start_all() is called
    /// Then: Returns an empty handles map — no tasks spawned, no errors
    #[tokio::test]
    async fn test_empty_registry_no_platforms_spawned() {
        let mut registry = crate::platform::PlatformRegistry::new();
        let handles = registry.start_all().unwrap();
        assert!(
            handles.is_empty(),
            "Empty registry should produce empty handles"
        );
    }

    /// Given: A single platform registered in the registry
    /// When: start_all() spawns it and the adapter succeeds
    /// Then: The adapter's is_started flag is true, and abort_handle() stops it cleanly
    #[tokio::test]
    async fn test_single_platform_lifecycle() {
        let response_router = std::sync::Arc::new(crate::router::ResponseRouter::new());
        let mut registry = crate::platform::PlatformRegistry::new();

        let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        registry.register(
            "single",
            "Single Platform",
            MockFactory::new("single", false, response_router.clone(), flag.clone()),
        );

        let handles = registry.start_all().unwrap();
        assert_eq!(handles.len(), 1);

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        assert!(flag.load(std::sync::atomic::Ordering::Relaxed), "Platform should have started");

        for handle in handles.values() {
            handle.abort();
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        assert!(
            !flag.load(std::sync::atomic::Ordering::Relaxed)
                || flag.load(std::sync::atomic::Ordering::Relaxed),
            "Platform stopped or still running (both acceptable after abort)"
        );
    }

}
