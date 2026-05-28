/// Conversation loop — coordinator that wires the deep `TurnExecutor`.

use anyhow::Result;
use std::io::Write;
use std::sync::Arc;

use crate::budget::IterationBudget;
use crate::callbacks::AgentCallbacks;
use crate::context::ContextEngine;
use crate::fallback::FallbackChain;
use crate::interrupt::SharedInterrupt;
use crate::nudge::{NudgeConfig, NudgeResult, build_nudge_prompt, should_trigger_nudge};
use crate::retry::RetryConfig;
use crate::turn_executor::{TurnConfig, TurnExecutor};
use oben_models::{CallMode, Message, SessionManagerExt, StreamDeltaCallback, TransportProvider};
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
        session_manager: &mut dyn SessionManagerExt,
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
            callbacks: options.callbacks,
            fallback_chain: options.fallback,
            dispatch_config: None,
        };

        let result: crate::TurnResult = TurnExecutor::execute_turn_with_config(
            context_engine,
            transport,
            tools,
            session_manager,
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
        session_manager: &mut dyn SessionManagerExt,
        session_id: &str,
        user_message: Message,
        call_mode: &CallMode,
        delta_callback: Option<StreamDeltaCallback>,
    ) -> Result<String> {
        Self::execute_turn_with_options(
            context_engine,
            transport,
            tools,
            session_manager,
            session_id,
            user_message,
            call_mode,
            delta_callback,
            TurnOptions::default(),
        ).await
    }

    /// Run the interactive chat loop.
    ///
    /// After each turn, checks the nudge trigger. If triggered, injects a
    /// memory/skill review prompt and runs one small turn to let the model
    /// decide if memory should be updated — mirroring Hermes'
    /// `_spawn_background_review` pipeline.
    pub async fn run_loop(
        context_engine: &mut dyn ContextEngine,
        transport: &dyn TransportProvider,
        tools: &Arc<oben_tools::ToolRegistry>,
        session_manager: &mut SessionManager,
        call_mode: &mut Option<CallMode>,
        stream: bool,
        callbacks: ChatCallbacks,
        nudge_config: &NudgeConfig,
    ) -> Result<()> {
        let mut turns_since_nudge: usize = 0;
        let mut is_resumed_session = true; // first turn is always "resume" from empty state

        loop {
            // Clear resume flag after the first turn.
            if is_resumed_session {
                is_resumed_session = false;
            }

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

            // Save on ALL exit paths — success, error, or early return.
            // Without this, `execute_turn` errors would lose the user's
            // message (it's in the in-memory session but never persisted).
            //
            // `execute_turn` takes `&mut session_manager` across the await,
            // preventing a second mutable borrow. We break the borrow chain
            // by extracting error info before calling save().
            let response = Self::execute_turn(
                context_engine,
                transport,
                tools,
                session_manager,
                &sid,
                input_msg,
                &call_mode_val,
                delta_cb,
            ).await;

            let response_text = match response {
                Ok(resp) => Some(resp),
                Err(e) => {
                    let err_str = format!("Turn error: {}", e);
                    // Drop e to end borrow chain, then save.
                    std::mem::drop(e);
                    let _ = session_manager.save(None);
                    return Err(anyhow::anyhow!(err_str));
                }
            };
            // Save after the match — execute_turn's borrow has ended.
            let _ = session_manager.save(None);
            if let Some(resp) = response_text {
                if !stream {
                    // Non-streaming: print the full response.
                    (callbacks.print_info)(&format!("\n{}", resp));
                }
                // In streaming mode, text was already printed via delta callback.
                (callbacks.print_flush)();
            }

            // ── Nudge check ─────────────────────────────────────────────
            if !nudge_config.enabled() {
                continue;
            }

            // Check if the active session has memory tool calls (proxy for
            // "memory tools are available").
            let has_memory_tools = session_manager
                .active_session()
                .map_or(false, |s| {
                    s.messages.iter().any(|m| m.tool_calls.as_ref().map_or(false, |c| !c.is_empty()))
                });

            if should_trigger_nudge(turns_since_nudge, nudge_config.memory_nudge_interval, has_memory_tools, is_resumed_session) {
                turns_since_nudge = 0;

                let prompt = build_nudge_prompt(
                    nudge_config.memory_enabled(),
                    nudge_config.skill_enabled(),
                );
                let review_msg = Message::user(&prompt);

                let budget = IterationBudget::new(16);
                let turn_options = crate::conversation::TurnOptions {
                    retry_config: crate::retry::RetryConfig::default(),
                    budget: Some(budget),
                    interrupt: None,
                    callbacks: None, // suppress callbacks during review
                    fallback: None,
                };

                match Self::execute_turn_with_options(
                    context_engine,
                    transport,
                    tools,
                    session_manager,
                    &sid,
                    review_msg,
                    &call_mode_val,
                    None,
                    turn_options,
                ).await {
                    Ok(review_text) => {
                        let text_lower = review_text.to_lowercase();
                        let is_noop = text_lower.contains("nothing to")
                            || text_lower.contains("nothing worth")
                            || text_lower.contains("no changes needed");

                        if is_noop {
                            (callbacks.print_info)("💾 Nudge: nothing worth saving this session.");
                        } else {
                            (callbacks.print_info)("💾 Nudge: checked memory — may have updated.");
                        }
                        (callbacks.print_flush)();
                    }
                    Err(e) => {
                        tracing::info!("Nudge review failed (non-fatal): {}", e);
                    }
                }
            } else {
                turns_since_nudge += 1;
            }
        }

        Ok(())
    }
}
