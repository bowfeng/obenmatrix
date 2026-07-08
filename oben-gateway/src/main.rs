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

/// Load WASM plugins and inject hook adapters into a HookBuilder.
///
/// Flow (under `wasm-plugins` feature):
/// 1. Resolve plugin directory (config > env > default).
/// 2. Discover plugins via PluginDiscoverer (manifest parsing).
/// 3. Filter by PluginConfig::is_enabled / disabled lists.
/// 4. Create PluginLifecycleManager for crash tracking.
/// 5. Create WasmRuntime + PluginLoader to load plugins into bundles.
/// 6. Create separate WasmHookRegistry to load hook adapters.
/// 7. Wire adapters into the HookBuilder for the agent loop.
///
/// Returns the builder unchanged when no plugins are found or the
/// feature is disabled (the call-site cfg-gates the whole call).
///
/// NOTE: PluginLoader and WasmHookRegistry each own their own WasmRuntime
/// instance. The loader uses PluginDiscoverer for manifest-based discovery
/// (finding .platform.json / plugin.yaml), while the registry uses
/// discover_plugins() for raw .wasm-file scanning. Lifecycle tracking
/// covers all discovered plugins regardless of which system loaded them.
#[cfg(feature = "wasm-plugins")]
async fn load_wasm_hooks(
    builder: HookBuilder,
    plugins_dir: &Option<std::path::PathBuf>,
) -> HookBuilder {
    use std::path::PathBuf;

    use oben_wasm::lifecycle::PluginLifecycleManager;
    use oben_wasm::{
        PluginDiscoverer, PluginLoader, WasmHookRegistry,
        WasmRuntime, WasmRuntimeConfig,
    };

    // 1. Resolve plugin directory
    let plugin_path = plugins_dir.clone().or_else(|| {
        std::env::var("HOME")
            .ok()
            .map(|h| PathBuf::from(h).join(".obenmatrix").join("plugins").join("wasm"))
    });

    let Some(pdir) = plugin_path.as_ref()
        .filter(|p| p.exists() && p.is_dir())
    else {
        return builder;
    };

    tracing::info!(?pdir, "WASM plugin loading enabled");

    // 2. Discover plugins via manifest files (.platform.json / plugin.yaml)
    let discovered = match PluginDiscoverer::discover(pdir.as_path()) {
        Ok(d) if !d.is_empty() => d,
        Ok(_) => {
            tracing::info!(?pdir, "No plugins discovered, skipping WASM loading");
            return builder;
        }
        Err(e) => {
            tracing::warn!(error = %e, ?pdir, "WASM plugin discovery failed");
            return builder;
        }
    };

    // 3. Filter by PluginConfig (enabled/disabled lists)
    let plugin_config = oben_config::PluginConfig::default();
    let enabled_list: Vec<_> = discovered.iter()
        .filter(|d| plugin_config.is_enabled(&d.manifest.name))
        .collect();

    if enabled_list.is_empty() {
        tracing::info!(count = discovered.len(), "All discovered plugins are disabled");
        return builder;
    }

    tracing::info!(count = enabled_list.len(), "Enabled plugins after filtering");

    // 4. Create lifecycle manager (max 3 crash restarts per plugin)
    let mut lifecycle = PluginLifecycleManager::new(3);

    // 5a. Create runtime and loader (for bundle loading + caching)
    let loader_runtime = match WasmRuntime::new(WasmRuntimeConfig::default()) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "WASM runtime creation failed");
            return builder;
        }
    };
    let loader = PluginLoader::new(loader_runtime);

    // 5b. Load plugins into bundles (populates the loader's internal component cache)
    let loader_results = loader.load_plugins(pdir.as_path()).await;

    // Track bundle load errors at lifecycle level
    for bundle in &loader_results.bundles {
        let plugin_name: String = match bundle.tools.first() {
            Some(t) => t.name.clone(),
            None => "unknown".to_string(),
        };
        if !bundle.errors.is_empty() {
            lifecycle.crash(&plugin_name, &format!("bundle load errors: {:?}", bundle.errors));
            tracing::warn!(plugin = %plugin_name, errors = ?bundle.errors, "Plugin bundle has load errors");
        }
    }

    // 5c. (Alternative) Use loader's `discover` instead of PluginDiscoverer
    // let loader_discovered = PluginLoader::discover(pdir.as_path())?;
    // Filter by plugin_config.is_enabled same as above.

    // 6. Create separate hook registry (uses its own runtime)
    let hook_runtime = match WasmRuntime::new(WasmRuntimeConfig::default()) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "WASM hook runtime creation failed");
            return builder;
        }
    };

    let registry = WasmHookRegistry::new(hook_runtime, pdir.clone());

    // Build HookBuilder incrementally across all successfully loaded plugins
    let mut hook_builder = builder;
    let mut loaded_count = 0usize;
    let mut hook_plugin_names = Vec::new();

    // Discover hook candidates via registry's raw WASM scanning
    match registry.discover_plugins().await {
        Ok(wasm_paths) => {
            for wasm_path in &wasm_paths {
                let stem = wasm_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .map(String::from)
                    .unwrap_or_else(|| "unknown".to_string());

                // Skip plugins not in our enabled list
                if !enabled_list.iter().any(|d| d.manifest.name == stem) {
                    tracing::debug!(
                        plugin = %stem,
                        "Skipping plugin not in enabled list",
                    );
                    continue;
                }

                lifecycle.start(&stem);

                match registry.load_hooks().await {
                    Ok(components) => {
                        if components.is_empty() {
                            lifecycle.started(&stem);
                            continue;
                        }

                        match registry.instantiate_hooks(components).await {
                            Ok(hook_components) => {
                                hook_builder = hook_builder
                                    .with_agent_loop_hooks(hook_components.agent_loop)
                                    .with_turn_hooks(hook_components.turn)
                                    .with_tool_hooks(hook_components.tool)
                                    .with_streaming_hooks(hook_components.streaming)
                                    .with_system_hooks(hook_components.system)
                                    .with_session_hooks(hook_components.session)
                                    .with_interrupt_hooks(hook_components.interrupt);
                                loaded_count += 1;
                                hook_plugin_names.push(stem.clone());
                            },
                            Err(e) => {
                                lifecycle.crash(&stem, &format!("instantiate hooks failed: {e}"));
                                tracing::warn!(
                                    plugin = %stem, error = %e,
                                    "Instantiating WASM hook adapters failed"
                                );
                            }
                        }
                    },
                    Err(e) => {
                        lifecycle.crash(&stem, &format!("load hooks failed: {e}"));
                        tracing::warn!(plugin = %stem, error = %e, "Loading WASM hook component failed");
                    }
                }

                lifecycle.started(&stem);
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "WASM hook plugin discovery failed");
        }
    }

    // 7. Save lifecycle state for crash detection / shutdown
    let running = lifecycle.running_plugins();
    tracing::info!(
        loaded = loaded_count,
        hook_plugins = ?hook_plugin_names,
        plugins = ?running,
        "WASM plugin lifecycle state committed"
    );

    hook_builder
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
