/// Gateway — manages platform connections and routes messages to the agent.
/// Maps to `gateway/gateway.py` in Hermes.
use anyhow::Result;
use super::qq_protocol::Intents as QQIntents;
use std::collections::HashMap;

use crate::dispatcher::Dispatcher;
use crate::platform::{IncomingMessage, PlatformAdapter, PlatformInfo, PlatformStatus};
use crate::qq_bot::QQBotAdapter;

use oben_config::GatewayConfig;
use oben_sessions::DBSessionManager;
use tracing::{info, warn};

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
    config: GatewayConfig,
    dispatcher: std::sync::Arc<Dispatcher>,
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
        config: GatewayConfig,
        dispatcher: std::sync::Arc<Dispatcher>,
        platform_handles: HashMap<String, crate::platform::PlatformHandle>,
    ) -> Self {
        let mut abort_handles = HashMap::new();
        for (name, handle) in &platform_handles {
            abort_handles.insert(name.clone(), handle.abort_handle());
        }
        Self {
            session_manager,
            config,
            dispatcher,
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

    /// Parse QQBot intents from config into Intents bitflags.
    fn parse_qq_intents(config: &oben_config::QQBotConfig) -> QQIntents {
        let intents = QQIntents::new().with_guilds().with_group_and_c2c();
        for intent in &config.intents {
match intent {
    oben_config::QQBotIntent::DirectMessage => {}
    oben_config::QQBotIntent::C2CAndGroup => {}
    oben_config::QQBotIntent::Interaction => {}
            }
        }
        intents
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

    async fn start_platforms(&self) -> Result<(Vec<tokio::task::JoinHandle<()>>, Vec<String>)> {
        let mut handles = Vec::new();
        let mut platform_names = Vec::new();
        let config = &self.config;
        let registry = self.registry();

        // ── Telegram ──────────────────────────────────────────────────
        if let Some(ref tg_config) = config.telegram {
            if tg_config.enabled {
                let name = "telegram".to_string();
                platform_names.push(name.clone());
                info!("Starting Telegram platform adapter");
                registry.register(PlatformInfo {
                    name: name.clone(),
                    status: PlatformStatus::Connecting,
                    started_at: None,
                    error: None,
                }).await;
                registry.set_status("telegram", PlatformStatus::Failed("Not implemented yet".into()), Some("platform adapter not implemented".to_string())).await;
                warn!("Telegram platform placeholder — no adapter implemented yet");
            }
        }

        // ── Discord ───────────────────────────────────────────────────
        if let Some(ref dc_config) = config.discord {
            if dc_config.enabled {
                let name = "discord".to_string();
                platform_names.push(name.clone());
                info!("Starting Discord platform adapter");
                registry.register(PlatformInfo {
                    name: name.clone(),
                    status: PlatformStatus::Connecting,
                    started_at: None,
                    error: None,
                }).await;
                registry.set_status("discord", PlatformStatus::Failed("Not implemented yet".into()), Some("platform adapter not implemented".to_string())).await;
                warn!("Discord platform placeholder — no adapter implemented yet");
            }
        }

        // ── Slack ─────────────────────────────────────────────────────
        if let Some(ref sl_config) = config.slack {
            if sl_config.enabled {
                let name = "slack".to_string();
                platform_names.push(name.clone());
                info!("Starting Slack platform adapter");
                registry.register(PlatformInfo {
                    name: name.clone(),
                    status: PlatformStatus::Connecting,
                    started_at: None,
                    error: None,
                }).await;
                registry.set_status("slack", PlatformStatus::Failed("Not implemented yet".into()), Some("platform adapter not implemented".to_string())).await;
                warn!("Slack platform placeholder — no adapter implemented yet");
            }
        }

        // ── WhatsApp ──────────────────────────────────────────────────
        if let Some(ref wa_config) = config.whatsapp {
            if wa_config.enabled {
                let name = "whatsapp".to_string();
                platform_names.push(name.clone());
                info!("Starting WhatsApp platform adapter");
                registry.register(PlatformInfo {
                    name: name.clone(),
                    status: PlatformStatus::Connecting,
                    started_at: None,
                    error: None,
                }).await;
                registry.set_status("whatsapp", PlatformStatus::Failed("Not implemented yet".into()), Some("platform adapter not implemented".to_string())).await;
                warn!("WhatsApp platform placeholder — no adapter implemented yet");
            }
        }

        // ── QQ Bot ────────────────────────────────────────────────────
        if let Some(ref qq_config) = config.qq_bot {
            if qq_config.enabled {
                let name = "qq_bot".to_string();
                platform_names.push(name.clone());
                registry.register(PlatformInfo {
                    name: name.clone(),
                    status: PlatformStatus::Connecting,
                    started_at: None,
                    error: None,
                }).await;

                let intents = Self::parse_qq_intents(qq_config);
                info!(
                    app_id = %qq_config.app_id,
                    sandbox = qq_config.sandbox,
                    intents_value = %intents.to_u64(),
                    "Starting QQ Bot adapter"
                );

                let adapter = QQBotAdapter::new(
                    &qq_config.app_id,
                    &qq_config.app_secret,
                    qq_config.sandbox,
                    qq_config.shard,
                    intents,
                    self.dispatcher.clone(),
                );

                let handle = tokio::spawn(async move {
                    let mut a = adapter;
                    if let Err(e) = a.listen().await {
                        tracing::error!("QQ Bot adapter crashed: {}", e);
                    }
                });
                handles.push(handle);

                registry.set_status("qq_bot", PlatformStatus::Running, None).await;
                info!("QQ Bot adapter started successfully");
            }
        }

        if handles.is_empty() {
            info!("No platform adapters enabled in config; gateway will block on ctrl-c");
        }

        Ok((handles, platform_names))
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

    /// Get the PlatformRegistry reference for use by start_platforms.
    pub fn registry(&self) -> std::sync::Arc<PlatformRegistry> {
        self.platform_registry.clone()
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::*;
    use crate::router::ResponseRouter;

    fn make_gateway() -> Gateway {
        Gateway::new(
            DBSessionManager::new().unwrap(),
            oben_config::GatewayConfig::default(),
            std::sync::Arc::new(Dispatcher::new(
                oben_config::AppConfig::default(),
                std::sync::Arc::new(oben_tools::ToolRegistry::new()),
                std::sync::Arc::new(ResponseRouter::new()),
            )),
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

    /// Given: A gateway with default (empty) config — no platforms enabled
    /// When: start_platforms() is called
    /// Then: Returns empty handles list and logs warning
    #[tokio::test]
    async fn test_start_platforms_empty_config() {
        let gateway = make_gateway();
        let (handles, _names) = gateway.start_platforms().await.unwrap();
        assert!(handles.is_empty());
    }
}
