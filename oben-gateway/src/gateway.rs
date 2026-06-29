/// Gateway — manages platform connections and routes messages to the agent.
/// Maps to `gateway/gateway.py` in Hermes.
use anyhow::Result;
use super::qq_protocol::Intents as QQIntents;

use crate::dispatcher::Dispatcher;
use crate::platform::{IncomingMessage, PlatformAdapter};
use crate::qq_bot::QQBotAdapter;

use oben_config::GatewayConfig;
use oben_sessions::DBSessionManager;
use tracing::info;

/// The gateway process — listens on multiple platforms and routes to the agent.
pub struct Gateway {
    session_manager: DBSessionManager,
    config: GatewayConfig,
    dispatcher: std::sync::Arc<Dispatcher>,
    /// Handles running platform listeners; dropped to abort.
    platform_handles: std::sync::Mutex<Vec<tokio::task::AbortHandle>>,
}

impl Gateway {
    pub fn new(
        session_manager: DBSessionManager,
        config: GatewayConfig,
        dispatcher: std::sync::Arc<Dispatcher>,
    ) -> Self {
        Self {
            session_manager,
            config,
            dispatcher,
            platform_handles: std::sync::Mutex::new(Vec::new()),
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
                oben_config::QQBotIntent::Guilds => { /* already included */ }
                oben_config::QQBotIntent::C2cMessage => { /* included in GROUP_AND_C2C */ }
                oben_config::QQBotIntent::GroupAtMessage => { /* included in GROUP_AND_C2C */ }
            }
        }
        intents
    }

    /// Start the gateway — run all enabled platform adapter listeners.
    /// Block until Ctrl-C. Called from #[tokio::main], so no nested runtime.
    pub async fn start_blocking(&self) -> Result<()> {
        info!("Gateway starting with configured platforms...");

        let handles = self.start_platforms().await?;

        // Store handles for shutdown
        {
            let mut h = self.platform_handles.lock().unwrap();
            h.extend(handles.into_iter().map(|h| h.abort_handle()));
        }

        // Keep running until Ctrl-C
        tokio::signal::ctrl_c().await?;
        info!("Shutting down gateway...");
        Ok(())
    }

    async fn start_platforms(&self) -> Result<Vec<tokio::task::JoinHandle<()>>> {
        let mut handles = Vec::new();

        // ── QQ Bot ─────────────────────────────────────────────────────
        if let Some(ref qq_config) = self.config.qq_bot {
            if qq_config.enabled {
                info!(
                    app_id = %qq_config.app_id,
                    sandbox = qq_config.sandbox,
                    "Starting QQ Bot adapter"
                );

                let intents = Self::parse_qq_intents(qq_config);

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
                    if let Err(e) = a.listen(Box::new(QqMessageHandler)).await {
                        tracing::error!("QQ Bot adapter crashed: {}", e);
                    }
                });
                handles.push(handle);
            }
        }

        if handles.is_empty() {
            info!("No platform adapters enabled in config; gateway will block.");
        }

        Ok(handles)
    }

    pub fn session_manager(&self) -> &DBSessionManager {
        &self.session_manager
    }

    pub fn platform_handles(
        &self,
    ) -> std::sync::MutexGuard<'_, Vec<tokio::task::AbortHandle>> {
        self.platform_handles.lock().unwrap()
    }
}

/// Message handler implementation for QQ Bot.
/// Routes incoming messages into the gateway processing pipeline.
struct QqMessageHandler;

#[async_trait::async_trait]
impl crate::platform::MessageHandler for QqMessageHandler {
    async fn handle(&self, msg: IncomingMessage) -> Result<Option<String>> {
        info!(platform = %msg.platform, user_id = %msg.user_id, "QQ Bot message received");
        // TODO: Route to agent conversation loop
        let _ = msg;
        Ok(None)
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
}
