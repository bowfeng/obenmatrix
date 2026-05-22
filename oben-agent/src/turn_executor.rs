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
use crate::context::ContextEngine;
use crate::error_classifier::{ClassifiedError, ErrorKind};
use crate::interrupt::SharedInterrupt;
use crate::message_sanitize::sanitize_messages;
use crate::retry::{retry_with_backoff, RetryConfig, retryable_transient};
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
}

impl Default for TurnConfig {
    fn default() -> Self {
        Self {
            retry_config: RetryConfig::default(),
            budget_warning: None,
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
        config: TurnConfig,
    ) -> Result<TurnResult> {
        let session = store
            .session_mut(session_id)
            .ok_or_else(|| anyhow::anyhow!("Session not found: {}", session_id))?;

        // Add user message to session
        session.messages.push(user_message);

        // ── Streaming setup ──────────────────────────────────────────
        let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(4096);
        let mut callback_handle: Option<tokio::task::JoinHandle<()>> = None;
        let has_callback = delta_callback.is_some();

        if let Some(cb) = delta_callback {
            let cb = Arc::new(std::sync::Mutex::new(cb));
            callback_handle = Some(tokio::task::spawn(async move {
                let mut buf = String::new();
                const FLUSH_THRESHOLD: usize = 512;
                while let Some(chunk) = rx.recv().await {
                    buf.push_str(&chunk);
                    if buf.len() >= FLUSH_THRESHOLD {
                        let text = std::mem::take(&mut buf);
                        cb.lock().unwrap()(&text);
                    }
                }
                if !buf.is_empty() {
                    cb.lock().unwrap()(&buf);
                }
            }));
        }

        // ── Budget setup ─────────────────────────────────────────────
        let mut budget = budget.unwrap_or_else(|| IterationBudget::new(90));
        if let Some(cb) = config.budget_warning {
            budget.on_warning(cb);
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

            // ── API call with retry ────────────────────────────────────
            let transport_ref = transport;

            let response = retry_with_backoff(&config.retry_config, || {
                let transport_ref = transport_ref;
                let messages = session.messages.clone();
                let mode = call_mode.clone();

                async move {
                    match transport_ref.chat(&messages, &mode).await {
                        Ok(resp) => Ok(resp),
                        Err(e) => {
                            let classified = ClassifiedError::from_anyhow(&e);
                            if classified.kind.is_retryable() {
                                Err(retryable_transient(e.to_string()))
                            } else {
                                Err(e)
                            }
                        }
                    }
                }
            }).await;

            match response {
                Ok(response) => {
                    let tool_calls = &response.tool_calls;
                    let text = &response.text;

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
                        if has_callback {
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

                    // ── Dispatch tool calls ──────────────────────────────
                    for call in tool_calls {
                        // Check interrupt before each tool
                        if let Some(int_state) = &interrupt {
                            if int_state.is_interrupted() {
                                let msg = int_state.drain_interrupt_message();
                                info!("Interrupted during tool dispatch: {:?}", msg);
                                return Ok(TurnResult {
                                    text: String::new(),
                                    messages: session.messages.clone(),
                                });
                            }
                        }

                        let result = tools.execute(&call.tool_name, &call.arguments).await;
                        session.messages.push(Message::tool_result(&call.id, &result.output));
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
                    if has_callback {
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
