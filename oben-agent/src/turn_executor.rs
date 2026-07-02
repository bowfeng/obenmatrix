/// Turn executor — deep module for the core agent turn cycle.
///
/// Encapsulates: compression → LLM call → termination policy → remedy policy → tool dispatch.
use std::sync::Arc;

use anyhow::Result;
use serde_json::Value;
use tracing::info;

use crate::coordinator::termination::{
    BudgetRemedyPolicy, DefaultTurnTerminationPolicy, EmptyResponseRemedyPolicy,
    TurnRemedyAction, TurnRemedyPolicy, TurnRemedyPolicyGroup, TurnTerminationDecision,
    TurnTerminationPolicy, TurnTerminationPolicyGroup,
};
use crate::concurrent_dispatch::{self, ConcurrentDispatchConfig, PendingToolCall};
use crate::context::{CompactStatus, ContextWindowManager};
use crate::fallback::FallbackChain;
use crate::hooks::HookEngine;
// shared interrupt handled via __INTERRUPT__: marker in messages
use crate::message_sanitize::sanitize_messages;
use crate::retry::{retry_with_backoff, RetryConfig};
use crate::stream_processor;
use oben_models::{
    Message, MessageContent, MessageRole, Session, SessionManager, StreamDeltaCallback,
    StreamReasoningCallback, TransportProvider,
};

// ---------------------------------------------------------------------------
// TurnConfig
// ---------------------------------------------------------------------------

pub struct TurnConfig {
    pub retry_config: RetryConfig,
    pub hooks: Option<Arc<HookEngine>>,
    pub fallback_chain: Option<FallbackChain>,
    pub dispatch_config: Option<ConcurrentDispatchConfig>,
    pub max_iterations: usize,
}

impl Default for TurnConfig {
    fn default() -> Self {
        Self {
            retry_config: RetryConfig::default(),
            hooks: None,
            fallback_chain: None,
            dispatch_config: None,
            max_iterations: 50,
        }
    }
}

// ---------------------------------------------------------------------------
// TurnResult
// ---------------------------------------------------------------------------

pub struct TurnResult {
    pub text: String,
    pub reason: TurnResultReason,
    pub turn_count: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TurnResultReason {
    Normal,
    ToolResult,
    BudgetExhausted,
}

enum CompactionResult {
    Continue,
    Rotated(String, Vec<Message>),
    Complete(()),
}

    // ---------------------------------------------------------------------------
    // TurnExecutor
    // ---------------------------------------------------------------------------

pub struct TurnExecutor;

impl TurnExecutor {
    /// Execute one turn with default policies.
    pub async fn execute_turn(
        context_window_manager: &mut dyn ContextWindowManager,
        transport: &dyn TransportProvider,
        tools: &Arc<oben_tools::ToolRegistry>,
        session_manager: &mut dyn SessionManager,
        session_id: &str,
        user_message: Message,
        call_mode: &oben_models::CallMode,
    ) -> Result<TurnResult> {
        Self::execute_turn_with_config(
            context_window_manager,
            transport,
            tools,
            session_manager,
            session_id,
            user_message,
            call_mode,
            None,
            None,
            TurnConfig::default(),
        )
        .await
    }

    /// Execute one turn with configurable termination/remedy policy groups.
    pub async fn execute_turn_with_config(
        context_window_manager: &mut dyn ContextWindowManager,
        transport: &dyn TransportProvider,
        tools: &Arc<oben_tools::ToolRegistry>,
        session_manager: &mut dyn SessionManager,
        session_id: &str,
        user_message: Message,
        call_mode: &oben_models::CallMode,
        termination_policy: Option<TurnTerminationPolicyGroup>,
        remedy_policy: Option<TurnRemedyPolicyGroup>,
        mut config: TurnConfig,
    ) -> Result<TurnResult> {
        let (term, mut rem) = match (termination_policy, remedy_policy) {
            (Some(t), Some(r)) => (t, r),
            (Some(t), None) => {
                let rem = Self::build_default_remedy(config.max_iterations);
                (t, rem)
            }
            (None, Some(r)) => {
                let term = Self::build_default_termination();
                (term, r)
            }
            (None, None) => {
                let term = Self::build_default_termination();
                let rem = Self::build_default_remedy(config.max_iterations);
                (term, rem)
            }
        };

        let (mut session, mut current_session_id, mut turn_count) =
            Self::pre_turn_setup(context_window_manager, session_manager, session_id, user_message, &mut config)?;
        let mut consecutive_empty: u32 = 0;
        #[allow(unused_assignments)]
        let mut decision_result: Option<TurnResult> = None;
        'turn_loop: loop {
            turn_count += 1;

            // Interrupt
            if let Some(r) = Self::check_interrupt(&session, turn_count) {
                decision_result = Some(r);
                break 'turn_loop;
            }

            // Compaction
            match Self::run_compaction(
                context_window_manager,
                &mut session,
                &mut current_session_id,
                session_manager,
                transport,
            )
            .await?
            {
                CompactionResult::Continue => {}
                CompactionResult::Rotated(child_id, compacted) => {
                    let (id, name, created_at, updated_at, memory_context, summary_chunks, persisted_message_count, metadata) = {
                        let s = session_manager.session_mut(&child_id).ok_or_else(|| {
                            anyhow::anyhow!("Child session disappeared: {}", child_id)
                        })?;
                        (s.id.clone(), s.name.clone(), s.created_at, s.updated_at,
                         s.memory_context.clone(), s.summary_chunks.clone(),
                         s.persisted_message_count, s.metadata.clone())
                    };
                    session.id = id;
                    session.name = name;
                    session.created_at = created_at;
                    session.updated_at = updated_at;
                    session.memory_context = memory_context;
                    session.summary_chunks = summary_chunks;
                    session.persisted_message_count = persisted_message_count;
                    session.metadata = metadata;
                    session.messages = compacted;
                }
                CompactionResult::Complete(_) => {
                    decision_result = Some(TurnResult { text: String::new(), reason: TurnResultReason::BudgetExhausted, turn_count });
                    break;
                }
            }

            // Sanitize
            sanitize_messages(&mut session.messages);

            // API call
            let response = Self::api_call_with_retry(
                transport,
                &session.messages,
                call_mode,
                &config,
            )
                .await?;
            let tool_calls = response.tool_calls.clone();

            // Process response: scrub, update tokens, inject assistant msg
            Self::process_response(
                &response,
                &tool_calls,
                &mut session,
                &current_session_id,
                &mut consecutive_empty,
                context_window_manager,
            )?;

            // Evaluate termination policy
            let decision = term.evaluate(&crate::coordinator::termination::TurnTerminationContext {
                response: &response,
                messages: &session.messages,
            })?;

            // Enact decision
            match decision {
                TurnTerminationDecision::Continue => {
                    // Phase 2: remedy check
                    let action = rem
                        .evaluate(config.max_iterations, &mut session.messages, consecutive_empty)?;
                    match action {
                        TurnRemedyAction::Continue => {
                            Self::dispatch_tool_results(
                                tools,
                                &tool_calls,
                                &mut session,
                                &current_session_id,
                                &config,
                            )
                            .await?;
                        }
                        TurnRemedyAction::Remedy => {
                            // Hint injected into messages, loop continues
                        }
                        TurnRemedyAction::RemedyExhausted => {
                            let last = Self::last_tool_result_text(&session.messages).unwrap_or_default();
                            decision_result = Some(TurnResult { text: last.to_string(), reason: TurnResultReason::BudgetExhausted, turn_count });
                            break;
                        }
                    }
                }
                TurnTerminationDecision::ReturnLastToolResult => {
                    if let Some(last) = Self::last_tool_result_text(&session.messages) {
                        decision_result = Some(TurnResult { text: last.to_string(), reason: TurnResultReason::ToolResult, turn_count });
                        break;
                    }
                }
                TurnTerminationDecision::Return(text) => {
                    decision_result = Some(TurnResult { text, reason: TurnResultReason::Normal, turn_count });
                    break;
                }
            }
        }

        // Write final messages back to the session manager. All mutations in the main loop
        // (process_response, dispatch_tool_results) operate on the local clone created in
        // pre_turn_setup. We must sync them back so the TUI can read them after the turn.
        let msg_count = session.messages.len();
        if let Some(s) = session_manager.session_mut(&current_session_id) {
            s.messages = session.messages;
            s.metadata.message_count = msg_count;
            s.metadata.tool_call_count = session.metadata.tool_call_count;
            s.metadata.input_tokens = session.metadata.input_tokens;
            s.metadata.output_tokens = session.metadata.output_tokens;
            s.metadata.total_tokens = session.metadata.total_tokens;
            s.metadata.estimated_cost_usd = session.metadata.estimated_cost_usd;
            s.metadata.turn_count = turn_count;
        }
        let _ = session_manager.incremental_save(Some(&current_session_id));

        decision_result.ok_or_else(|| anyhow::anyhow!("Turn loop exited without a decision"))
    }

    // ------------------------------------------------------------------
    // Private helpers
    // ------------------------------------------------------------------

    fn build_default_termination() -> TurnTerminationPolicyGroup {
        let mut group = TurnTerminationPolicyGroup::new();
        group.add_policy(Box::new(DefaultTurnTerminationPolicy::default()));
        group
    }

    fn build_default_remedy(max_iterations: usize) -> TurnRemedyPolicyGroup {
        // BudgetRemedyPolicy::remedyed is per-turn state; EmptyResponseRemedyPolicy
        // only reads max_consecutive (no mutable state). Creating fresh group
        // per turn is cheap — both constructors are simple struct initialization.
        TurnRemedyPolicyGroup::new()
            .with_policy(Box::new(BudgetRemedyPolicy::new(max_iterations)))
            .with_policy(Box::new(EmptyResponseRemedyPolicy::new(3)))
    }

    fn pre_turn_setup(
        context_window_manager: &mut dyn ContextWindowManager,
        session_manager: &mut dyn SessionManager,
        session_id: &str,
        user_message: Message,
        config: &mut TurnConfig,
     ) -> Result<(Session, String, u32)> {
        let mut current_id = session_id.to_string();

        // Use whichever session ID the CWM considers active as the primary.
        // This may differ from the input `session_id` if the CWM already
        // switched to a child session via split_before_turn.
        let active_id = context_window_manager
            .session_id()
            .unwrap_or_else(|| session_id.to_string());

        if let Some(new_id) = context_window_manager.should_do_time_based_split(session_manager) {
            current_id = new_id;
        }

        // Prefer CWM's active session ID if available. This ensures we always
        // have a valid session to work with, even if time-based split generated
        // a name that isn't yet persisted to the store.
        let id = {
            // Try the CWM's session first (most likely to be valid)
            match session_manager.session(&active_id) {
                Some(s) => s.id.clone(),
                None => {
                    // Fall back to get_or_create (lazily creates if needed)
                    session_manager.get_or_create_session(&current_id).id.clone()
                }
            }
        };
        // Ensure the CWM is synced to the actual session we're using.
        context_window_manager.set_active_session(session_manager, id.clone());

        let session_ref = session_manager
            .session_mut(&id)
            .ok_or_else(|| anyhow::anyhow!("Session missing after resolution: {}", id))?;

        session_ref.messages.push(user_message);
        context_window_manager.on_message_received(chrono::Utc::now());

        if let Some(ref hooks) = config.hooks {
            hooks.emit_status("lifecycle", "turn_start");
        }

        let session = Session {
            id: session_ref.id.clone(),
            name: session_ref.name.clone(),
            created_at: session_ref.created_at,
            updated_at: session_ref.updated_at,
            messages: session_ref.messages.clone(),
            memory_context: session_ref.memory_context.clone(),
            summary_chunks: session_ref.summary_chunks.clone(),
            persisted_message_count: session_ref.persisted_message_count,
            metadata: session_ref.metadata.clone(),
        };

        let turn_count = session.metadata.turn_count;

        Ok((session, id, turn_count))
    }

    fn check_interrupt(session: &Session, turn_count: u32) -> Option<TurnResult> {
        session.messages.iter().find_map(|m| {
            if let MessageContent::Text(ref t) = m.content {
                if t.starts_with("__INTERRUPT__:") {
                    return Some(TurnResult { text: String::new(), reason: TurnResultReason::Normal, turn_count });
                }
            }
            None
        })
    }

    async fn api_call_with_retry(
        transport: &dyn TransportProvider,
        messages: &[Message],
        call_mode: &oben_models::CallMode,
        config: &TurnConfig,
    ) -> Result<oben_models::TransportResponse> {
        let hooks = config.hooks.as_ref().map(|cb| Arc::clone(cb));
        let messages = messages.to_vec();
        let mode = call_mode.clone();

        retry_with_backoff(&config.retry_config, move || {
            let messages = messages.clone();
            let mode = mode.clone();
            let hooks = hooks.clone();

            async move {
                if let Some(ref hooks) = hooks {
                    hooks.emit_status("lifecycle", "api_call_start");
                }

                // Create reasoning delta callback for API-level reasoning_content.
                let reasoning_cb: Option<StreamReasoningCallback> = if let Some(ref hooks) = hooks {
                    let hooks = Arc::clone(hooks);
                    Some(Box::new(move |reasoning: &str| {
                        hooks.emit_reasoning(reasoning);
                    }))
                } else {
                    None
                };

                // Create a stateful scrubber so thinking blocks split across
                // deltas accumulate reasoning correctly.
                let mut scrubber = crate::stream_processor::StreamingThinkScrubber::new();

                let cb: StreamDeltaCallback = Box::new(move |delta: &str| {
                    if let Some(ref hooks) = hooks {
                        let scrubbed = scrubber.scrub_delta(delta);

                        // Also emit extracted reasoning from thinking blocks
                        // when the entire open+close arrives in one delta.
                        let (_, reasoning) =
                            crate::stream_processor::scrub_thinking_blocks(delta);
                        if let Some(text) = reasoning {
                            hooks.emit_reasoning(&text);
                        }

                        if !scrubbed.is_empty() {
                            hooks.emit_stream_delta(&scrubbed);
                        }
                    }
                });
                transport
                    .stream_chat(&messages, &mode, cb, reasoning_cb)
                    .await
                    .map_err(|e| e.into())
            }
        })
        .await
    }

    async fn run_compaction(
        ctx: &mut dyn ContextWindowManager,
        session: &mut Session,
        current_id: &mut String,
        session_mgr: &mut dyn SessionManager,
        transport: &dyn TransportProvider,
    ) -> Result<CompactionResult> {
        if !ctx.should_compact(&session.messages) {
            return Ok(CompactionResult::Continue);
        }

        info!(
            "Compacting ({} msgs, {} tokens)",
            session.messages.len(),
            ctx.estimate_tokens(&session.messages)
        );

        let status = ctx.compact(&mut session.messages, Some(transport), None).await?;
        if status == CompactStatus::Unchanged {
            return Ok(CompactionResult::Continue);
        }

        match Self::rotate_session(ctx, &mut session.messages, current_id, session_mgr) {
            Ok((cid, msgs)) => Ok(CompactionResult::Rotated(cid, msgs)),
            Err(e) => {
                tracing::warn!("Rotation failed: {e}");
                Ok(CompactionResult::Complete(()))
            }
        }
    }

    fn rotate_session(
        ctx: &mut dyn ContextWindowManager,
        msgs: &mut Vec<Message>,
        current_id: &mut String,
        session_mgr: &mut dyn SessionManager,
    ) -> Result<(String, Vec<Message>)> {
        let parent_id = current_id.clone();
        let compacted = msgs.clone();

        let child = session_mgr.split_after_compression(&parent_id).map_err(|e| anyhow::anyhow!("Split failed: {}", e))?;
        let child_id = child.id.clone();

        let parent_msgs = {
            let p = session_mgr.session(&parent_id).ok_or_else(|| anyhow::anyhow!("Parent gone: {}", parent_id))?;
            p.messages.clone()
        };
        session_mgr.save_compacted(&child_id, &parent_msgs).map_err(|e| anyhow::anyhow!("Persist failed: {}", e))?;

        *current_id = child_id.clone();
        ctx.on_session_split(session_mgr, child_id.clone());
        Ok((child_id, compacted))
    }

    fn process_response(
        response: &oben_models::TransportResponse,
        tool_calls: &[oben_models::TransportToolCall],
        session: &mut Session,
        _session_id: &str,
        consecutive_empty: &mut u32,
        context_window_manager: &mut dyn ContextWindowManager,
    ) -> Result<()> {
        let (scrubbed, scrubbed_reasoning) = stream_processor::scrub_thinking_blocks(&response.text);
        let combined_reasoning: Option<String> = match (&response.reasoning, scrubbed_reasoning) {
            (Some(a), Some(s)) => {
                let mut c = a.clone();
                c.push_str("\n\n");
                c.push_str(&s);
                Some(c)
            }
            (Some(a), None) => Some(a.clone()),
            (None, Some(s)) => Some(s),
            (None, None) => None,
        };
        let scrubbed = stream_processor::scrub_memory_context(&scrubbed);

        if let Some(tokens) = response.tokens_used {
            context_window_manager.update_from_response(tokens, 0, tokens);
        }

        let is_empty = scrubbed.trim().is_empty() && tool_calls.is_empty() && response.tokens_used.unwrap_or(0) > 0;
        if is_empty {
            *consecutive_empty += 1;
        } else {
            *consecutive_empty = 0;
        }

        let assistant = if !tool_calls.is_empty() {
            let mut msg = Message::assistant_tool_calls(tool_calls.iter().map(oben_models::ToolCall::from_transport).collect());
            msg.reasoning = combined_reasoning;
            msg
        } else if !is_empty {
            let mut msg = Message::assistant(scrubbed.trim().to_string());
            msg.reasoning = combined_reasoning;
            msg
        } else {
            // LLM returned empty text with no tool calls — skip to avoid
            // persisting blank assistant rows that pollute context.
            // consecutive_empty counter is already updated above.
            return Ok(());
        };
        session.messages.push(assistant);

        Ok(())
    }

    pub(crate) fn last_tool_result_text(messages: &[Message]) -> Option<&str> {
        messages.last().and_then(|m| {
            if m.role == MessageRole::Tool {
                m.content.to_text_ref()
            } else {
                None
            }
        })
    }

    async fn dispatch_tool_results(
        tools: &Arc<oben_tools::ToolRegistry>,
        tool_calls: &[oben_models::TransportToolCall],
        session: &mut Session,
        session_id: &str,
        config: &TurnConfig,
    ) -> Result<()> {
        let default_dispatch = ConcurrentDispatchConfig::default();
        let dispatch_config = config
            .dispatch_config
            .as_ref()
            .unwrap_or(&default_dispatch);

        let mut delegation_counter: u32 = 0;
        let pending: Vec<PendingToolCall> = tool_calls
            .iter()
            .enumerate()
            .map(|(_idx, c)| {
                let mut args = c.arguments.clone();
                if c.tool_name == "delegate_task" {
                    if let Some(obj) = args.as_object_mut() {
                        obj.entry("parent_session_id").or_insert_with(|| {
                            serde_json::Value::String(session_id.to_string())
                        });
                        // Inject a unique delegation_id into call args so result messages
                        // can be grouped by subagent in the TUI.
                        if !obj.contains_key("delegation_id") {
                            obj.insert("delegation_id".into(), Value::Number(delegation_counter.into()));
                            delegation_counter += 1;
                        }
                    }
                }
                PendingToolCall {
                    tool_name: c.tool_name.clone(),
                    arguments: args,
                    call_id: c.id.clone(),
                }
            })
            .collect();

        if let Some(ref hooks) = config.hooks {
            for call in &pending {
                hooks.emit_tool_gen(&call.tool_name, &call.call_id);
                hooks.emit_tool_start(&call.tool_name, &call.arguments.to_string());
            }
        }

        let results = concurrent_dispatch::dispatch_tool_calls(tools, dispatch_config, &pending, None).await?;

        for (i, result) in results.iter().enumerate() {
            let call = &pending[i];
            if result.output.is_empty() && call.call_id != "steer" && result.error.is_none() {
                continue;
            }
            // Derive delegation_id for delegate_tool messages from the call
            // arguments — the agent passes `delegation_id` alongside `goal`/`tasks`.
            let delegation_id = if call.tool_name == "delegate_task" {
                call.arguments
                    .get("delegation_id")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u32)
            } else {
                None
            };
            let msg = if !result.output.is_empty() {
                Message::tool_result(&call.call_id, &result.output)
                    .with_delegation_id(delegation_id.unwrap_or(0))
            } else if let Some(ref err) = result.error {
                Message::tool_result(&call.call_id, err)
                    .with_delegation_id(delegation_id.unwrap_or(0))
            } else {
                Message {
                    role: MessageRole::Tool,
                    content: MessageContent::Text(String::new()),
                    id: None,
                    tool_call_ids: vec![call.call_id.clone()],
                    tool_calls: None,
                    reasoning: None,
                    delegation_id,
                }
            };
            session.messages.push(msg);

            if let Some(ref hooks) = config.hooks {
                if let Some(ref err) = result.error {
                    hooks.emit_tool_error(&call.tool_name, &call.arguments.to_string(), err);
                } else {
                    hooks.emit_tool_complete(&call.tool_name, &call.arguments.to_string(), &result.output);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn test_utf8_char_slice_does_not_panic_with_chinese() {
        let text = "\n\n有一天，一块三分熟的牛排在街上走着，突然看到一块五分熟的牛排，却没有打招呼。\n为什么？\n因为他们**不熟**。😄\n\n还想听程序员专属笑话，还是日常冷笑话？随时点单～";
        assert!(text.len() > 100);
        let preview: String = text.chars().take(100).collect();
        assert!(!preview.is_empty());
        assert!(preview.contains("没"));
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

    #[test]
    fn test_empty_response_heuristic() {
        fn mk(text: &str, tools: Vec<oben_models::TransportToolCall>, tokens: Option<usize>) -> oben_models::TransportResponse {
            oben_models::TransportResponse {
                text: text.into(),
                tool_calls: tools,
                tokens_used: tokens,
                reasoning: None,
            }
        }

        let r = mk("Hello", vec![], Some(100));
        let is_empty = r.text.trim().is_empty() && r.tool_calls.is_empty() && r.tokens_used.unwrap_or(0) > 0;
        assert!(!is_empty);

        let r = mk("", vec![], Some(100));
        let is_empty = r.text.trim().is_empty() && r.tool_calls.is_empty() && r.tokens_used.unwrap_or(0) > 0;
        assert!(is_empty);

        let r = mk("", vec![], None);
        let is_empty = r.text.trim().is_empty() && r.tool_calls.is_empty() && r.tokens_used.unwrap_or(0) > 0;
        assert!(!is_empty);
    }
}
