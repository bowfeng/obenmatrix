//! Top-level entry point for the multi-platform messaging gateway.
//!
//! Connects platform adapters to a central Dispatcher through the factory
//! startup pipeline. On Ctrl+C, platforms are gracefully stopped via their
//! handles.

use std::sync::Arc;

use anyhow::Result;
use tracing::info;
use oben_agent::hooks::HookBuilder;
use oben_config::AppConfig;
use oben_gateway::{Dispatcher, Gateway, ResponseRouter};
use oben_sessions::DBSessionManager;
use oben_tools::ToolRegistry;

/// Directory where gateway logs are persisted.
pub const LOG_DIR: &str = ".config/obenmatrix/logs";
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

/// Load WASM hook adapters and inject them into a HookBuilder via typed channels.
///
/// Returns the builder unchanged when no plugins are found or the feature is disabled.
#[cfg(feature = "wasm-plugins")]
async fn load_wasm_hooks(
    builder: HookBuilder,
    plugins_dir: &Option<std::path::PathBuf>,
) -> HookBuilder {
    use std::path::PathBuf;
    use oben_wasm::{WasmHookRegistry, WasmRuntime, WasmRuntimeConfig};

    let plugin_path = plugins_dir
        .clone()
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join(".obenmatrix").join("plugins").join("wasm"))
        });

    let Some(pdir) = plugin_path.as_ref()
        .filter(|p| p.exists() && p.is_dir())
    else {
        return builder;
    };

    tracing::info!(?pdir, "Loading WASM hook components");

    let runtime = match WasmRuntime::new(WasmRuntimeConfig::default()) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "WASM runtime creation failed");
            return builder;
        }
    };

    let registry = WasmHookRegistry::new(runtime, pdir.clone());
    let components = match registry.load_hooks().await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "WASM hook loading failed");
            return builder;
        }
    };

    let hook_components = match registry.instantiate_hooks(components).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "WASM adapter instantiation failed");
            return builder;
        }
    };

    builder
        .with_agent_loop_hooks(hook_components.agent_loop)
        .with_turn_hooks(hook_components.turn)
        .with_tool_hooks(hook_components.tool)
        .with_streaming_hooks(hook_components.streaming)
        .with_system_hooks(hook_components.system)
        .with_session_hooks(hook_components.session)
        .with_interrupt_hooks(hook_components.interrupt)
}

#[tokio::main]
async fn main() -> Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install ring crypto provider");

    init_logging();
    info!("=== Oben Gateway Starting ===");

    let app_config = AppConfig::load(None)?;

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

    // Build HookEngine with NudgeHook + WASM hook adapters
    #[cfg(feature = "wasm-plugins")]
    let hook_engine = load_wasm_hooks(
        HookBuilder::from_config(&app_config.hooks),
        &gateway_config.plugin_dir,
    )
    .await
    .build();

    #[cfg(not(feature = "wasm-plugins"))]
    let hook_engine = HookBuilder::from_config(&app_config.hooks).build();

    tracing::info!("HookEngine built");

    let dispatcher = Arc::new(Dispatcher::new(
        app_config.clone(),
        tools,
        response_router.clone(),
        Arc::new(hook_engine),
    ));
    info!("Dispatcher created");

    // Start platforms through the factory-based pipeline.
    // QQBotFactory::spawn() builds the QQBotAdapter and starts its async listen loop,
    // eliminating the hardcoded if/elif chain from the startup flow.
    // QQBotConfig is built from gateway config and converted to internal types in factory.
    let mut registry = oben_gateway::platform::PlatformRegistry::new();
    #[allow(unused_mut)]
    let mut platform_handles = {
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
        }

        #[cfg(feature = "telegram")]
        if let Some(ref tg_cfg) = gateway_config.telegram {
            if tg_cfg.enabled {
                let config = oben_config::TelegramConfig {
                    enabled: tg_cfg.enabled,
                    token: tg_cfg.token.clone(),
                    webhook_url: tg_cfg.webhook_url.clone(),
                    webhook_secret: tg_cfg.webhook_secret.clone(),
                    allowed_users: tg_cfg.allowed_users.clone(),
                    allowed_chats: tg_cfg.allowed_chats.clone(),
                    forum_topics: tg_cfg.forum_topics,
                };
                let factory = oben_gateway::platform::TelegramPlatformFactory::new(
                    config,
                    dispatcher.clone(),
                    response_router.clone(),
                );
                registry.register("telegram", "Telegram Bot", factory);
            }
        }

        #[cfg(feature = "discord")]
        if let Some(ref dc_cfg) = gateway_config.discord {
            if dc_cfg.enabled {
                let config = oben_config::DiscordConfig {
                    enabled: dc_cfg.enabled,
                    token: dc_cfg.token.clone(),
                    intents: dc_cfg.intents.clone(),
                    allowed_guilds: dc_cfg.allowed_guilds.clone(),
                    allowed_users: dc_cfg.allowed_users.clone(),
                    slash_commands: dc_cfg.slash_commands,
                    voice: dc_cfg.voice,
                    dm_role_auth_guild: dc_cfg.dm_role_auth_guild.clone(),
                };
                let factory = oben_gateway::platform::DiscordPlatformFactory::new(
                    config,
                    dispatcher.clone(),
                    response_router.clone(),
                );
                registry.register("discord", "Discord Bot", factory);
            }
        }

        #[cfg(feature = "whatsapp")]
        if let Some(ref wa_cfg) = gateway_config.whatsapp {
            if wa_cfg.enabled {
                let config = oben_config::WhatsAppConfig {
                    enabled: wa_cfg.enabled,
                    access_token: wa_cfg.access_token.clone(),
                    phone_number_id: wa_cfg.phone_number_id.clone(),
                    business_account_id: wa_cfg.business_account_id.clone(),
                    webhook_verify_token: wa_cfg.webhook_verify_token.clone(),
                    api_version: wa_cfg.api_version.clone(),
                    allowed_numbers: wa_cfg.allowed_numbers.clone(),
                    default_language: wa_cfg.default_language.clone(),
                };
                let factory = oben_gateway::platform::WhatsAppPlatformFactory::new(
                    config,
                    dispatcher.clone(),
                    response_router.clone(),
                );
                registry.register("whatsapp", "WhatsApp Bot", factory);
            }
        }

        #[cfg(feature = "slack")]
        if let Some(ref sl_cfg) = gateway_config.slack {
            if sl_cfg.enabled {
                let config = oben_config::SlackConfig {
                    enabled: sl_cfg.enabled,
                    app_token: sl_cfg.app_token.clone(),
                    bot_token: sl_cfg.bot_token.clone(),
                    allowed_channels: sl_cfg.allowed_channels.clone(),
                    slash_commands: sl_cfg.slash_commands.clone(),
                };
                let factory = oben_gateway::platform::SlackPlatformFactory::new(
                    config,
                    dispatcher.clone(),
                    response_router.clone(),
                );
                registry.register("slack", "Slack Bot", factory);
            }
        }

        registry.start_all()?
    };
    // Load WASM platform plugins from the configured directory
    #[cfg(feature = "wasm-plugins")]
    {
        use std::path::PathBuf;

        let plugin_dir = gateway_config
            .plugin_dir
            .clone()
            .or_else(|| {
                std::env::var("HOME")
                    .ok()
                    .map(|h| PathBuf::from(h).join(".obenmatrix").join("plugins").join("wasm"))
            });

        if let Some(ref plugin_dir) = plugin_dir {
            if plugin_dir.exists() && plugin_dir.is_dir() {
                tracing::info!(?plugin_dir, "Loading WASM platform plugins");
                if let Ok(entries) = std::fs::read_dir(plugin_dir) {
                    let mut stub_handles: Vec<(String, oben_gateway::platform::PlatformHandle)> =
                        Vec::new();
                    for entry in entries.filter_map(|e| e.ok()) {
                        let path = entry.path();
                        if path.extension().and_then(|e| e.to_str()) != Some("wasm") {
                            continue;
                        }
                        let file_stem = path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("unknown");
                        tracing::info!(plugin = %file_stem, source = ?path, "Found WASM platform plugin");

                        // Full WASM instantiation TBD in a future version.
                        // For now register a stub handle so the gateway lifecycle
                        // tracking includes it.
                        let abort = tokio::spawn(async {
                            loop {
                                tokio::time::sleep(std::time::Duration::from_secs(86_400))
                                    .await;
                            }
                        })
                        .abort_handle();
                        stub_handles.push((
                            format!("wasm_{}", file_stem),
                            oben_gateway::platform::PlatformHandle::new(abort),
                        ));
                    }
                    for (name, handle) in stub_handles {
                        platform_handles.insert(name, handle);
                    }
                }
            }
        }
    }

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
        let hook_engine = Arc::new(
            oben_agent::hooks::HookBuilder::from_config(&oben_config::HooksConfig::default()).build()
        );
        let dispatcher = Arc::new(Dispatcher::new(
            AppConfig::default(),
            Arc::new(ToolRegistry::new()),
            response_router.clone(),
            hook_engine,
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
