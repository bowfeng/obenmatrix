/// Factory-based platform registration.
///
/// This crate re-exports shared platform types from `oben-platform-sdk` and
/// defines only the gateway-specific factory / registry machinery.

pub use oben_platform_sdk::*;

use tracing::info;

#[cfg(feature = "discord")]
pub use super::discord_adapter::DiscordPlatformFactory;

#[cfg(feature = "telegram")]
pub use super::telegram_adapter::TelegramPlatformFactory;

#[cfg(feature = "whatsapp")]
pub use super::whatsapp_adapter::WhatsAppPlatformFactory;

#[cfg(feature = "slack")]
pub use super::slack_adapter::SlackPlatformFactory;

// ---------------------------------------------------------------------------
// Factory-based platform registration
// ---------------------------------------------------------------------------

/// Factory for the QQ Bot platform.
/// Accepts the config from YAML and converts to internal adapter types on spawn.
pub struct QQBotFactory {
    config: std::sync::Arc<oben_config::QQBotConfig>,
    dispatcher: std::sync::Arc<crate::dispatcher::Dispatcher>,
    response_router: std::sync::Arc<crate::router::ResponseRouter>,
}

impl QQBotFactory {
    pub fn new(
        config: oben_config::QQBotConfig,
        dispatcher: std::sync::Arc<crate::dispatcher::Dispatcher>,
        response_router: std::sync::Arc<crate::router::ResponseRouter>,
    ) -> Self {
        Self {
            config: std::sync::Arc::new(config),
            dispatcher,
            response_router,
        }
    }

    pub fn spawn(&self) -> tokio::task::AbortHandle {
        let config = std::sync::Arc::clone(&self.config);
        let dispatcher = std::sync::Arc::clone(&self.dispatcher);
        let response_router = std::sync::Arc::clone(&self.response_router);
        tokio::spawn(async move {
            // Convert config intents (Vec<QQBotIntent>) to protocol Intents
            let intents = super::qq_protocol::Intents::new().with_guilds().with_group_and_c2c();
            let mut adapter = crate::qq_bot::QQBotAdapter::new(
                &config.app_id,
                &config.app_secret,
                config.sandbox,
                config.shard,
                intents,
                dispatcher,
            );
            // Register a clone with the response router so outbound replies can find it.
            response_router.register("qq_bot", Box::new(adapter.clone())).await;
            // Start listen on the original adapter instance.
            if let Err(e) = adapter.listen().await {
                tracing::error!("QQ Bot adapter crashed: {e}");
            }
        })
        .abort_handle()
    }
}

/// A factory that spawns a platform adapter's listen loop.
pub trait PlatformFactory: Send + 'static {
    fn spawn(&self) -> tokio::task::AbortHandle;
}

impl PlatformFactory for QQBotFactory {
    fn spawn(&self) -> tokio::task::AbortHandle {
        QQBotFactory::spawn(self)
    }
}

/// Wraps a tokio task abort handle for controlling a platform's background task.
pub struct PlatformHandle {
    inner: tokio::task::AbortHandle,
}

impl PlatformHandle {
    pub fn new(inner: tokio::task::AbortHandle) -> Self {
        Self { inner }
    }

    pub fn abort_handle(&self) -> tokio::task::AbortHandle {
        self.inner.clone()
    }

    pub fn abort(&self) {
        self.inner.abort();
    }
}

impl Clone for PlatformHandle {
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone() }
    }
}

/// Registry of platform factories keyed by name.
pub struct PlatformRegistry {
    entries: std::collections::HashMap<String, Box<dyn PlatformFactory>>,
}

impl PlatformRegistry {
    pub fn new() -> Self {
        Self {
            entries: std::collections::HashMap::new(),
        }
    }

    pub fn register<F>(&mut self, name: &str, _label: &str, factory: F)
    where
        F: PlatformFactory,
    {
        self.entries.insert(name.to_string(), Box::new(factory));
    }

    pub fn start_all(&mut self) -> std::result::Result<std::collections::HashMap<String, PlatformHandle>, anyhow::Error> {
        let mut handles = std::collections::HashMap::new();
        for (name, factory) in self.entries.drain() {
            info!(platform = name, "Spawning platform adapter");
            let handle = factory.spawn();
            handles.insert(name, PlatformHandle::new(handle));
        }
        Ok(handles)
    }
}

impl Default for PlatformRegistry {
    fn default() -> Self {
        Self::new()
    }
}
