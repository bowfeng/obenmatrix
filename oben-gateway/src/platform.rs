/// Factory-based platform registration.
///
/// This crate re-exports shared platform types from `oben-platform-sdk` and
/// defines only the gateway-specific factory / registry machinery.

pub use oben_platform_sdk::*;

use std::sync::Arc;

use tracing::info;

#[cfg(feature = "discord")]
pub use super::discord_adapter::DiscordPlatformFactory;

#[cfg(feature = "telegram")]
pub use super::telegram_adapter::TelegramPlatformFactory;

#[cfg(feature = "whatsapp")]
pub use super::whatsapp_adapter::WhatsAppPlatformFactory;

#[cfg(feature = "slack")]
pub use super::slack_adapter::SlackPlatformFactory;

// ============================================================================
// Platform Adapter Registry (Task 12: Dynamic Adapter Discovery)
// ============================================================================

#[derive(Clone)]
pub struct PlatformEntryConfig {
    pub label: String,
    pub source: String,
    pub install_hint: Option<String>,
    pub required_env: Vec<String>,
    pub max_message_length: usize,
    pub pii_safe: bool,
    pub emoji: String,
    pub allow_update_command: bool,
    pub platform_hint: String,
}

pub type AdapterFactory = Arc<dyn Fn(&oben_config::PlatformConfig) -> Box<dyn PlatformAdapter> + Send + Sync>;
pub type CheckFn = Arc<dyn Fn() -> bool + Send + Sync>;
pub type ValidateConfigFn = Option<Arc<dyn Fn(&oben_config::PlatformConfig) -> bool + Send + Sync>>;

#[derive(Clone)]
pub struct PlatformEntry {
    pub config: PlatformEntryConfig,
    pub check_fn: CheckFn,
    pub validate_config: ValidateConfigFn,
    pub adapter_factory: AdapterFactory,
}

pub struct PlatformAdapterRegistry {
    entries: tokio::sync::RwLock<std::collections::HashMap<String, PlatformEntry>>,
    deferred: tokio::sync::RwLock<std::collections::HashMap<String, Box<dyn Fn() -> anyhow::Result<()> + Send + Sync>>>,
}

impl PlatformAdapterRegistry {
    pub fn new() -> Self {
        Self {
            entries: tokio::sync::RwLock::new(std::collections::HashMap::new()),
            deferred: tokio::sync::RwLock::new(std::collections::HashMap::new()),
        }
    }

    pub async fn register_deferred(
        &self,
        name: &str,
        loader: Box<dyn Fn() -> anyhow::Result<()> + Send + Sync>,
    ) {
        let mut deferred = self.deferred.write().await;
        deferred.insert(name.to_string(), loader);
    }

    async fn resolve(&self, name: &str) -> anyhow::Result<()> {
        let mut deferred = self.deferred.write().await;
        if let Some(loader) = deferred.remove(name) {
            loader().map_err(|e| {
                tracing::warn!("Deferred load of platform '{name}' failed: {e}");
                e
            })?;
        }
        Ok(())
    }

    pub async fn register(&self, entry: PlatformEntry) {
        let mut entries = self.entries.write().await;
        let name = entry.config.label.clone();

        let mut deferred = self.deferred.write().await;
        deferred.remove(&name);

        entries.insert(name.clone(), entry);
        tracing::debug!("Registered platform adapter: {}", name);
    }

    pub async fn get(&self, name: &str) -> Option<PlatformEntry> {
        {
            let entries = self.entries.read().await;
            if let Some(entry) = entries.get(name) {
                return Some(entry.clone());
            }
        }

        if self.deferred.read().await.contains_key(name) {
            if self.resolve(name).await.is_ok() {
                let entries = self.entries.read().await;
                return entries.get(name).cloned();
            }
        }

        None
    }

    pub async fn is_registered(&self, name: &str) -> bool {
        let entries = self.entries.read().await;
        entries.contains_key(name) || self.deferred.read().await.contains_key(name)
    }

    pub async fn create_adapter(
        &self,
        name: &str,
        config: &oben_config::PlatformConfig,
    ) -> Option<Box<dyn PlatformAdapter>> {
        if self.deferred.read().await.contains_key(name) {
            if self.resolve(name).await.is_err() {
                return None;
            }
        }

        let entries = self.entries.read().await;
        let entry = entries.get(name)?;

        if !(entry.check_fn)() {
            tracing::warn!(
                "Platform '{}' requirements not met{}",
                entry.config.label,
                entry.config.install_hint.as_ref().map(|h| format!(" ({h})")).unwrap_or_default()
            );
            return None;
        }

        if let Some(validate_fn) = &entry.validate_config {
            if !validate_fn(config) {
                tracing::warn!("Platform '{}' config validation failed", entry.config.label);
                return None;
            }
        }

        Some((entry.adapter_factory)(config))
    }

    pub async fn all_entries(&self) -> Vec<PlatformEntry> {
        {
            let deferred = self.deferred.read().await;
            for name in deferred.keys() {
                let _ = self.resolve(name).await;
            }
        }

        let entries = self.entries.read().await;
        entries.values().cloned().collect()
    }

    pub async fn plugin_entries(&self) -> Vec<PlatformEntry> {
        let all = self.all_entries().await;
        all.into_iter().filter(|e| e.config.source == "plugin").collect()
    }
}

impl Default for PlatformAdapterRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Legacy factory-based registry (for backward compatibility)
// ============================================================================

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
            let intents = super::qq_protocol::Intents::new().with_guilds().with_group_and_c2c();
            let mut adapter = crate::qq_bot::QQBotAdapter::new(
                &config.app_id,
                &config.app_secret,
                config.sandbox,
                config.shard,
                intents,
                dispatcher,
            );
            response_router.register("qq_bot", Box::new(adapter.clone())).await;
            if let Err(e) = adapter.listen().await {
                tracing::error!("QQ Bot adapter crashed: {e}");
            }
        })
        .abort_handle()
    }
}

pub trait PlatformFactory: Send + 'static {
    fn spawn(&self) -> tokio::task::AbortHandle;
}

impl PlatformFactory for QQBotFactory {
    fn spawn(&self) -> tokio::task::AbortHandle {
        QQBotFactory::spawn(self)
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    struct MockAdapter {
        name: String,
        created: bool,
    }

    impl MockAdapter {
        fn new(name: &str) -> Self {
            Self {
                name: name.to_string(),
                created: false,
            }
        }
    }

    #[async_trait::async_trait]
    impl PlatformAdapter for MockAdapter {
        fn name(&self) -> &str {
            &self.name
        }

        async fn listen(&mut self) -> anyhow::Result<()> {
            Ok(())
        }

        async fn stop(&mut self) {}

        async fn send(&self, _msg: OutgoingMessage) -> anyhow::Result<()> {
            Ok(())
        }

        async fn health_check(&self) -> bool {
            true
        }
    }

    #[test]
    fn test_platform_entry_fields() {
        let entry = PlatformEntry {
            config: PlatformEntryConfig {
                label: "test".to_string(),
                source: "builtin".to_string(),
                install_hint: None,
                required_env: Vec::new(),
                max_message_length: 0,
                pii_safe: false,
                emoji: "🔌".to_string(),
                allow_update_command: true,
                platform_hint: "".to_string(),
            },
            check_fn: Arc::new(|| true),
            validate_config: None,
            adapter_factory: Arc::new(|_cfg| -> Box<dyn PlatformAdapter> {
                Box::new(MockAdapter::new("test"))
            }),
        };

        assert_eq!(entry.config.label, "test");
        assert_eq!(entry.config.source, "builtin");
        assert!((entry.check_fn)());
    }

    #[tokio::test]
    async fn test_registry_register_and_get() {
        let registry = PlatformAdapterRegistry::new();

        let entry = PlatformEntry {
            config: PlatformEntryConfig {
                label: "test_platform".to_string(),
                source: "builtin".to_string(),
                install_hint: None,
                required_env: Vec::new(),
                max_message_length: 0,
                pii_safe: false,
                emoji: "🔌".to_string(),
                allow_update_command: true,
                platform_hint: "".to_string(),
            },
            check_fn: Arc::new(|| true),
            validate_config: None,
            adapter_factory: Arc::new(|_cfg| -> Box<dyn PlatformAdapter> {
                Box::new(MockAdapter::new("test_platform"))
            }),
        };

        registry.register(entry).await;

        let retrieved = registry.get("test_platform").await;
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().config.label, "test_platform");
    }

    #[tokio::test]
    async fn test_registry_is_registered() {
        let registry = PlatformAdapterRegistry::new();

        assert!(!registry.is_registered("nonexistent").await);

        let entry = PlatformEntry {
            config: PlatformEntryConfig {
                label: "registered_platform".to_string(),
                source: "builtin".to_string(),
                install_hint: None,
                required_env: Vec::new(),
                max_message_length: 0,
                pii_safe: false,
                emoji: "🔌".to_string(),
                allow_update_command: true,
                platform_hint: "".to_string(),
            },
            check_fn: Arc::new(|| true),
            validate_config: None,
            adapter_factory: Arc::new(|_cfg| -> Box<dyn PlatformAdapter> {
                Box::new(MockAdapter::new("registered_platform"))
            }),
        };

        registry.register(entry).await;

        assert!(registry.is_registered("registered_platform").await);
    }

    #[tokio::test]
    async fn test_registry_create_adapter_success() {
        let registry = PlatformAdapterRegistry::new();

        let entry = PlatformEntry {
            config: PlatformEntryConfig {
                label: "adapter_platform".to_string(),
                source: "builtin".to_string(),
                install_hint: None,
                required_env: Vec::new(),
                max_message_length: 0,
                pii_safe: false,
                emoji: "🔌".to_string(),
                allow_update_command: true,
                platform_hint: "".to_string(),
            },
            check_fn: Arc::new(|| true),
            validate_config: None,
            adapter_factory: Arc::new(|_cfg| -> Box<dyn PlatformAdapter> {
                Box::new(MockAdapter::new("adapter_platform"))
            }),
        };

        registry.register(entry).await;

        let config = oben_config::PlatformConfig {
            enabled: true,
            token: None,
        };
        let adapter = registry.create_adapter("adapter_platform", &config).await;

        assert!(adapter.is_some());
        assert_eq!(adapter.unwrap().name(), "adapter_platform");
    }

    #[tokio::test]
    async fn test_registry_create_adapter_not_found() {
        let registry = PlatformAdapterRegistry::new();
        let config = oben_config::PlatformConfig {
            enabled: true,
            token: None,
        };

        let adapter = registry.create_adapter("nonexistent", &config).await;
        assert!(adapter.is_none());
    }

    #[tokio::test]
    async fn test_registry_all_entries() {
        let registry = PlatformAdapterRegistry::new();

        registry
            .register(PlatformEntry {
                config: PlatformEntryConfig {
                    label: "platform_a".to_string(),
                    source: "builtin".to_string(),
                    install_hint: None,
                    required_env: Vec::new(),
                    max_message_length: 0,
                    pii_safe: false,
                    emoji: "🔌".to_string(),
                    allow_update_command: true,
                    platform_hint: "".to_string(),
                },
                check_fn: Arc::new(|| true),
                validate_config: None,
                adapter_factory: Arc::new(|_cfg| -> Box<dyn PlatformAdapter> {
                    Box::new(MockAdapter::new("platform_a"))
                }),
            })
            .await;

        registry
            .register(PlatformEntry {
                config: PlatformEntryConfig {
                    label: "platform_b".to_string(),
                    source: "builtin".to_string(),
                    install_hint: None,
                    required_env: Vec::new(),
                    max_message_length: 0,
                    pii_safe: false,
                    emoji: "🔌".to_string(),
                    allow_update_command: true,
                    platform_hint: "".to_string(),
                },
                check_fn: Arc::new(|| true),
                validate_config: None,
                adapter_factory: Arc::new(|_cfg| -> Box<dyn PlatformAdapter> {
                    Box::new(MockAdapter::new("platform_b"))
                }),
            })
            .await;

        let entries = registry.all_entries().await;
        assert_eq!(entries.len(), 2);
        let names: Vec<String> = entries.into_iter().map(|e| e.config.label).collect();
        assert!(names.contains(&"platform_a".to_string()));
        assert!(names.contains(&"platform_b".to_string()));
    }

    #[tokio::test]
    async fn test_registry_plugin_entries() {
        let registry = PlatformAdapterRegistry::new();

        registry
            .register(PlatformEntry {
                config: PlatformEntryConfig {
                    label: "builtin_a".to_string(),
                    source: "builtin".to_string(),
                    install_hint: None,
                    required_env: Vec::new(),
                    max_message_length: 0,
                    pii_safe: false,
                    emoji: "🔌".to_string(),
                    allow_update_command: true,
                    platform_hint: "".to_string(),
                },
                check_fn: Arc::new(|| true),
                validate_config: None,
                adapter_factory: Arc::new(|_cfg| -> Box<dyn PlatformAdapter> {
                    Box::new(MockAdapter::new("builtin_a"))
                }),
            })
            .await;

        registry
            .register(PlatformEntry {
                config: PlatformEntryConfig {
                    label: "plugin_b".to_string(),
                    source: "plugin".to_string(),
                    install_hint: None,
                    required_env: Vec::new(),
                    max_message_length: 0,
                    pii_safe: false,
                    emoji: "🔌".to_string(),
                    allow_update_command: true,
                    platform_hint: "".to_string(),
                },
                check_fn: Arc::new(|| true),
                validate_config: None,
                adapter_factory: Arc::new(|_cfg| -> Box<dyn PlatformAdapter> {
                    Box::new(MockAdapter::new("plugin_b"))
                }),
            })
            .await;

        let entries = registry.plugin_entries().await;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].config.label, "plugin_b");
    }
}
