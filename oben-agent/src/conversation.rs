/// Conversation loop — coordinator that wires the deep `TurnExecutor`.
///
/// **Responsibilities:**
/// - Interactive chat loop (prompt → input → execute → output)
/// - Call mode management (Fresh → Incremental)
/// - Preflight check before each turn
/// - Delegate to TurnExecutor for actual turn cycle
///
/// ConversationLoop does NOT own SessionManager, ContextEngine, Budget,
/// Transport, or Tools — those are Agent's responsibilities.

use anyhow::Result;
use std::io::Write;
use std::sync::{Arc, Mutex};

use crate::context::ContextEngine;
use crate::turn_executor::TurnExecutor;
use oben_models::{CallMode, Message, SessionStore, TransportProvider};
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

/// The conversation coordinator — wires the deep `TurnExecutor`.
///
/// **Thin coordinator layer only.** The actual turn logic lives in
/// `TurnExecutor` (deep module). This layer provides:
/// - Interactive chat loop
/// - Call mode management (Fresh → Incremental)
/// - Preflight compression check
/// - Delegation to `TurnExecutor` for the core turn cycle
///
/// ConversationLoop does NOT own SessionManager, ContextEngine, Budget,
/// Transport, or Tools — that's Agent's job. All resources are passed
/// as function parameters.
pub struct ConversationLoop;

impl ConversationLoop {
    /// Execute one turn — wraps preflight + execute_turn.
    ///
    /// All resources passed as parameters — ConversationLoop owns nothing.
    pub async fn execute_turn(
        context_engine: &Arc<Mutex<Box<dyn ContextEngine>>>,
        transport: &dyn TransportProvider,
        tools: &Arc<oben_tools::ToolRegistry>,
        store: &mut dyn SessionStore,
        session_id: &str,
        user_message: Message,
        call_mode: &CallMode,
        delta_callback: Option<oben_models::StreamDeltaCallback>,
    ) -> Result<String> {
        // Phase 1: preflight — compress if needed
        {
            let session = store.session_mut(session_id)
                .ok_or_else(|| anyhow::anyhow!("Session not found: {}", session_id))?;
            context_engine.lock().unwrap().preflight_check(
                &mut session.messages, Some(transport), None
            ).await?;
        }

        // Phase 2: execute turn
        let result = TurnExecutor::execute_turn(
            context_engine,
            transport,
            tools,
            store,
            session_id,
            user_message,
            call_mode,
            delta_callback,
        ).await?;

        Ok(result.text)
    }

    /// Run the interactive chat loop.
    ///
    /// **Ownership**: Call mode (Fresh → Incremental) is tracked in this
    /// method. Agent owns session state; ConversationLoop owns the loop
    /// and delegates turns to TurnExecutor.
    pub async fn run_loop(
        context_engine: &Arc<Mutex<Box<dyn ContextEngine>>>,
        transport: &dyn TransportProvider,
        tools: &Arc<oben_tools::ToolRegistry>,
        session_manager: &mut SessionManager,
        call_mode: &mut Mutex<Option<CallMode>>,
        stream: bool,
        callbacks: ChatCallbacks,
    ) -> Result<()> {
        // Core loop
        loop {
            (callbacks.print_prompt)();
            (callbacks.print_flush)();

            let input = match (callbacks.read_input)() {
                Some(line) if !line.trim().is_empty() => line.trim().to_string(),
                _ => continue,
            };

            if (callbacks.should_exit)(&input) { break; }

            // Resolve session — get or create
            let sid = session_manager
                .active_session()
                .map(|s| s.id.clone())
                .unwrap_or_else(|| {
                    // No active session — create one
                    let id = format!("chat-{}", chrono::Utc::now().format("%Y%m%d-%H%M%S"));
                    session_manager.new_session(&id);
                    session_manager.active_session().unwrap().id.clone()
                });

            // Update call mode: Fresh on first turn, Incremental after
            let call_mode_val = {
                let mut guard = call_mode.lock().unwrap();
                match guard.as_ref() {
                    Some(CallMode::Fresh(_)) => {
                        *guard = Some(CallMode::Incremental(sid.clone()));
                        CallMode::Fresh(sid.clone())
                    }
                    Some(CallMode::Incremental(_)) => guard.as_ref().unwrap().clone(),
                    None => {
                        let mode = CallMode::Fresh(sid.clone());
                        *guard = Some(mode.clone());
                        mode
                    }
                }
            };

            let input_msg = Message::user(&input);

            // Execute turn (preflight + execute in one borrow)
            let response = Self::execute_turn(
                context_engine,
                transport,
                tools,
                session_manager, // SessionManager implements SessionStore
                &sid,
                input_msg,
                &call_mode_val,
                if stream {
                    let cb = callbacks.clone();
                    Some(Box::new(move |text: &str| {
                        print!("{}", text);
                        (cb.print_flush)();
                    }))
                } else {
                    None
                },
            ).await?;

            // Output
            if stream {
                (callbacks.print_newline)();
            } else {
                (callbacks.print_info)(&format!("\n{}", response));
                (callbacks.print_flush)();
            }

            // Persist session after turn
            session_manager.save(None)?;
        }

        Ok(())
    }
}
