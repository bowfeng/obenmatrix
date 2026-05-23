/// Conversation loop — coordinator that wires the deep `TurnExecutor`.
///
/// **Responsibilities:**
/// - Interactive chat loop (prompt → input → execute → output)
/// - Call mode management (Fresh → Incremental)
/// - Preflight check before each turn
/// - Delegate to TurnExecutor for actual turn cycle
/// - Rich callback dispatch (Tier 2)
/// - Fallback model integration (Tier 2)

use anyhow::Result;
use std::io::Write;
use std::sync::Arc;

use crate::budget::IterationBudget;
use crate::callbacks::AgentCallbacks;
use crate::context::ContextEngine;
use crate::fallback::FallbackChain;
use crate::interrupt::{SharedInterrupt, shared_interrupt};
use crate::retry::RetryConfig;
use crate::turn_executor::{TurnConfig, TurnExecutor};
use oben_models::{CallMode, Message, SessionStore, StreamDeltaCallback, TransportProvider};
use oben_sessions::SessionManager;

/// Callbacks for interactive_chat — abstracts I/O for CLI/TUI.
#[derive(Clone)]
pub struct ChatCallbacks {
    pub print_info: fn(&str),
    pub print_prompt: fn(),
    pub print_flush: fn(),
    pub read_input: fn() -> Option<String>,
    pub print_newline: fn(),
    pub should_exit: fn(&str) -> bool,
}

impl ChatCallbacks {
    pub fn for_cli() -> Self {
        Self {
            print_info: |msg: &str| println!("{}", msg),
            print_prompt: || print!("> "),
            print_flush: || { let _ = std::io::stdout().flush(); },
            read_input: || {
                let mut input = String::new();
                if std::io::stdin().read_line(&mut input).is_ok() {
                    Some(input.trim().to_string())
                } else {
                    Some(String::new())
                }
            },
            print_newline: || println!(),
            should_exit: |input: &str| input == "quit" || input == "exit",
        }
    }
}

/// Configuration for a turn execution in ConversationLoop.
pub struct TurnOptions {
    pub retry_config: RetryConfig,
    pub budget: Option<IterationBudget>,
    pub interrupt: Option<SharedInterrupt>,
    pub callbacks: Option<AgentCallbacks>,
    pub fallback: Option<FallbackChain>,
}

impl Default for TurnOptions {
    fn default() -> Self {
        Self {
            retry_config: RetryConfig::default(),
            budget: None,
            interrupt: None,
            callbacks: None,
            fallback: None,
        }
    }
}

/// The conversation coordinator — wires the deep `TurnExecutor`.
pub struct ConversationLoop;

impl ConversationLoop {
    /// Execute one turn with full Tier 1 + Tier 2 features.
    pub async fn execute_turn_with_options(
        context_engine: &mut dyn ContextEngine,
        transport: &dyn TransportProvider,
        tools: &Arc<oben_tools::ToolRegistry>,
        store: &mut dyn SessionStore,
        session_id: &str,
        user_message: Message,
        call_mode: &CallMode,
        delta_callback: Option<StreamDeltaCallback>,
        options: TurnOptions,
    ) -> Result<String> {
        let TurnOptions {
            retry_config,
            budget,
            interrupt,
            ..
        } = options;

        let turn_config = TurnConfig {
            retry_config,
            budget_warning: None,
        };

        let result = TurnExecutor::execute_turn_with_config(
            context_engine,
            transport,
            tools,
            store,
            session_id,
            user_message,
            call_mode,
            delta_callback,
            budget,
            interrupt,
            turn_config,
        ).await?;

        Ok(result.text)
    }

    /// Execute one turn — wraps preflight + execute_turn.
    pub async fn execute_turn(
        context_engine: &mut dyn ContextEngine,
        transport: &dyn TransportProvider,
        tools: &std::sync::Arc<oben_tools::ToolRegistry>,
        store: &mut dyn SessionStore,
        session_id: &str,
        user_message: Message,
        call_mode: &CallMode,
        delta_callback: Option<StreamDeltaCallback>,
    ) -> Result<String> {
        Self::execute_turn_with_options(
            context_engine,
            transport,
            tools,
            store,
            session_id,
            user_message,
            call_mode,
            delta_callback,
            TurnOptions::default(),
        ).await
    }

    /// Run the interactive chat loop.
    pub async fn run_loop(
        context_engine: &mut dyn ContextEngine,
        transport: &dyn TransportProvider,
        tools: &Arc<oben_tools::ToolRegistry>,
        session_manager: &mut SessionManager,
        call_mode: &mut Option<CallMode>,
        stream: bool,
        callbacks: ChatCallbacks,
    ) -> Result<()> {
        loop {
            (callbacks.print_prompt)();
            (callbacks.print_flush)();

            let input = match (callbacks.read_input)() {
                Some(line) if !line.trim().is_empty() => line.trim().to_string(),
                _ => continue,
            };

            if (callbacks.should_exit)(&input) { break; }

            let sid = session_manager
                .active_session()
                .map(|s| s.id.clone())
                .unwrap_or_else(|| {
                    let id = format!("chat-{}", chrono::Utc::now().format("%Y%m%d-%H%M%S"));
                    session_manager.new_session(&id);
                    session_manager.active_session().unwrap().id.clone()
                });

            let call_mode_val = match call_mode {
                Some(CallMode::Fresh(_)) => {
                    *call_mode = Some(CallMode::Incremental(sid.clone()));
                    CallMode::Fresh(sid.clone())
                }
                Some(CallMode::Incremental(_)) => call_mode.as_ref().unwrap().clone(),
                None => {
                    let mode = CallMode::Fresh(sid.clone());
                    *call_mode = Some(mode.clone());
                    mode
                }
            };

            let input_msg = Message::user(&input);

            let delta_cb = if stream {
                let cb = callbacks.clone();
                let f: StreamDeltaCallback = Box::new(move |text: &str| {
                    let _ = write!(std::io::stdout(), "{}", text);
                    (cb.print_flush)();
                });
                Some(f)
            } else {
                None
            };

            let response = Self::execute_turn(
                context_engine,
                transport,
                tools,
                session_manager,
                &sid,
                input_msg,
                &call_mode_val,
                delta_cb,
            ).await?;

            if stream {
                (callbacks.print_newline)();
            } else {
                (callbacks.print_info)(&format!("\n{}", response));
                (callbacks.print_flush)();
            }

            session_manager.save(None)?;
        }

        Ok(())
    }
}
