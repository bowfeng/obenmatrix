//! Top-level entry point for the multi-platform messaging gateway.
//!
//! Connects platform adapters to a central Dispatcher through the factory
//! startup pipeline. On Ctrl+C, platforms are gracefully stopped via their
//! handles.

use std::sync::Arc;

use anyhow::Result;
use tracing::info;
use oben_config::AppConfig;
use oben_gateway::{Dispatcher, Gateway, ResponseRouter};
use oben_sessions::DBSessionManager;
use oben_tools::ToolRegistry;

/// Directory where gateway logs are persisted.
pub const LOG_DIR: &str = ".config/obenalien/logs";
pub const LOG_FILE_PREFIX: &str = "gateway";

fn init_logging() {
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

    let fmt_layer = tracing_subscriber::fmt::layer().with_target(false);

    let home = std::env::var("HOME").unwrap_or_default();
    let log_dir = std::path::Path::new(&home).join(LOG_DIR);
    let _ = std::fs::create_dir_all(&log_dir);

    let file_appender = tracing_appender::rolling::RollingFileAppender::new(
        tracing_appender::rolling::Rotation::DAILY,
        &log_dir,
        LOG_FILE_PREFIX,
    );

    let file_layer = tracing_subscriber::fmt::layer()
        .with_target(false)
        .with_writer(file_appender);

    let env_filter = EnvFilter::try_from_env("RUST_LOG")
        .unwrap_or_else(|_| EnvFilter::new("oben=info,gateway=info"));

    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer)
        .with(file_layer)
        .try_init()
        .unwrap_or_else(|e| tracing::warn!("tracing init failed: {e}"));

    info!(log_dir = %log_dir.display(), "Logging initialized");
}

/// Create a ToolRegistry populated with all built-in tools.
fn create_tool_registry() -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    oben_tools::discover_builtin_tools(&mut registry);
    registry
}

#[tokio::main]
async fn main() -> Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install ring crypto provider");

    init_logging();
    info!("=== Oben Gateway Starting ===");

    let app_config = AppConfig::load()?;

    let gateway_config = app_config
        .gateway
        .clone()
        .unwrap_or_default();

    info!(
        model = %app_config.model.model,
        max_iterations = %app_config.max_iterations.unwrap_or(50),
        "Config loaded"
    );

    let session_manager = DBSessionManager::new()?;
    info!("Session manager initialized");

    let response_router = Arc::new(ResponseRouter::new());

    let tools = Arc::new(create_tool_registry());
    info!("Tool registry created");

    let dispatcher = Arc::new(Dispatcher::new(
        app_config.clone(),
        tools,
        response_router.clone(),
    ));
    info!("Dispatcher created");

    // Start platforms through the factory-based pipeline.
    // QQBotFactory::spawn() builds the QQBotAdapter and starts its async listen loop,
    // eliminating the hardcoded if/elif chain from the startup flow.
    // QQBotConfig is built from gateway config and converted to internal types in factory.
    let mut registry = oben_gateway::platform::PlatformRegistry::new();
    let platform_handles = {
        if let Some(ref qq_cfg) = gateway_config.qq_bot {
            if qq_cfg.enabled {
                let config = oben_config::QQBotConfig {
                    enabled: qq_cfg.enabled,
                    app_id: qq_cfg.app_id.clone(),
                    app_secret: qq_cfg.app_secret.clone(),
                    intents: qq_cfg.intents.clone(),
                    shard: qq_cfg.shard,
                    sandbox: qq_cfg.sandbox,
                };
                let factory = oben_gateway::platform::QQBotFactory::new(
                    config,
                    dispatcher.clone(),
                    response_router.clone(),
                );
                registry.register("qq_bot", "Tencent QQ Bot", factory);
            }
        };
        registry.start_all()?
    };
    info!("Platforms started via factory pipeline");

    info!("Gateway initialized — calling start_blocking()");
    info!("Press Ctrl+C to shut down");

    let gateway = Gateway::new(session_manager, platform_handles);
    gateway.start_blocking().await?;

    info!("Gateway shut down cleanly");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Given: Empty platform registry with no factories registered
    /// When: start_all() is called
    /// Then: Returns an empty platform_handles map with no errors
    #[tokio::test]
    async fn test_platform_registry_empty() {
        let mut registry = oben_gateway::platform::PlatformRegistry::new();
        let handles = registry.start_all().unwrap();
        assert!(handles.is_empty());
    }

    /// Given: QQ Bot configuration with factory registered
    /// When: start_all() is called
    /// Then: Returns handles containing the platform entry
    #[tokio::test]
    async fn test_platform_registry_qq_factory() {
        let mut registry = oben_gateway::platform::PlatformRegistry::new();
        let response_router = Arc::new(super::ResponseRouter::new());
        let dispatcher = Arc::new(Dispatcher::new(
            AppConfig::default(),
            Arc::new(ToolRegistry::new()),
            response_router.clone(),
        ));
        let config = oben_config::QQBotConfig {
            enabled: true,
            app_id: "test".to_string(),
            app_secret: "test".to_string(),
            intents: vec![],
            shard: None,
            sandbox: false,
        };
        let factory = oben_gateway::platform::QQBotFactory::new(config, dispatcher, response_router);
        registry.register("qq_bot", "Tencent QQ Bot", factory);

        let handles = registry.start_all().unwrap();
        assert_eq!(handles.len(), 1, "Expected 1 platform handle");
        handles.values().next().unwrap().abort();
    }
}
