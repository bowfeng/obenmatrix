/// Conversation loop — coordinator that wires the deep `TurnExecutor`.
use anyhow::Result;
use std::sync::Arc;

use crate::budget::IterationBudget;
use crate::callbacks::AgentCallbacks;
use crate::concurrent_dispatch::ConcurrentDispatchConfig;
use crate::context::ContextEngine;
use crate::fallback::FallbackChain;
use crate::interrupt::SharedInterrupt;
use crate::post_turn_hook::PostTurnHook;
use crate::retry::RetryConfig;
use crate::turn_executor::{TurnConfig, TurnExecutor};
use oben_config::AppConfig;
use oben_models::{CallMode, Message, SessionManager, TransportProvider};

/// Configuration for a turn execution in ConversationLoop.
pub struct TurnOptions {
    pub retry_config: RetryConfig,
    pub budget: Option<IterationBudget>,
    pub interrupt: Option<SharedInterrupt>,
    pub callbacks: Option<Arc<AgentCallbacks>>,
    pub fallback: Option<FallbackChain>,
    pub dispatch_config: Option<ConcurrentDispatchConfig>,
}

impl Default for TurnOptions {
    fn default() -> Self {
        Self {
            retry_config: RetryConfig::default(),
            budget: None,
            interrupt: None,
            callbacks: None,
            fallback: None,
            dispatch_config: None,
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
        options: TurnOptions,
    ) -> Result<String> {
        let TurnOptions {
            retry_config,
            budget,
            interrupt,
            callbacks,
            fallback,
            dispatch_config,
        } = options;

        let turn_config = TurnConfig {
            retry_config,
            budget_warning: None,
            callbacks: callbacks,
            fallback_chain: fallback,
            dispatch_config: dispatch_config,
        };

        let result: crate::TurnResult = TurnExecutor::execute_turn_with_config(
            context_engine,
            transport,
            tools,
            session_manager,
            session_id,
            user_message,
            call_mode,
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
    ) -> Result<String> {
        Self::execute_turn_with_options(
            context_engine,
            transport,
            tools,
            session_manager,
            session_id,
            user_message,
            call_mode,
            TurnOptions::default(),
        )
        .await
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
        callbacks: Arc<AgentCallbacks>,
        hooks: &mut [Box<dyn PostTurnHook>],
        app_config: &AppConfig,
    ) -> Result<()> {
        let mut is_resumed_session = session_manager.active_session().is_some();

        // Extract config once to avoid rebuilding on every loop iteration
        let retry_config = RetryConfig {
            max_retries: app_config.retry.max_retries,
            base_delay_ms: app_config.retry.base_delay_ms,
            max_delay_ms: app_config.retry.max_delay_ms,
            jitter_factor: app_config.retry.jitter_factor,
            retryable_codes: app_config.retry.retryable_codes.clone(),
        };
        let max_iterations = app_config.max_iterations.unwrap_or(50);

        // Build fallback chain from app_config
        let fallback_chain = if !app_config.fallback_models.is_empty() {
            let fallback_configs: Vec<crate::fallback::FallbackConfig> = app_config
                .fallback_models
                .iter()
                .map(|fb| crate::fallback::FallbackConfig {
                    provider: fb.provider.clone(),
                    model: fb.model.clone(),
                    api_key: fb.api_key.clone(),
                    base_url: fb.base_url.clone(),
                })
                .collect();
            Some(FallbackChain::new(fallback_configs))
        } else {
            None
        };

        // Build concurrent dispatch config from app_config
        let dispatch_config = Some(ConcurrentDispatchConfig {
            max_concurrency: app_config.concurrency.max_concurrency,
            serial_only_tools: app_config.concurrency.serial_only_tools.clone(),
            destructive_tools: app_config.concurrency.destructive_tools.clone(),
        });

        loop {
            let cb = Arc::as_ref(&callbacks);
            cb.on_print_prompt();
            cb.on_print_flush();

            let input = match cb.on_read_input() {
                Some(line) if line.trim().is_empty() => {
                    return Err(anyhow::anyhow!("No more input available"));
                }
                Some(line) => line.trim().to_string(),
                None => {
                    return Err(anyhow::anyhow!("stdin closed"));
                }
            };

            let should_exit_flag = cb.on_should_exit(&input);
            if should_exit_flag {
                break;
            }

            let sid = session_manager
                .active_session()
                .map(|s| s.id.clone())
                .unwrap_or_else(|| {
                    let id = crate::agent::generate_session_name();
                    let _ = session_manager.new_session(&id);
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

            // --- Turn execution with error-path persistence ---
            //
            // `execute_turn` takes `&mut session_manager` across the await,
            // preventing a second mutable borrow. We extract the result and
            // then call save() separately. Without this, errors would lose
            // the user's message (in memory but never persisted).
            let response = Self::execute_turn_with_options(
                context_engine,
                &*transport,
                tools,
                session_manager,
                &sid,
                input_msg,
                &call_mode_val,
                TurnOptions {
                    retry_config: retry_config.clone(),
                    budget: Some(IterationBudget::new(max_iterations)),
                    interrupt: None,
                    callbacks: Some(Arc::clone(&callbacks)),
                    fallback: fallback_chain.clone(),
                    dispatch_config: dispatch_config.clone(),
                },
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
                    Arc::as_ref(&callbacks).on_print_info(&format!("{}", resp));
                } else {
                    // Stream mode: text already written via delta_cb without trailing newline
                    Arc::as_ref(&callbacks).on_print_newline();
                }
                Arc::as_ref(&callbacks).on_print_flush();
            }

            // Print a newline after each response so the next prompt
            // doesn't attach to the LLM output
            Arc::as_ref(&callbacks).on_print_newline();

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
                    retry_config: retry_config.clone(),
                    budget: Some(budget),
                    interrupt: None,
                    callbacks: None,
                    fallback: fallback_chain.clone(),
                    dispatch_config: dispatch_config.clone(),
                };
                match Self::execute_turn_with_options(
                    context_engine,
                    &*transport,
                    tools,
                    session_manager,
                    &sid,
                    turn_msg,
                    &call_mode_val,
                    turn_options,
                )
                .await
                {
                    Ok(resp) => hook.handle_result(&resp),
                    Err(_) => hook.handle_error(),
                }
            }
            // After the first loop iteration, clear the resumed flag so
            // subsequent turns behave normally.
            is_resumed_session = false;
        }

        Ok(())
    }
}
