/// Conversation loop — coordinator that wires the deep `TurnExecutor`.
use anyhow::Result;
use std::io::Write;
use std::sync::Arc;

use crate::budget::IterationBudget;
use crate::callbacks::AgentCallbacks;
use crate::context::ContextEngine;
use crate::fallback::FallbackChain;
use crate::interrupt::SharedInterrupt;
use crate::post_turn_hook::PostTurnHook;
use crate::retry::RetryConfig;
use crate::turn_executor::{TurnConfig, TurnExecutor};
use oben_models::{CallMode, Message, SessionManager, StreamDeltaCallback, TransportProvider};

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
        session_manager: &mut dyn SessionManager,
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
        )
        .await?;

        Ok(result.text)
    }

    /// Execute one turn — wraps preflight + execute_turn.
    pub async fn execute_turn(
        context_engine: &mut dyn ContextEngine,
        transport: &dyn TransportProvider,
        tools: &std::sync::Arc<oben_tools::ToolRegistry>,
        session_manager: &mut dyn SessionManager,
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
        )
        .await
    }

    /// Internal loop — shared turn pipeline with post-turn hook evaluation.
    ///
    /// `hooks` is the list of post-turn hooks to evaluate after each user turn.
    /// `run_loop` calls this with a single `NudgePostTurnHook`; higher-level
    /// code can pass additional hooks (goal continuation, etc.)
    pub async fn run_loop_impl(
        context_engine: &mut dyn ContextEngine,
        transport: Arc<dyn TransportProvider + Send + Sync>,
        tools: &Arc<oben_tools::ToolRegistry>,
        session_manager: &mut dyn SessionManager,
        call_mode: &mut Option<CallMode>,
        stream: bool,
        callbacks: &AgentCallbacks,
        hooks: &mut [Box<dyn crate::post_turn_hook::PostTurnHook>],
    ) -> Result<()> {
        let mut is_resumed_session = true;

        loop {
            if is_resumed_session {
                is_resumed_session = false;
            }

            // Print a newline before the prompt so the next response
            // doesn't attach to the text
            // if let Some(ref cb) = callbacks.print_newline {
            //     cb();
            // }

            if let Some(ref cb) = callbacks.print_prompt {
                cb();
            }
            if let Some(ref cb) = callbacks.print_flush {
                cb();
            }

            let input = match callbacks.read_input.as_ref().and_then(|f| f()) {
                Some(line) if line.trim().is_empty() => {
                    return Err(anyhow::anyhow!("No more input available"));
                }
                Some(line) => line.trim().to_string(),
                None => {
                    return Err(anyhow::anyhow!("stdin closed"));
                }
            };

            let should_exit_flag = callbacks.should_exit.as_ref().map(|f| f(&input)).unwrap_or(false);
            if should_exit_flag {
                break;
            }

            let sid = session_manager
                .active_session()
                .map(|s| s.id.clone())
                .unwrap_or_else(|| {
                    let id = crate::agent::generate_session_name();
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
                let f: StreamDeltaCallback = Box::new(move |text: &str| {
                    let _ = write!(std::io::stdout(), "{}", text);
                    let _ = std::io::stdout().flush();
                });
                Some(f)
            } else {
                None
            };

            // --- Turn execution with error-path persistence ---
            //
            // `execute_turn` takes `&mut session_manager` across the await,
            // preventing a second mutable borrow. We extract the result and
            // then call save() separately. Without this, errors would lose
            // the user's message (in memory but never persisted).
            let response = Self::execute_turn(
                context_engine,
                &*transport,
                tools,
                session_manager,
                &sid,
                input_msg,
                &call_mode_val,
                delta_cb,
            )
            .await;

            let response_text = match response {
                Ok(resp) => Some(resp),
                Err(e) => {
                    let err_str = format!("Turn error: {}", e);
                    std::mem::drop(e);
                    let _ = session_manager.incremental_save(None);
                    return Err(anyhow::anyhow!(err_str));
                }
            };
            let _ = session_manager.incremental_save(None);

            if let Some(resp) = response_text {
                if !stream {
                    if let Some(ref cb) = callbacks.print_info {
                        cb(&format!("{}", resp));
                    }
                } else {
                    // Stream mode: text already written via delta_cb without trailing newline
                    if let Some(ref cb) = callbacks.print_newline {
                        cb();
                    }
                }
                if let Some(ref cb) = callbacks.print_flush {
                    cb();
                }
            }

            // Print a newline after each response so the next prompt
            // doesn't attach to the LLM output
            if let Some(ref cb) = callbacks.print_newline {
                cb();
            }

            // --- Post-turn hooks ---
            let msg_count = session_manager
                .active_session()
                .map_or(0, |s| s.messages.len());
            let mut turns_since: usize = 0;

            for hook in hooks.iter_mut() {
                if !hook.should_trigger(msg_count, turns_since) {
                    turns_since += 1;
                    if is_resumed_session {
                        turns_since = 0; // skip first turn nudge
                    }
                    continue;
                }
                turns_since = 0;
                tracing::debug!("Post-turn hook '{}' triggered", hook.id());
                let turn_msg = hook.prepare_turn();
                let budget = IterationBudget::new(16);
                let turn_options = crate::conversation::TurnOptions {
                    retry_config: RetryConfig::default(),
                    budget: Some(budget),
                    interrupt: None,
                    callbacks: None,
                    fallback: None,
                };
                match Self::execute_turn_with_options(
                    context_engine,
                    &*transport,
                    tools,
                    session_manager,
                    &sid,
                    turn_msg,
                    &call_mode_val,
                    None,
                    turn_options,
                )
                .await
                {
                    Ok(resp) => hook.handle_result(&resp),
                    Err(_) => hook.handle_error(),
                }
            }
        }

        Ok(())
    }

    /// Run the interactive chat loop.
    ///
    /// After each turn, evaluates `hooks` for potential action. Each hook is
    /// responsible for its own trigger logic (e.g. `NudgePostTurnHook` checks
    /// turn count thresholds). If triggered, it runs a bounded review turn
    /// to let the model decide.
    ///
    /// The hook list is built once at the `Agent` level from config, making
    /// it configurable via `config.yaml` without editing source.
    pub async fn run_loop(
        context_engine: &mut dyn ContextEngine,
        transport: Arc<dyn TransportProvider + Send + Sync>,
        tools: &Arc<oben_tools::ToolRegistry>,
        session_manager: &mut dyn SessionManager,
        call_mode: &mut Option<CallMode>,
        stream: bool,
        callbacks: &AgentCallbacks,
        hooks: &mut [Box<dyn PostTurnHook>],
    ) -> Result<()> {
        Self::run_loop_impl(
            context_engine,
            transport,
            tools,
            session_manager,
            call_mode,
            stream,
            callbacks,
            hooks,
        )
        .await
    }
}
