/// Turn executor — deep module for the core agent turn cycle.
///
/// **Stateless**: `TurnExecutor` owns nothing — all resources (ContextEngine,
/// Budget, Transport, Tools) are passed as function parameters.
///
/// Encapsulates the full turn cycle: budget check → compression → LLM call
/// → tool dispatch → repeat until no more tool calls.

use std::sync::Arc;

use anyhow::Result;
use tracing::info;

use crate::budget::{BudgetWarningCallback, IterationBudget};
use crate::callbacks::AgentCallbacks;
use crate::concurrent_dispatch::{self, ConcurrentDispatchConfig, PendingToolCall};
use crate::context::ContextEngine;
use crate::error_classifier::{ClassifiedError, ErrorKind};
use crate::fallback::FallbackChain;
use crate::interrupt::SharedInterrupt;
use crate::message_sanitize::sanitize_messages;
use crate::retry::{retry_with_backoff, RetryConfig, retryable_transient};
use crate::stream_processor;
use oben_models::{Message, MessageRole, SessionStore, TransportProvider};

// ---------------------------------------------------------------------------
// TurnConfig — configuration for turn execution
// ---------------------------------------------------------------------------

/// Configuration for a turn execution.
pub struct TurnConfig {
    /// Retry configuration for API calls.
    pub retry_config: RetryConfig,
    /// Budget warning callback.
    pub budget_warning: Option<BudgetWarningCallback>,
    /// Agent callbacks for rich event dispatch (Tier 2).
    pub callbacks: Option<AgentCallbacks>,
    /// Fallback model chain (Tier 2).
    pub fallback_chain: Option<FallbackChain>,
    /// Concurrent dispatch configuration (Tier 2).
    pub dispatch_config: Option<ConcurrentDispatchConfig>,
}

impl Default for TurnConfig {
    fn default() -> Self {
        Self {
            retry_config: RetryConfig::default(),
            budget_warning: None,
            callbacks: None,
            fallback_chain: None,
            dispatch_config: None,
        }
    }
}

// ---------------------------------------------------------------------------
// TurnResult — what the executor returns after executing a turn
// ---------------------------------------------------------------------------

pub struct TurnResult {
    pub text: String,
    pub messages: Vec<Message>,
}

// ---------------------------------------------------------------------------
// TurnExecutor — stateless turn cycle
// ---------------------------------------------------------------------------

/// Stateless executor — the full agent turn cycle.
///
/// **Deep module**: one method, high leverage. All resources (ContextEngine,
/// Budget, Transport, Tools) are passed as function parameters — `TurnExecutor`
/// owns none of them.
///
/// **Responsibilities**: budget check → compression → LLM call → tool dispatch → repeat.
pub struct TurnExecutor;

impl TurnExecutor {
    /// Execute one turn: budget check → compress → LLM → dispatch → repeat.
    ///
    /// **Parameters** — all resources passed as parameters:
    /// - `context_engine`: token tracking & compression
    /// - `transport`: LLM API (dyn trait)
    /// - `tools`: tool registry
    /// - `store`: session store
    /// - `session_id`: which session to operate on
    /// - `user_message`: the user's input
    /// - `call_mode`: Fresh or Incremental
    /// - `delta_callback`: optional streaming callback
    pub async fn execute_turn(
        context_engine: &mut dyn ContextEngine,
        transport: &dyn TransportProvider,
        tools: &Arc<oben_tools::ToolRegistry>,
        store: &mut dyn SessionStore,
        session_id: &str,
        user_message: Message,
        call_mode: &oben_models::CallMode,
        delta_callback: Option<oben_models::StreamDeltaCallback>,
    ) -> Result<TurnResult> {
        Self::execute_turn_with_config(
            context_engine,
            transport,
            tools,
            store,
            session_id,
            user_message,
            call_mode,
            delta_callback,
            None,
            None,
            TurnConfig::default(),
        ).await
    }

    /// Execute one turn with full Tier 1 feature integration.
    ///
    /// This is the production-ready turn execution that includes:
    /// - Message sanitization (surrogate stripping, thinking-only drop, user merge)
    /// - Iteration budget with warnings
    /// - Retry with jittered exponential backoff
    /// - Error classification
    /// - Cross-thread interrupt support
    /// - Steer injection
    pub async fn execute_turn_with_config(
        context_engine: &mut dyn ContextEngine,
        transport: &dyn TransportProvider,
        tools: &Arc<oben_tools::ToolRegistry>,
        store: &mut dyn SessionStore,
        session_id: &str,
        user_message: Message,
        call_mode: &oben_models::CallMode,
        delta_callback: Option<oben_models::StreamDeltaCallback>,
        budget: Option<IterationBudget>,
        interrupt: Option<SharedInterrupt>,
        mut config: TurnConfig,
    ) -> Result<TurnResult> {
        let session = store
            .session_mut(session_id)
            .ok_or_else(|| anyhow::anyhow!("Session not found: {}", session_id))?;

        // Add user message to session
        session.messages.push(user_message);

        // ── Streaming setup ──────────────────────────────────────────
        let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(4096);
        let mut callback_handle: Option<tokio::task::JoinHandle<()>> = None;
        let has_streaming = delta_callback.is_some();

        // Build the callback that stream_chat will invoke.
        // This callback writes each text chunk to `tx`. A separate spawned
        // task reads from `rx`, buffers chunks, and forwards them to the
        // user-provided callback.
        let mut stream_cb: Option<oben_models::StreamDeltaCallback> = if let Some(user_cb) = delta_callback {
            let user_cb = Arc::new(std::sync::Mutex::new(user_cb));
            callback_handle = Some(tokio::task::spawn(async move {
                let mut buf = String::new();
                const FLUSH_THRESHOLD: usize = 512;
                while let Some(chunk) = rx.recv().await {
                    buf.push_str(&chunk);
                    if buf.len() >= FLUSH_THRESHOLD {
                        let text = std::mem::take(&mut buf);
                        user_cb.lock().unwrap()(&text);
                    }
                }
                if !buf.is_empty() {
                    user_cb.lock().unwrap()(&buf);
                }
            }));
            // Create a callback that writes chunks to the channel
            let cb_tx = tx.clone();
            Some(Box::new(move |text: &str| {
                let _ = cb_tx.try_send(text.to_string());
            }) as oben_models::StreamDeltaCallback)
        } else {
            None
        };

        // ── Budget setup ─────────────────────────────────────────────
        let mut budget = budget.unwrap_or_else(|| IterationBudget::new(90));
        if let Some(cb) = config.budget_warning {
            budget.on_warning(cb);
        }

        // ── Activity tracking: mark turn start ───────────────────────
        if let Some(ref cb) = config.callbacks {
            cb.call_status("lifecycle", "turn_start");
        }
        if let Some(ref int_state) = interrupt {
            int_state.touch_activity();
        }

        // ── Core loop: LLM call → tool dispatch → repeat ─────────────
        loop {
            // Check interrupt
            if let Some(int_state) = &interrupt {
                if int_state.is_interrupted() {
                    let msg = int_state.drain_interrupt_message();
                    info!("Turn interrupted: {:?}", msg);
                    return Ok(TurnResult {
                        text: String::new(),
                        messages: session.messages.clone(),
                    });
                }
            }

            // Budget check
            if let Err(e) = budget.check() {
                let classified = ClassifiedError::from_anyhow(&e);
                match &classified.kind {
                    ErrorKind::Other(msg) if msg.contains("grace") || msg.contains("exhausted") => {
                        // Budget warning injected — add a message to context
                        let warning_msg = oben_models::Message::user(
                            "⚠️ You have reached your iteration limit. Please provide a final answer now without using any more tools."
                        );
                        session.messages.push(warning_msg);
                        budget.consume_grace_call();
                        continue;
                    }
                    _ => return Err(e),
                }
            }

            // Auto-compression
            if context_engine.should_compact(&session.messages) {
                info!(
                    "Auto-compression: context full ({} messages, {} est. tokens), compressing",
                    session.messages.len(),
                    context_engine.estimate_tokens(&session.messages),
                );
                context_engine
                    .compact(&mut session.messages, Some(transport), None)
                    .await?;
            }

            // Sanitize messages before API call
            sanitize_messages(&mut session.messages);

            // Check for pending steer to inject into last tool result
            if let Some(int_state) = &interrupt {
                if let Some(steer_text) = int_state.drain_pending_steer() {
                    let steer_msg = Message::tool_result("steer", &steer_text);
                    session.messages.push(steer_msg);
                    info!("Injected steer text into tool result");
                }
            }

            // ── API call with retry + activity tracking ──
            let transport_ref = transport;
            let callbacks_ref = &config.callbacks;

            // On the first iteration, consume the streaming callback.
            // Subsequent iterations (tool call responses) use non-streaming chat().
            let response = match stream_cb.take() {
                Some(delta_cb) => {
                    // ── Streaming path: use stream_chat for real-time output ──
                    // Wrap callback in Arc<Mutex> so retries can share it.
                    // Create a closure wrapper for each retry attempt.
                    let cb_shared = Arc::new(std::sync::Mutex::new(delta_cb));
                    retry_with_backoff(&config.retry_config, || {
                        let transport_ref = transport_ref;
                        let messages = session.messages.clone();
                        let mode = call_mode.clone();
                        let cb = cb_shared.clone();
                        let callbacks_ref = callbacks_ref;

                        async move {
                            if let Some(cb) = callbacks_ref {
                                cb.call_status("lifecycle", "api_call_start");
                            }
                            // Create a new callback that delegates to the shared mutex
                            let cb_clone = cb.clone();
                            let cb_wrapper = Box::new(move |text: &str| {
                                cb_clone.lock().unwrap()(text);
                            }) as oben_models::StreamDeltaCallback;
                            match transport_ref.stream_chat(&messages, &mode, cb_wrapper).await {
                                Ok(resp) => {
                                    if let Some(cb) = callbacks_ref {
                                        cb.call_status("lifecycle", "api_call_complete");
                                    }
                                    Ok(resp)
                                }
                                Err(e) => {
                                    if let Some(cb) = callbacks_ref {
                                        cb.call_status("warn", &format!("api_call_failed: {}", e));
                                    }
                                    Err(retryable_transient(e.to_string()))
                                }
                            }
                        }
                    }).await
                }
                None => {
                    // ── Non-streaming path: use chat() ──
                    retry_with_backoff(&config.retry_config, || {
                        let transport_ref = transport_ref;
                        let messages = session.messages.clone();
                        let mode = call_mode.clone();
                        let callbacks_ref = callbacks_ref;

                        async move {
                            if let Some(cb) = callbacks_ref {
                                cb.call_status("lifecycle", "api_call_start");
                            }
                            match transport_ref.chat(&messages, &mode).await {
                                Ok(resp) => {
                                    if let Some(cb) = callbacks_ref {
                                        cb.call_status("lifecycle", "api_call_complete");
                                    }
                                    Ok(resp)
                                }
                                Err(e) => {
                                    if let Some(cb) = callbacks_ref {
                                        cb.call_status("warn", &format!("api_call_failed: {}", e));
                                    }
                                    Err(retryable_transient(e.to_string()))
                                }
                            }
                        }
                    }).await
                }
            };

            // ── Fallback activation (after retry loop) ─────────────────
            if let Err(ref e) = response {
                let classified = ClassifiedError::from_anyhow(e);
                if classified.kind.is_retryable() {
                    if let Some(ref mut chain) = config.fallback_chain {
                        if let Some(_fb) = chain.activate_next() {
                            info!(
                                "Fallback activated after retries: {}/{} (HTTP {:?})",
                                _fb.provider, _fb.model, classified.http_code
                            );
                            if let Some(ref cb) = config.callbacks {
                                cb.call_status(
                                    "warn",
                                    &format!("fallback_activated: {}/{}", _fb.provider, _fb.model),
                                );
                            }
                        }
                    }
                }
            }

            match response {
                Ok(response) => {
                    let tool_calls = &response.tool_calls;
                    let mut text = response.text.to_string();

                    tracing::debug!("TurnExecutor: got LLM response text len={}, tool_calls={}, first_100={:?}",
                        text.len(), tool_calls.len(), &text[..text.len().min(100)]);

                    // ── Stream scrubbing (Tier 2): strip thinking blocks & memory context ──
                    let before_scrub = text.clone();
                    text = stream_processor::scrub_thinking_blocks(&text);
                    if text != before_scrub {
                        tracing::warn!("scrub_thinking_blocks changed text: before_len={} after_len={} diff={}",
                            before_scrub.len(), text.len(), before_scrub.len() - text.len());
                    }
                    text = stream_processor::scrub_memory_context(&text);

                    // Update token tracking
                    if let Some(tokens) = response.tokens_used {
                        context_engine.update_from_response(tokens, 0, tokens);
                    }

                    // Add assistant response to session
                    let assistant_msg = if !tool_calls.is_empty() {
                        let tool_call_data = tool_calls
                            .iter()
                            .map(oben_models::ToolCall::from_transport)
                            .collect();
                        Message::assistant_tool_calls(tool_call_data)
                    } else {
                        Message::assistant(text.trim().to_string())
                    };
                    session.messages.push(assistant_msg);

                    if tool_calls.is_empty() {
                        // Flush streaming output
                        if has_streaming {
                            drop(tx);
                            if let Some(handle) = callback_handle.take() {
                                let _ = handle.await;
                            }
                        }

                        // When text is empty, return last tool result if available
                        if text.trim().is_empty() {
                            if let Some(last_tool_result) = session.messages.last().and_then(|m| {
                                if m.role == MessageRole::Tool {
                                    m.content.to_text_ref()
                                } else {
                                    None
                                }
                            }) {
                                if !last_tool_result.is_empty() {
                                    return Ok(TurnResult {
                                        text: last_tool_result.to_string(),
                                        messages: session.messages.clone(),
                                    });
                                }
                            }
                        }

                        return Ok(TurnResult {
                            text: text.trim().to_string(),
                            messages: session.messages.clone(),
                        });
                    }

                    // ── Dispatch tool calls (concurrent + callbacks, Tier 2) ─────
                    let default_dispatch = ConcurrentDispatchConfig::default();
                    let dispatch_config = config.dispatch_config.as_ref().unwrap_or(&default_dispatch);

                    // Convert to pending calls and dispatch
                    let pending_calls: Vec<PendingToolCall> = tool_calls
                        .iter()
                        .map(|c| PendingToolCall {
                            tool_name: c.tool_name.clone(),
                            arguments: c.arguments.clone(),
                            call_id: c.id.clone(),
                        })
                        .collect();

                    // Notify callbacks of tool generation
                    if let Some(cb) = &config.callbacks {
                        for call in &pending_calls {
                            cb.call_tool_gen(&call.tool_name, &call.call_id);
                            cb.call_tool_start(&call.tool_name, &call.arguments.to_string());
                        }
                    }

                    // Dispatch: concurrent if multiple, sequential if single
                    let results = concurrent_dispatch::dispatch_tool_calls(
                        tools,
                        dispatch_config,
                        &pending_calls,
                        interrupt.as_ref(),
                    ).await?;

                    // Store results and notify callbacks
                    for (i, result) in results.iter().enumerate() {
                        session.messages.push(Message::tool_result(&pending_calls[i].call_id, &result.output));
                        if let Some(cb) = &config.callbacks {
                            cb.call_tool_complete(&pending_calls[i].tool_name, &pending_calls[i].arguments.to_string(), &result.output);
                        }
                    }
                }
                Err(e) => {
                    // All retries exhausted — return error
                    let classified = ClassifiedError::from_anyhow(&e);
                    info!(
                        "API call failed after retries (kind={:?}, code={:?}): {}",
                        classified.kind, classified.http_code, e
                    );

                    // Flush streaming if active
                    if has_streaming {
                        drop(tx);
                        if let Some(handle) = callback_handle.take() {
                            let _ = handle.await;
                        }
                    }

                    return Err(e);
                }
            }
        }
    }
}
