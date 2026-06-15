/// The main conversation loop that all modes implement.
///
/// Shared infrastructure only — concrete coordinators live in `oben-cli`
/// (`CliCoordinator`) and `oben-tui` (`TuiCoordinator`).
///
/// **Does NOT own resources.** All runtime resources (ContextEngine, Transport,
/// Tools, SessionManager) are passed as parameters. The coordinator only
/// orchestrates. Hook engine and termination policy are owned by the concrete
/// coordinator since they define its behavior.
use std::pin::Pin;
use std::sync::Arc;

use anyhow::Result;

use crate::budget::IterationBudget;
use crate::hooks::HookEngine;
use crate::context::ContextEngine;
use crate::interrupt::SharedInterrupt;
use crate::turn_executor::TurnExecutor;
use oben_models::{SessionManager, TransportProvider};
use oben_tools::ToolRegistry;

/// Shared config for both CLI and TUI conversation loops.
pub mod config;
pub use config::*;

/// Pluggable termination policies used by concrete coordinators.
pub mod termination;

/// Subagent tree management and interrupt propagation hub.
///
/// The coordinator uses `InterruptHub` to track direct children (spawned via
/// the delegate tool) and propagate interrupts in DFS order from deepest
/// nodes first (leaf subagents → their parents → root).
pub mod tree;

// -- Shared turn execution utilities --

/// Execute a single turn.
///
/// Free function used by both `CliCoordinator` and `TuiCoordinator`.
/// This avoids duplicating the `TurnExecutor` call signature.
pub async fn execute_turn(
    context_engine: &mut dyn ContextEngine,
    transport: &dyn TransportProvider,
    tools: &Arc<oben_tools::ToolRegistry>,
    session_manager: &mut dyn SessionManager,
    session_id: &str,
    user_message: oben_models::Message,
    call_mode: &oben_models::CallMode,
    conversation_config: &ConversationConfig,
) -> Result<String> {
    let turn_config = crate::turn_executor::TurnConfig {
        retry_config: conversation_config.retry_config.clone(),
        budget_warning: None,
        hooks: None,
        fallback_chain: if conversation_config.fallback_configs.is_empty() {
            None
        } else {
            Some(crate::fallback::FallbackChain::new(
                conversation_config
                    .fallback_configs
                    .iter()
                    .map(|fb| crate::fallback::FallbackConfig {
                        provider: fb.provider.clone(),
                        model: fb.model.clone(),
                        api_key: fb.api_key.clone(),
                        base_url: fb.base_url.clone(),
                    })
                    .collect(),
            ))
        },
        dispatch_config: conversation_config.dispatch_config.clone(),
    };

    let result = TurnExecutor::execute_turn_with_config(
        context_engine,
        transport,
        tools,
        session_manager,
        session_id,
        user_message,
        call_mode,
        Some(IterationBudget::new(conversation_config.max_iterations)),
        None,
        turn_config,
    )
    .await?;

    Ok(result.text)
}

/// Execute a single turn with full agent integration (hooks, interrupt).
///
/// Used by `Agent::turn()`, `Agent::turn_with_message()`, and
/// `Agent::trigger_nudge()` for the callback/interrupt features.
pub async fn execute_turn_full(
    context_engine: &mut dyn ContextEngine,
    transport: &dyn TransportProvider,
    tools: &Arc<oben_tools::ToolRegistry>,
    session_manager: &mut dyn SessionManager,
    session_id: &str,
    user_message: oben_models::Message,
    call_mode: &oben_models::CallMode,
    conversation_config: &ConversationConfig,
    hooks: Option<Arc<HookEngine>>,
    interrupt: Option<SharedInterrupt>,
) -> Result<String> {
    let turn_config = crate::turn_executor::TurnConfig {
        retry_config: conversation_config.retry_config.clone(),
        budget_warning: None,
        hooks,
        fallback_chain: if conversation_config.fallback_configs.is_empty() {
            None
        } else {
            Some(crate::fallback::FallbackChain::new(
                conversation_config
                    .fallback_configs
                    .iter()
                    .map(|fb| crate::fallback::FallbackConfig {
                        provider: fb.provider.clone(),
                        model: fb.model.clone(),
                        api_key: fb.api_key.clone(),
                        base_url: fb.base_url.clone(),
                    })
                    .collect(),
            ))
        },
        dispatch_config: conversation_config.dispatch_config.clone(),
    };

    let result = TurnExecutor::execute_turn_with_config(
        context_engine,
        transport,
        tools,
        session_manager,
        session_id,
        user_message,
        call_mode,
        Some(IterationBudget::new(conversation_config.max_iterations)),
        interrupt,
        turn_config,
    )
    .await?;

    Ok(result.text)
}

/// Result of a conversation loop.
#[derive(Debug, Clone, PartialEq)]
pub enum ConversationResult {
    /// User chose to exit normally.
    Exit,
    /// Loop ended due to budget/iteration limit.
    BudgetExhausted,
    /// Loop ended due to external stop signal (Ctrl+C, disconnect).
    Interrupted,
    /// Loop ended due to a goal being satisfied (for goal coordinator).
    GoalDone,
    /// Loop ended due to an unrecoverable error.
    Error(String),
}

/// The conversation coordinator — orchestrates the full multi-turn loop.
///
/// The coordinator is the ONLY place where:
/// - Turn loop control flow lives (NOT in `TurnExecutor`, which does exactly ONE turn)
/// - Subagent tree management lives (InterruptHub owned by coordinator)
/// - Termination policy evaluation lives
/// - Hook engine integration lives
/// - I/O provider creation lives (e.g. `StdioProvider` for CLI, custom for TUI)
///
/// The agent passes its resources to the coordinator — the coordinator owns
/// its own I/O and execution model.
#[async_trait::async_trait]
pub trait ConversationCoordinator: Send + Sync {
    /// Run the conversation loop to completion.
    ///
    /// Each coordinator creates its own `InteractionProvider` internally
    /// (e.g. `StdioProvider` for CLI, custom event loop for TUI).
    async fn run(
        &mut self,
        context_engine: &mut dyn ContextEngine,
        transport: Arc<dyn TransportProvider + Send + Sync>,
        tools: Arc<ToolRegistry>,
        session_manager: &mut dyn SessionManager,
    ) -> Result<ConversationResult>;

    /// Send a message through the coordinator.
    /// Used by external interfaces (TUI) to inject messages into the loop.
    fn send_message(
        &self,
        _text: String,
    ) -> Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        Box::pin(async {})
    }

    /// Request interrupt propagation.
    ///
    /// For coordinators with subagent trees, this propagates the interrupt
    /// DFS from deepest nodes first (leaf subagents → their parents → root).
    fn request_interrupt(
        &self,
        _message: Option<String>,
    ) {
    }
}
