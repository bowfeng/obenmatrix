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
use crate::retry::{retry_with_backoff, retryable_transient, RetryConfig};
use crate::stream_processor;
use oben_models::{Message, MessageRole, Session, SessionManager, StreamDeltaCallback, TransportProvider};

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
    pub callbacks: Option<Arc<AgentCallbacks>>,
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
    /// - `session_manager`: session lifecycle (for rotation on compression)
    /// - `session_id`: which session to operate on
    /// - `user_message`: the user's input
    /// - `call_mode`: Fresh or Incremental
    pub async fn execute_turn(
        context_engine: &mut dyn ContextEngine,
        transport: &dyn TransportProvider,
        tools: &Arc<oben_tools::ToolRegistry>,
        session_manager: &mut dyn SessionManager,
        session_id: &str,
        user_message: Message,
        call_mode: &oben_models::CallMode,
    ) -> Result<TurnResult> {
        Self::execute_turn_with_config(
            context_engine,
            transport,
            tools,
            session_manager,
            session_id,
            user_message,
            call_mode,
            None,  // budget
            None,  // interrupt
            TurnConfig::default(),
        )
        .await
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
        session_manager: &mut dyn SessionManager,
        session_id: &str,
        user_message: Message,
        call_mode: &oben_models::CallMode,
        budget: Option<IterationBudget>,
        interrupt: Option<SharedInterrupt>,
        mut config: TurnConfig,
    ) -> Result<TurnResult> {
        let mut current_session_id = session_id.to_string();
        let mut current_session: Option<&mut Session> =
            session_manager.session_mut(&current_session_id).map(|s| {
                current_session_id = s.id.clone();
                s
            });

        let mut session = current_session
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("Session not found: {}", current_session_id))?;

        // Add user message to session
        session.messages.push(user_message);

        // ── Budget setup ─────────────────────────────────────────────
        let mut budget = budget.unwrap_or_else(|| IterationBudget::new(90));
        if let Some(cb) = config.budget_warning {
            budget.on_warning(cb);
        }

        // ── Activity tracking: mark turn start ───────────────────────
        if let Some(ref cb) = config.callbacks {
            cb.on_status("lifecycle", "turn_start");
        }
        if let Some(ref int_state) = interrupt {
            int_state.touch_activity();
        }

        // ── Core loop: LLM call → tool dispatch → repeat ─────────────
        // Tracks consecutive empty (no text, no tool calls) LLM responses.
        // Used to retry when a model returns empty output after tool results.
        let mut consecutive_empty_responses: u32 = 0;

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
                let compacted = context_engine
                    .compact(&mut session.messages, Some(transport), None)
                    .await?;

                if compacted == crate::context::CompactStatus::Unchanged {
                    // Compression was ineffective — messages unchanged,
                    // skip rotation and continue with the current session.
                    tracing::warn!(
                        "Compression ineffective ({} messages, {} est. tokens), skipping rotation",
                        session.messages.len(),
                        context_engine.estimate_tokens(&session.messages),
                    );
                } else {
                    // Session rotation: end parent, create child with lineage.
                    // This mirrors Hermes Agent's `compress_context()` which splits
                    // the session into parent (ended) + child (continuation).
                    //
                    // We can't hold `session` (borrowed from `session_manager`)
                    // while calling `session_manager` methods. Drop the borrow,
                    // do the rotation, then re-acquire.
                    let parent_id = current_session_id.clone();
                    // Clone compacted messages before dropping the borrow — they need
                    // to be copied into the child session after rotation.
                    let compacted_msgs = session.messages.clone();
                    let _ = session;

                    // 1. End the parent session
                    context_engine.on_session_end(&parent_id);

                    // 2. Split: end parent in DB, create child with lineage
                    let child_session = match session_manager.split_after_compression(&parent_id) {
                        Ok(s) => s,
                        Err(e) => {
                            tracing::warn!(
                                "Session rotation failed: {} (continuing with parent)",
                                e
                            );
                            // Re-acquire old session reference
                            current_session =
                                Some(session_manager.session_mut(&parent_id).ok_or_else(|| {
                                    anyhow::anyhow!("Parent session disappeared: {}", parent_id)
                                })?);
                            session = current_session.as_mut().ok_or_else(|| {
                                anyhow::anyhow!("Parent session disappeared: {}", parent_id)
                            })?;
                            continue;
                        }
                    };
                    let child_id = child_session.id.clone();

                    // 3. Before taking &mut session, clone parent messages and persist to DB.
                    //    Both session_manager.session() and save_compacted need access to
                    //    session_manager, and they conflict with &mut session.
                    let compacted = {
                        let parent = session_manager.session(&parent_id).ok_or_else(|| {
                            anyhow::anyhow!("Parent session disappeared after split: {}", parent_id)
                        })?;
                        parent.messages.clone()
                    };
                    // Now the &borrow on parent is released, so we can call save_compacted.
                    session_manager
                        .save_compacted(&child_id, &compacted)
                        .map_err(|e| {
                            anyhow::anyhow!(
                                "Failed to persist compacted messages to child {}: {}",
                                child_id,
                                e
                            )
                        })?;

                    // 4. Update our tracking — take mutable ref to child session
                    current_session_id = child_id.clone();
                    current_session =
                        Some(session_manager.session_mut(&child_id).ok_or_else(|| {
                            anyhow::anyhow!("Child session disappeared: {}", child_id)
                        })?);
                    session = current_session.as_mut().ok_or_else(|| {
                        anyhow::anyhow!("Child session disappeared: {}", child_id)
                    })?;

                    // 5. Copy compacted messages into child session's in-memory state
                    session.messages = compacted_msgs;

                    // 6. Start the new session on the context engine
                    let model = session.metadata.model.as_deref().unwrap_or("unknown");
                    let context_length = if session.metadata.last_prompt_tokens > 0 {
                        Some(session.metadata.last_prompt_tokens * 2)
                    } else {
                        Some(128_000)
                    };
                    context_engine.on_session_start(&child_id, model, context_length);

                    info!(
                        "Session rotated: {} → {}, copied {} compacted messages",
                        parent_id,
                        child_id,
                        session.messages.len(),
                    );
                }
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

            // All streaming dispatch goes through the hook system.
            // Clone Arc for 'static lifetime in the stream_chat closure.
            let callback = config.callbacks.as_ref().map(|cb| Arc::clone(cb));
            let callback_for_stream = callback.clone();
            let response = retry_with_backoff(&config.retry_config, || {
                let transport_ref = transport_ref;
                let messages = session.messages.clone();
                let mode = call_mode.clone();
                let callback = callback_for_stream.clone();

                async move {
                    if let Some(ref cb) = callback {
                        cb.on_status("lifecycle", "api_call_start");
                    }
                    let callback_for_dispatch = callback.clone();
                    let cb_wrapper: StreamDeltaCallback = Box::new(move |text: &str| {
                        // Dispatch every delta through the hook system in real-time.
                        // The accumulated response text will also be dispatched below.
                        if let Some(ref cb) = callback {
                            cb.on_stream_delta(text);
                        }
                    }) as StreamDeltaCallback;
                    match transport_ref
                        .stream_chat(&messages, &mode, cb_wrapper)
                        .await
                    {
                        Ok(response) => {
                            // Dispatch accumulated response text through the hook system.
                            // Ensures subscribers receive the full response.
                            if let Some(ref cb) = callback_for_dispatch {
                                cb.on_stream_delta(&response.text);
                            }
                            Ok(response)
                        }
                        Err(e) => {
                            Err(retryable_transient(e.to_string()))
                        }
                    }
                }
            })
            .await;

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
                                cb.on_status(
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

                    // text.len() is byte count (UTF-8), not char count.
                    // Slice by chars to avoid cutting mid-character.
                    let preview: String = text.chars().take(100).collect();
                    tracing::debug!(
                        "TurnExecutor: got LLM response text len={}, tool_calls={}, first_100={:?}",
                        text.len(),
                        tool_calls.len(),
                        preview
                    );

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

                    // ── Detect empty LLM response (text="" + tool_calls=[]). ──────
                    // When the model returns a fully empty response after tool
                    // results, it has gone silent. Inject a system hint and retry,
                    // bounded to prevent infinite loops (up to 2 extra attempts).
                    let is_response_empty = text.trim().is_empty()
                        && tool_calls.is_empty()
                        && response.tokens_used.unwrap_or(0) > 0;
                    if is_response_empty {
                        consecutive_empty_responses += 1;
                        let hint = "Your previous response was completely empty and will be skipped. Please summarize what you learned from the tool results above, or use a tool call if you need more information.";

                        if consecutive_empty_responses < 2 {
                            info!(
                                "Empty LLM response (attempt {}), injecting system hint to recover",
                                consecutive_empty_responses
                            );
                            if let Some(cb) = &config.callbacks {
                                cb.on_status(
                                    "info",
                                    &format!(
                                        "empty_response_recovery: attempt={}",
                                        consecutive_empty_responses
                                    ),
                                );
                            }
                            session.messages.push(Message::system(hint.to_string()));
                            continue;
                        } else {
                            info!(
                                "Empty LLM response after 2 retries, skipping assistant message and falling back to last tool result"
                            );
                        }
                    } else {
                        consecutive_empty_responses = 0;
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
                    let dispatch_config =
                        config.dispatch_config.as_ref().unwrap_or(&default_dispatch);

                    // Convert to pending calls and dispatch
                    let parent_session_id = current_session_id.clone();
                    let pending_calls: Vec<PendingToolCall> = tool_calls
                        .iter()
                        .map(|c| {
                            let mut args = c.arguments.clone();
                            // Inject parent_session_id into delegate task calls
                            if c.tool_name == "delegate_task" {
                                if let Some(obj) = args.as_object_mut() {
                                    obj.entry("parent_session_id").or_insert_with(|| {
                                        serde_json::Value::String(parent_session_id.clone())
                                    });
                                }
                            }
                            PendingToolCall {
                                tool_name: c.tool_name.clone(),
                                arguments: args,
                                call_id: c.id.clone(),
                            }
                        })
                        .collect();

                    // Notify callbacks of tool generation
                    if let Some(cb) = &config.callbacks {
                        for call in &pending_calls {
                            cb.on_tool_gen(&call.tool_name, &call.call_id);
                            cb.on_tool_start(&call.tool_name, &call.arguments.to_string());
                        }
                    }

                    // Dispatch: concurrent if multiple, sequential if single
                    let results = concurrent_dispatch::dispatch_tool_calls(
                        tools,
                        dispatch_config,
                        &pending_calls,
                        interrupt.as_ref(),
                    )
                    .await?;

                    // Store results and notify callbacks
                    for (i, result) in results.iter().enumerate() {
                        let tool_call_id = &pending_calls[i].call_id;

                        // Skip completely empty results unless it was a steer message.
                        // A result with an error is not empty — the error message
                        // is critical feedback for the LLM to understand what went wrong.
                        let is_steer = tool_call_id == "steer";
                        if result.output.is_empty() && !is_steer && result.error.is_none() {
                            continue;
                        }

                        let msg = if !result.output.is_empty() {
                            Message::tool_result(tool_call_id, &result.output)
                        } else if let Some(ref err) = result.error {
                            // Store the error as the tool result so the LLM can learn
                            Message::tool_result(tool_call_id, err)
                        } else {
                            // For steer messages with no output/error, add placeholder
                            Message {
                                role: oben_models::MessageRole::Tool,
                                content: oben_models::MessageContent::Text(String::new()),
                                id: None,
                                tool_call_ids: vec![tool_call_id.clone()],
                                tool_calls: None,
                            }
                        };
                        session.messages.push(msg);
                        if let Some(cb) = &config.callbacks {
                            cb.on_tool_complete(
                                &pending_calls[i].tool_name,
                                &pending_calls[i].arguments.to_string(),
                                &result.output,
                            );
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

                    return Err(e);
                }
            }
        }
        // Note: all code paths inside the loop use `return`, so this point is unreachable.
        #[allow(unreachable_code)]
        let _ = unreachable!("all loop paths return early");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression test for UTF-8 char boundary slicing bug.
    ///
    /// **Bug**: `&text[..text.len().min(100)]` uses **byte count**, not
    /// **char count**. A Chinese character like "没" is 3 bytes (98-101),
    /// so byte 100 splits the character in half → panic!
    ///
    /// **Fix**: `text.chars().take(100).collect()` iterates char boundaries.
    #[test]
    fn test_utf8_char_slice_does_not_panic_with_chinese() {
        // From the actual crash: byte 100 falls inside "没" (bytes 98-101)
        let text = "\n\n有一天，一块三分熟的牛排在街上走着，突然看到一块五分熟的牛排，却没有打招呼。\n为什么？\n因为他们**不熟**。😄\n\n还想听程序员专属笑话，还是日常冷笑话？随时点单～";

        // text.len() is byte count; the old code used it to slice → panic
        assert!(
            text.len() > 100,
            "text must be > 100 bytes for the bug to trigger"
        );

        // The OLD buggy code would panic:
        // let _ = &text[..text.len().min(100)]; // panic: end byte index 100 is not a char boundary

        // The fix: take chars, not bytes → safe
        let preview: String = text.chars().take(100).collect();
        assert!(!preview.is_empty());
        assert!(preview.contains("没")); // "没" is preserved fully

        // Sanity: slicing at actual char boundary never panics
        let _ = &preview[..preview.len()];
    }

    #[test]
    fn test_char_slice_preserves_full_characters() {
        let text = "你好世界Hello";
        let preview: String = text.chars().take(4).collect();
        assert_eq!(preview, "你好世界");
    }

    #[test]
    fn test_char_slice_shorter_than_text() {
        let text = "short";
        let preview: String = text.chars().take(100).collect();
        assert_eq!(preview, "short");
    }

    /// Verifies the empty-response detection heuristic.
    ///
    /// When text is blank, tool_calls is empty, and tokens_used > 0 (the model
    /// consumed prompt tokens but produced nothing), the executor injects a
    /// system hint on the first empty response and falls back to the last
    /// tool result on the second consecutive empty response.
    #[test]
    fn test_empty_response_heuristic() {
        fn mk(
            text: &str,
            tool_calls: Vec<oben_models::TransportToolCall>,
            tokens: Option<usize>,
        ) -> oben_models::TransportResponse {
            oben_models::TransportResponse {
                text: text.to_string(),
                tool_calls,
                tokens_used: tokens,
            }
        }

        // Non-empty response with tokens → NOT empty
        let resp = mk("Hello", vec![], Some(100));
        let is_empty = resp.text.trim().is_empty()
            && resp.tool_calls.is_empty()
            && resp.tokens_used.unwrap_or(0) > 0;
        assert!(!is_empty, "Normal response should not be flagged as empty");

        // Empty text, no tool calls, but tokens > 0 → IS empty
        let resp = mk("", vec![], Some(100));
        let is_empty = resp.text.trim().is_empty()
            && resp.tool_calls.is_empty()
            && resp.tokens_used.unwrap_or(0) > 0;
        assert!(is_empty, "Empty response with tokens should be flagged");

        // Zero/None tokens → model produced nothing (likely error), NOT empty-flagged
        let resp = mk("", vec![], None);
        let is_empty = resp.text.trim().is_empty()
            && resp.tool_calls.is_empty()
            && resp.tokens_used.unwrap_or(0) > 0;
        assert!(
            !is_empty,
            "Response with no tokens should not be flagged (likely error)"
        );

        // Response with tool calls → not empty (even if text is blank)
        let resp = mk(
            "",
            vec![oben_models::TransportToolCall {
                id: "tc1".into(),
                tool_name: "terminal".into(),
                arguments: serde_json::json!({"command": "ls"}),
            }],
            Some(100),
        );
        let is_empty = resp.text.trim().is_empty()
            && resp.tool_calls.is_empty()
            && resp.tokens_used.unwrap_or(0) > 0;
        assert!(
            !is_empty,
            "Response with tool calls should not be flagged as empty"
        );

        // Whitespace-only text with tokens → empty
        let resp = mk("   \n  ", vec![], Some(50));
        let is_empty = resp.text.trim().is_empty()
            && resp.tool_calls.is_empty()
            && resp.tokens_used.unwrap_or(0) > 0;
        assert!(is_empty, "Whitespace-only text should be flagged as empty");
    }
}
