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

use anyhow::{anyhow, Result};
use futures::future::join_all;
use tracing::{error, info, warn};

use oben_config::{AppConfig, QQBotIntent};
use oben_gateway::{Dispatcher, Gateway, QQBotAdapter, ResponseRouter};
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
            QQBotIntent::DirectMessage => { /* already included by default */ }
            QQBotIntent::C2CAndGroup => { /* already included */ }
            QQBotIntent::Interaction => { /* already included */ }
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

/// Discover enabled platform adapters from config and attempt to create them concurrently.
///
/// Runs Telegram, Discord, Slack, WhatsApp, and QQ Bot creation tasks in parallel.
/// Failed adapters are logged but don't prevent others from starting.
/// On success, registers the adapter with the response router.
async fn discover_platforms(
    gateway_config: &oben_config::GatewayConfig,
    _dispatcher: Arc<oben_gateway::Dispatcher>,
    _response_router: Arc<oben_gateway::ResponseRouter>,
) -> Vec<(String, anyhow::Result<()>)> {
    let mut tasks = Vec::new();

    // Telegram — adapter not yet implemented
    if let Some(ref tg_cfg) = gateway_config.telegram {
        if tg_cfg.enabled {
            tasks.push(tokio::spawn(async move {
                warn!("Telegram platform enabled but adapter not yet implemented");
                ("telegram".to_string(), Err(anyhow!("Telegram adapter not implemented")))
            }));
        }
    }

    // Discord — adapter not yet implemented
    if let Some(ref dc_cfg) = gateway_config.discord {
        if dc_cfg.enabled {
            tasks.push(tokio::spawn(async move {
                warn!("Discord platform enabled but adapter not yet implemented");
                ("discord".to_string(), Err(anyhow!("Discord adapter not implemented")))
            }));
        }
    }

    // Slack — adapter not yet implemented
    if let Some(ref sl_cfg) = gateway_config.slack {
        if sl_cfg.enabled {
            tasks.push(tokio::spawn(async move {
                warn!("Slack platform enabled but adapter not yet implemented");
                ("slack".to_string(), Err(anyhow!("Slack adapter not implemented")))
            }));
        }
    }

    // WhatsApp — adapter not yet implemented
    if let Some(ref wa_cfg) = gateway_config.whatsapp {
        if wa_cfg.enabled {
            tasks.push(tokio::spawn(async move {
                warn!("WhatsApp platform enabled but adapter not yet implemented");
                ("whatsapp".to_string(), Err(anyhow!("WhatsApp adapter not implemented")))
            }));
        }
    }

    // Run all discovery tasks concurrently; empty task list yields empty result
    let discovered = join_all(tasks).await;

    // Collect results, handling task panics gracefully
    let mut results = Vec::new();
    for discovery in discovered {
        match discovery {
            Ok((name, result)) => results.push((name, result)),
            Err(join_err) => {
                error!("Platform discovery task panicked: {join_err}");
                results.push(("unknown".to_string(), Err(anyhow::anyhow!("Task panicked: {join_err}"))));
            }
        }
    }

    results
}

#[tokio::main]
async fn main() -> Result<()> {
    // Install ring crypto provider BEFORE any other code runs.
    // hyper-rustls (via reqwest) has its own direct rustls dependency with aws-lc-rs.
    // Since Cargo merges both ring and aws-lc-rs into one binary, rustls 0.23+ would
    // panic on auto-detection. Explicitly installing ring avoids this.
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install ring crypto provider");

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

    // Discover enabled platforms (Telegram, Discord, Slack, WhatsApp placeholders).
    let platforms_enabled = discover_platforms(&gateway_config, dispatcher.clone(), response_router.clone()).await;

    // QQ Bot — create and register synchronously (NOT in spawn_blocking).
    // QQBotAdapter::new() needs tokio runtime context for WebSocket connections,
    // so it must be called on the main async task, not in spawn_blocking.
    if let Some(ref qq_cfg) = gateway_config.qq_bot {
        if qq_cfg.enabled {
            let intents = parse_qq_intents(&qq_cfg.intents);
            let adapter = QQBotAdapter::new(
                &qq_cfg.app_id,
                &qq_cfg.app_secret,
                qq_cfg.sandbox,
                qq_cfg.shard,
                intents,
                dispatcher.clone(),
            );
            response_router.register("qq_bot", Box::new(adapter)).await;
            info!("QQ Bot adapter created and registered");
        }
    }

    info!(
        platforms_found = platforms_enabled.len(),
        "Platform discovery complete"
    );

    // Start the gateway (async, blocks until Ctrl+C via tokio::signal::ctrl_c)
    info!("Gateway initialized — calling start_blocking()");
    info!("Press Ctrl+C to shut down");

    let gateway = Gateway::new(session_manager, gateway_config, dispatcher);
    gateway.start_blocking().await?;

    info!("Gateway shut down cleanly");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Given: GatewayConfig with no platforms enabled (all None/default)
    /// When: discover_platforms is called with empty config
    /// Then: Returns empty results with no adapters registered, no panics
    #[tokio::test]
    async fn test_discover_platforms_empty() {
        let empty_config =oben_config::GatewayConfig {
            telegram: None,
            discord: None,
            slack: None,
            whatsapp: None,
            qq_bot: None,
        };
        let response_router = Arc::new(super::ResponseRouter::new());
        let dispatcher = Arc::new(super::Dispatcher::new(
            oben_config::AppConfig::default(),
            Arc::new(ToolRegistry::new()),
            response_router.clone(),
        ));

        let results = discover_platforms(&empty_config, dispatcher, response_router).await;

        assert!(results.is_empty(), "Expected no platforms with empty config");
    }
}
