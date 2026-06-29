//! Top-level entry point for the multi-platform messaging gateway.
//!
//! Connects platform adapters (QQ Bot, Telegram, Discord, Slack, WhatsApp) to a
//! central Dispatcher, which routes inbound messages to per-user Agent conversation
//! sessions. Agent responses flow back through a ResponseRouter to the correct
//! platform.
//!
//! ## Usage
//!
//! ```bash
//! # Build and run the gateway process
//! cargo run --package oben-gateway
//! ```
//!
//! The gateway reads its configuration from `~/.config/obenalien/config.yaml`.
//! To enable a platform, add its config under `gateway` in the YAML file.
//!
//! ## Architecture
//!
//! ```
//! External platforms (QQ, Telegram, Discord, Slack, WhatsApp)
//!         │
//!         ▼
//!   Platform adapters (QQBotAdapter, etc.)
//!         │ listen() — receives WebSocket/HTTP events
//!         ▼
//!   event_to_incoming() — converts raw events → IncomingMessage
//!         │
//!         ▼
//!   Dispatcher::dispatch(msg)
//!         ├── session_key = platform/user_id/thread_id
//!         ├── If session exists → send through existing channel
//!         └── If new → spawn_coordinator_task()
//!                       ├── GatewayCoordinator
//!                       ├── Agent::new(config, system_prompt, tools)
//!                       └── Agent::run(coordinator)
//!                               │
//!                               ▼
//!                        Agent processes message (LLM reasoning, tools)
//!                               │
//!                               ▼
//!                        Coordinator.on_turn_complete(response)
//!                               │ sends ResponseMessage to channel
//!                               ▼
//!                          Dispatcher consumes response
//!                               │
//!                               ▼
//!                          ResponseRouter.send(name, msg)
//!                               │ looks up adapter by platform name
//!                               ▼
//!                          QQBotAdapter.send(OutgoingMessage)
//!                               │ calls QQ API REST endpoint
//!                               ▼
//!                          User receives reply
//! ```

use std::sync::Arc;

use anyhow::Result;
use tracing::info;

use oben_gateway::{
    Dispatcher, Gateway, PlatformAdapter, QQBotAdapter, ResponseRouter,
};
use oben_config::{AppConfig, QQBotIntent};
use oben_sessions::DBSessionManager;
use oben_tools::ToolRegistry;

/// Directory where gateway logs are persisted, so daemonized child processes
/// (stdout/stderr are disconnected by `daemonize`) retain a full log history.
/// Using absolute path to work regardless of working directory.
pub const LOG_DIR: &str = ".config/obenalien/logs";
pub const LOG_FILE_PREFIX: &str = "gateway";

/// Initialize tracing with level-based filtering AND a rolling log file.
fn init_logging() {
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

    // Console layer — visible when run manually (not daemonized).
    let fmt_layer = tracing_subscriber::fmt::layer().with_target(false);

    // Rolling log file — always written, survives restarts.
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

    info!(log_dir = %log_dir.display(), "Logging initialized — logs written to file");
}

/// Parse config QQBotIntent values into the GG protocol Intents bitflags.
fn parse_qq_intents(intents: &[QQBotIntent]) -> oben_gateway::Intents {
    let result = oben_gateway::Intents::new()
        .with_guilds()
        .with_group_and_c2c();
    for intent in intents {
        match intent {
            QQBotIntent::Guilds => { /* already included by default */ }
            QQBotIntent::C2cMessage => { /* already included */ }
            QQBotIntent::GroupAtMessage => { /* already included */ }
        }
    }
    result
}

/// Create a ToolRegistry populated with all built-in tools.
fn create_tool_registry() -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    oben_tools::discover_builtin_tools(&mut registry);
    registry
}

#[tokio::main]
async fn main() -> Result<()> {
    init_logging();
    info!("=== Oben Gateway Starting ===");

    // Load application config
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

    // Create session manager
    let session_manager = DBSessionManager::new()?;
    info!("Session manager initialized");

    // Create response router
    let response_router = Arc::new(ResponseRouter::new());

    // Create tool registry
    let tools = Arc::new(create_tool_registry());
    info!("Tool registry created");

    // Create dispatcher
    let dispatcher = Arc::new(Dispatcher::new(
        app_config.clone(),
        tools,
        response_router.clone(),
    ));
    info!("Dispatcher created");

    // Create and register platform adapters
    if let Some(ref qq_config) = gateway_config.qq_bot {
        if qq_config.enabled {
            info!(
                app_id = %qq_config.app_id,
                sandbox = qq_config.sandbox,
                "QQ Bot config found — creating adapter"
            );

            let intents = parse_qq_intents(&qq_config.intents);

            let adapter: Box<dyn PlatformAdapter + Send + Sync> = Box::new(QQBotAdapter::new(
                &qq_config.app_id,
                &qq_config.app_secret,
                qq_config.sandbox,
                qq_config.shard,
                intents,
                dispatcher.clone(),
            ));

            // Register with response router (takes ownership of the Box)
            response_router.register("qq_bot", adapter).await;

            info!("QQ Bot adapter registered with response router");
        }
    }

    // Start the gateway (async, blocks until Ctrl+C via tokio::signal::ctrl_c)
    info!("Gateway initialized — calling start_blocking()");
    info!("Press Ctrl+C to shut down");

    let gateway = Gateway::new(session_manager, gateway_config, dispatcher);
    gateway.start_blocking().await?;

    info!("Gateway shut down cleanly");
    Ok(())
}
