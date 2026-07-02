/// Context compression — summarization to manage long conversations.
///
/// Full session compaction that:
/// 1. Prunes old tool results (cheap pre-pass, no LLM call)
/// 2. Protects head messages (system prompt + first N)
/// 3. Protects tail messages by token budget (recent context)
/// 4. Summarizes middle turns with LLM
/// 5. Iteratively updates previous summaries
/// 6. Sanitizes orphaned tool_call/tool_result pairs
///
/// Maps to `agent/context_compressor.py` in the Hermes Python agent.
///
/// The `ContextWindowManager` in `context.rs` is the unified entry point that owns
/// the message buffer, tracks token usage, decides when to compress,
/// and calls the functions in this module.
use anyhow::Result;

use oben_models::{Message, MessageContent, MessagePart, TransportProvider};

// ---------------------------------------------------------------------------
// CompactCofig — parameters for the full compaction algorithm
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct CompactCofig {
    /// Total context window size in tokens.
    pub context_length: usize,
    /// Token threshold as a percentage of context_length (e.g. 0.75 = 75%).
    pub threshold_percent: f64,
    /// Number of non-system head messages to protect.
    pub protect_first_n: usize,
    /// Token budget for the tail — walk backward accumulating tokens.
    /// Default: ~20K tokens (scales with model context).
    pub tail_token_budget: usize,
    /// Hard minimum: always protect at least this many messages in the tail.
    pub tail_min_messages: usize,
    /// Soft ceiling multiplier — allow budget to exceed by this factor to
    /// avoid cutting inside an oversized message.
    pub tail_overhead: f64,
    /// Min tokens for the initial summary.
    pub min_summary_tokens: usize,
    /// Max tokens for the initial summary.
    pub max_summary_tokens: usize,
    /// Max tokens for iterated summaries.
    pub iterated_max_tokens: usize,
    /// Min tokens for iterated summaries.
    pub iterated_min_tokens: usize,
    /// Max tokens for the final combined summary.
    pub final_summary_max_tokens: usize,
    /// Max tool result tokens to keep before pruning.
    pub max_tool_result_tokens: usize,
    /// Min percentage savings for a compression to be considered effective.
    pub ineffective_threshold: f64,
    /// Max consecutive ineffective compressions before anti-thrashing kicks in.
    pub max_ineffective_consecutive: usize,
}

impl Default for CompactCofig {
    fn default() -> Self {
        Self {
            context_length: 128_000,
            threshold_percent: 0.75,
            protect_first_n: 3,
            tail_token_budget: 20_000,
            tail_min_messages: 3,
            tail_overhead: 1.5,
            min_summary_tokens: 2000,
            max_summary_tokens: 4000,
            iterated_max_tokens: 3000,
            iterated_min_tokens: 1000,
            final_summary_max_tokens: 2500,
            max_tool_result_tokens: 10000,
            ineffective_threshold: 10.0,
            max_ineffective_consecutive: 2,
        }
    }
}

impl CompactCofig {
    /// Derive the compression threshold in tokens from the current
    /// context_length and threshold_percent.
    pub fn threshold_tokens(&self) -> usize {
        (self.context_length as f64 * self.threshold_percent) as usize
    }

    /// Derive threshold tokens from a given context length using the
    /// current threshold_percent.
    pub fn threshold_tokens_for(&self, context_length: usize) -> usize {
        (context_length as f64 * self.threshold_percent) as usize
    }
}

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct CompressionStats {
    pub original_count: usize,
    pub compacted_count: usize,
    pub original_tokens: usize,
    pub compacted_tokens: usize,
    pub savings_pct: f64,
    pub pruned_tool_results: usize,
    pub summary_generated: bool,
}

pub struct CompressionResult {
    pub messages: Vec<Message>,
    pub stats: CompressionStats,
    pub summary: Option<String>,
}

/// Outcome of a manual `compact_session` operation — distinguishes the
/// different reasons why compaction may not change the message list.
#[derive(Debug, Clone)]
pub enum CompactOutcome {
    /// Session messages are within token budget — nothing to compact.
    AlreadyCompact,
    /// All messages are protected (head/tail) — no middle messages to summarize.
    NoMiddleMessages {
        head_count: usize,
        tail_count: usize,
    },
    /// Compression attempted but savings below threshold — messages unchanged.
    Ineffective {
        original_tokens: usize,
        compacted_tokens: usize,
        savings_pct: f64,
    },
    /// Compression succeeded — messages were replaced with a summary.
    Compressed {
        original_count: usize,
        compacted_count: usize,
        savings_pct: f64,
    },
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Compact a session's message list.
///
/// This is the full compaction algorithm. It returns a new message list with
/// the compacted/summarized version of the original.
///
/// # Arguments
/// * `messages` — The original message list to compress
/// * `config` — Compression parameters
/// * `previous_summary` — Optional previous summary for iterative updates
/// * `focus_topic` — Optional topic string to guide the summary
/// * `compression_round` — Round number for the iterative update process
/// Compact a session's message list.
///
/// This is the full compaction algorithm. It returns a new message list with
/// the compacted/summarized version of the original.
///
/// # Arguments
/// * `transport` — LLM transport for generating summaries (must be OpenAI-compatible)
/// * `messages` — The original message list to compress
/// * `config` — Compression parameters
/// * `previous_summary` — Optional previous summary for iterative updates
/// * `focus_topic` — Optional topic string to guide the summary
/// * `compression_round` — Round number for the iterative update process
pub async fn compact_session_messages(
    transport: &dyn TransportProvider,
    messages: &[Message],
    config: &CompactCofig,
    previous_summary: Option<&str>,
    focus_topic: Option<&str>,
    _compression_round: usize,
) -> Result<CompressionResult> {
    // Step 1: Token estimation — computed once, reused
    let original_tokens = messages
        .iter()
        .map(|m| message_token_estimate(m))
        .sum::<usize>();
    let original_count = messages.len();

    // Step 2: Prune tool results
    let (pruned, pruned_count) = prune_tool_results(messages, config.max_tool_result_tokens);

    // Step 3: Split into head/tail/middle
    let (head, middle, tail) = split_messages(&pruned, config);

    // Early return: no middle messages means nothing to summarize.
    // Avoid wasting an LLM call — return 0% savings (compact == original).
    if middle.is_empty() {
        tracing::info!(
            "No middle messages to compress (head={}, tail={}), skipping LLM call",
            head.len(),
            tail.len()
        );
        let compacted_count = pruned.len();
        return Ok(CompressionResult {
            messages: pruned,
            stats: CompressionStats {
                original_count,
                compacted_count,
                original_tokens,
                compacted_tokens: original_tokens,
                savings_pct: 0.0,
                pruned_tool_results: pruned_count,
                summary_generated: false,
            },
            summary: None,
        });
    }

    // Step 4: Compute middle tokens once, pass to generate_summary
    let middle_tokens: usize = middle.iter().map(|m| message_token_estimate(m)).sum();
    let summary = {
        let summary_text = generate_summary(
            transport,
            &middle,
            previous_summary,
            focus_topic,
            config,
            middle_tokens,
        )
        .await?;
        Some(summary_text)
    };

    let summary_generated = summary.is_some();

    // Step 5: Build compacted message list
    let mut compacted = Vec::new();

    // Add head (protected verbatim)
    compacted.extend(head.iter().cloned());

    // Add summary if present
    if let Some(ref summary_text) = summary {
        compacted.push(Message::system(summary_text));
    }

    // Add tail (protected verbatim)
    compacted.extend(tail.iter().cloned());

    // Step 6: Sanitize orphaned tool_call/tool_result pairs
    let (removed_orphans, added_stubs) = sanitize_tool_pairs(&mut compacted);
    if removed_orphans > 0 || added_stubs > 0 {
        tracing::info!(
            "Sanitizer: removed {} orphaned tool result(s), added {} stub(s)",
            removed_orphans,
            added_stubs,
        );
    }

    // Step 7: Strip historical image content
    let stripped_media = strip_historical_media(&mut compacted);
    if stripped_media > 0 {
        tracing::info!(
            "Stripped {} image part(s) from historical messages",
            stripped_media
        );
    }

    let compacted_tokens = compacted
        .iter()
        .map(|m| message_token_estimate(m))
        .sum::<usize>();
    let compacted_count = compacted.len();

    let savings_pct = if original_tokens > 0 {
        ((original_tokens as f64 - compacted_tokens as f64) / original_tokens as f64 * 100.0)
            .round()
    } else {
        0.0
    };

    Ok(CompressionResult {
        messages: compacted,
        stats: CompressionStats {
            original_count,
            compacted_count,
            original_tokens,
            compacted_tokens,
            savings_pct,
            pruned_tool_results: pruned_count,
            summary_generated,
        },
        summary,
    })
}

pub fn message_token_estimate(msg: &Message) -> usize {
    let text = match &msg.content {
        MessageContent::Text(s) => s,
        MessageContent::Image { .. } => return 500,
        MessageContent::Parts(parts) => {
            return parts
                .iter()
                .map(|p| match p {
                    MessagePart::Text(s) => s.len() / 4,
                    MessagePart::Image { .. } => 500,
                })
                .sum();
        }
    };
    text.len() / 4 + 5 // per-message overhead
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn prune_tool_results(messages: &[Message], _max_tokens: usize) -> (Vec<Message>, usize) {
    let mut results = Vec::with_capacity(messages.len());
    let mut pruned_count = 0usize;

    // ---- Pass 1: Deduplicate by content ----
    // Keep only the newest (last) full copy, replace older duplicates
    // with a back-reference.
    let mut seen_contents: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    let mut duplicate_indices: std::collections::HashSet<usize> = std::collections::HashSet::new();

    for (i, msg) in messages.iter().enumerate() {
        if msg.role != oben_models::MessageRole::Tool {
            continue;
        }
        let content = match &msg.content {
            MessageContent::Text(s) => s.clone(),
            _ => continue,
        };
        if content.is_empty() {
            continue;
        }
        if let Some(&prev_idx) = seen_contents.get(&content) {
            // Mark earlier duplicate for replacement
            duplicate_indices.insert(prev_idx);
        }
        seen_contents.insert(content, i);
    }

    // Second pass: build message list with dedup replacements
    for (i, msg) in messages.iter().enumerate() {
        if duplicate_indices.contains(&i) {
            // Replace duplicate with back-reference
            let dup_msg = Message {
                role: oben_models::MessageRole::Tool,
                content: MessageContent::Text(
                    "[Duplicate tool output — same content as a more recent call]".into(),
                ),
                id: msg.id.clone(),
                tool_call_ids: msg.tool_call_ids.clone(),
                tool_calls: None,
                reasoning: None,
                delegation_id: msg.delegation_id,
            };
            results.push(dup_msg);
            pruned_count += 1;
        } else {
            results.push(msg.clone());
        }
    }

    // ---- Pass 2: Replace large tool results with 1-line summaries ----
    let max_output_len = 200;
    for msg in results.iter_mut() {
        if msg.role != oben_models::MessageRole::Tool {
            continue;
        }
        match &msg.content {
            MessageContent::Image { .. } => {
                // Replace image content with placeholder
                msg.content = MessageContent::Text("[screenshot removed to save context]".into());
                pruned_count += 1;
                continue;
            }
            MessageContent::Parts(parts) => {
                // Check for image parts
                let has_image = parts.iter().any(|p| matches!(p, MessagePart::Image { .. }));
                if has_image {
                    let text_parts: Vec<String> = parts
                        .iter()
                        .filter_map(|p| match p {
                            MessagePart::Text(s) => Some(s.clone()),
                            _ => None,
                        })
                        .collect();
                    if text_parts.is_empty() {
                        msg.content =
                            MessageContent::Text("[screenshot removed to save context]".into());
                    } else {
                        msg.content = MessageContent::Text(text_parts.join("\n"));
                    }
                    pruned_count += 1;
                    continue;
                }
            }
            _ => {}
        }

        // Check text content length
        let text_len = match &msg.content {
            MessageContent::Text(s) => s.len(),
            _ => 0,
        };

        if text_len > max_output_len {
            // Extract tool name from tool_call_ids (first ID is the parent call)
            let tool_name = msg
                .tool_call_ids
                .first()
                .map(|id| {
                    // Try to extract tool name from context — if available, use it
                    // Otherwise use a generic label
                    id.chars().take(20).collect::<String>()
                })
                .unwrap_or_else(|| "tool".to_string());

            // Create informative 1-line summary
            let summary = format!(
                "[{}] {} -> {} chars output (truncated)",
                tool_name,
                if text_len > 0 && msg.content.to_text().contains('\n') {
                    format!(
                        "{} lines output",
                        msg.content.to_text().matches('\n').count() + 1
                    )
                } else {
                    format!("{} chars output", text_len)
                },
                text_len
            );
            msg.content = MessageContent::Text(summary);
            pruned_count += 1;
        }
    }

    // ---- Pass 3: Truncate large tool_call.arguments in assistant messages ----
    let max_args_len = 500;
    for msg in results.iter_mut() {
        if msg.role != oben_models::MessageRole::Assistant {
            continue;
        }
        if let Some(ref mut tool_calls) = msg.tool_calls {
            for tc in tool_calls.iter_mut() {
                if tc.arguments.is_string() {
                    let args_str = tc.arguments.as_str().unwrap_or("");
                    if args_str.len() > max_args_len {
                        // Parse JSON and shrink string leaves, then re-serialize to string
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(args_str) {
                            let shrunk = shrink_json_strings(&json, max_args_len);
                            tc.arguments = serde_json::Value::String(
                                serde_json::to_string(&shrunk)
                                    .unwrap_or_else(|_| args_str.to_string()),
                            );
                            pruned_count += 1;
                        } else {
                            // If unparseable JSON, just truncate the raw string
                            tc.arguments = serde_json::Value::String(format!(
                                "{}... [truncated: {} chars]",
                                &args_str[..max_args_len.min(args_str.len())],
                                args_str.len()
                            ));
                            pruned_count += 1;
                        }
                    }
                }
            }
        }
    }

    // Return pruned count via tuple
    (results, pruned_count)
}

/// Strip historical image content from messages before the anchor.
///
/// The "anchor" is the newest user message that carries image content.
/// All messages before the anchor have their image parts replaced with
/// placeholders. Messages at/after the anchor are preserved verbatim.
fn strip_historical_media(messages: &mut Vec<Message>) -> usize {
    // Find the newest user message with image content (the anchor)
    let anchor_idx = messages
        .iter()
        .enumerate()
        .rev()
        .find(|(_, msg)| {
            msg.role == oben_models::MessageRole::User && has_image_content(&msg.content)
        })
        .map(|(idx, _)| idx);

    // If no anchor or anchor is the first message, nothing to strip
    let anchor_idx = match anchor_idx {
        Some(0) => return 0,
        Some(idx) => idx,
        None => return 0,
    };

    let mut stripped_count = 0;

    for msg in messages.iter_mut().take(anchor_idx) {
        let image_count = strip_images_from_content(&mut msg.content);
        if image_count > 0 {
            stripped_count += image_count;
        }
    }

    stripped_count
}

/// Check if a MessageContent contains any image parts.
fn has_image_content(content: &MessageContent) -> bool {
    match content {
        MessageContent::Image { .. } => true,
        MessageContent::Parts(parts) => {
            parts.iter().any(|p| matches!(p, MessagePart::Image { .. }))
        }
        MessageContent::Text(_) => false,
    }
}

/// Strip images from content, replacing with placeholders.
/// Returns count of images replaced.
fn strip_images_from_content(content: &mut MessageContent) -> usize {
    let mut count = 0;
    match content {
        MessageContent::Image { .. } => {
            *content = MessageContent::Text("[screenshot removed to save context]".into());
            count = 1;
        }
        MessageContent::Parts(parts) => {
            let mut new_parts = Vec::new();
            for part in parts.drain(..) {
                match part {
                    MessagePart::Image { .. } => {
                        new_parts.push(MessagePart::Text(
                            "[screenshot removed to save context]".into(),
                        ));
                        count += 1;
                    }
                    MessagePart::Text(t) => {
                        new_parts.push(MessagePart::Text(t));
                    }
                }
            }
            *content = MessageContent::Parts(new_parts);
        }
        MessageContent::Text(_) => {}
    }
    count
}

/// Sanitize orphaned tool_call/tool_result pairs after compression.
///
/// Two failure modes:
/// 1. Tool result references a call_id whose assistant tool_call was removed
/// 2. Assistant message has tool_calls whose results were dropped
fn sanitize_tool_pairs(messages: &mut Vec<Message>) -> (usize, usize) {
    let mut removed_orphaned_results = 0usize;
    let mut added_stub_results = 0usize;

    // Collect all call_ids from assistant tool_calls (these are "surviving")
    let mut surviving_call_ids: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    for msg in messages.iter() {
        if let Some(ref tool_calls) = msg.tool_calls {
            for tc in tool_calls {
                surviving_call_ids.insert(tc.id.clone());
            }
        }
    }

    // Collect call_ids from tool results to know which are "covered"
    let mut covered_call_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    messages.retain(|msg| {
        if msg.role != oben_models::MessageRole::Tool {
            return true;
        }
        for call_id in &msg.tool_call_ids {
            if surviving_call_ids.contains(call_id) {
                covered_call_ids.insert(call_id.clone());
                return true;
            }
        }
        removed_orphaned_results += 1;
        false
    });

    // Find orphaned tool_calls (no matching tool_result)
    let orphaned_call_ids: Vec<String> = surviving_call_ids
        .iter()
        .filter(|id| !covered_call_ids.contains(*id))
        .cloned()
        .collect();

    // Add stub results for orphaned calls
    for call_id in orphaned_call_ids {
        messages.push(Message::tool_result(
            &call_id,
            "[Result from earlier conversation — see context summary above]",
        ));
        added_stub_results += 1;
    }

    // Update assistant messages to remove orphaned tool_calls
    for msg in messages.iter_mut() {
        if let Some(ref mut tool_calls) = msg.tool_calls {
            tool_calls.retain(|tc| covered_call_ids.contains(&tc.id));
        }
    }

    (removed_orphaned_results, added_stub_results)
}

/// Shrink string values in a JSON tree to fit within max_chars limit.
/// Preserves JSON structure while truncating string leaves.
fn shrink_json_strings(value: &serde_json::Value, max_chars: usize) -> serde_json::Value {
    match value {
        serde_json::Value::String(s) => {
            if s.len() > max_chars {
                serde_json::Value::String(format!("{}...", &s[..max_chars.min(s.len())]))
            } else {
                value.clone()
            }
        }
        serde_json::Value::Object(map) => {
            let new_map: serde_json::Map<String, serde_json::Value> = map
                .iter()
                .map(|(k, v)| (k.clone(), shrink_json_strings(v, max_chars)))
                .collect();
            serde_json::Value::Object(new_map)
        }
        serde_json::Value::Array(arr) => {
            let new_arr: Vec<serde_json::Value> = arr
                .iter()
                .map(|v| shrink_json_strings(v, max_chars))
                .collect();
            serde_json::Value::Array(new_arr)
        }
        other => other.clone(),
    }
}

fn split_messages<'a>(
    messages: &'a [Message],
    config: &'a CompactCofig,
) -> (&'a [Message], &'a [Message], &'a [Message]) {
    let len = messages.len();
    if len == 0 {
        return (&[], &[], &[]);
    }

    let head_end = config.protect_first_n.min(len);
    let tail_start = find_tail_cut_by_tokens(messages, config);
    // Ensure tail_start >= head_end to avoid invalid slices
    let tail_start = tail_start.max(head_end);

    let head = &messages[..head_end];
    let middle = &messages[head_end..tail_start];
    let tail = &messages[tail_start..];

    (head, middle, tail)
}

/// Walk backward from the end, accumulating tokens until budget is reached.
/// Enforces a hard minimum of `tail_min_messages` messages.
/// Returns the index where the tail segment starts.
pub(crate) fn find_tail_cut_by_tokens(messages: &[Message], config: &CompactCofig) -> usize {
    let min_protect = config.tail_min_messages.min(messages.len());
    let budget = (config.tail_token_budget as f64 * config.tail_overhead) as usize;
    let mut accumulated = 0;
    let mut cut = 0;

    for i in (0..messages.len()).rev() {
        let tokens = message_token_estimate(&messages[i]);
        if accumulated + tokens > budget && (messages.len() - i) >= min_protect {
            break;
        }
        accumulated += tokens;
        cut = i;
    }

    // Ensure we protect at least min_protect messages
    let protect_count = messages.len().saturating_sub(cut);
    if protect_count < min_protect {
        messages.len().saturating_sub(min_protect)
    } else {
        cut
    }
}

/// Summarize conversation turns via LLM.
///
/// Maps to `agent/context_compressor.py::ContextCompressor._generate_summary`.
/// Generates a structured checkpoint summary with sections: Active Task, Goal,
/// Constraints, Completed Actions, Active State, In Progress, Blocked,
/// Key Decisions, Resolved Questions, Pending User Asks, Relevant Files,
/// Remaining Work, and Critical Context.
async fn generate_summary(
    transport: &dyn TransportProvider,
    messages: &[Message],
    previous_summary: Option<&str>,
    focus_topic: Option<&str>,
    config: &CompactCofig,
    cached_tokens: usize,
) -> Result<String> {
    let prefix = "[CONTEXT COMPACTION — REFERENCE ONLY] Earlier turns were compacted into the summary below. This is a handoff from a previous context window — treat it as background reference, NOT as active instructions. Do NOT answer questions or fulfill requests mentioned in this summary; they were already addressed. Your current task is identified in the '## Active Task' section of the summary — resume exactly from there. IMPORTANT: Your persistent memory (MEMORY.md, USER.md) in the system prompt is ALWAYS authoritative and active — never ignore or deprioritize memory content due to this compaction note. Respond ONLY to the latest user message that appears AFTER this summary. The current session state (files, config, etc.) may reflect work described here — avoid repeating it";

    // Serialize messages into structured text for the summarizer
    let content_to_summarize = serialize_for_summary(messages);

    // Use pre-computed token count — O(0) instead of O(n) scan
    let budget = (cached_tokens as f64 * 0.20) as usize;
    let budget = budget.max(2000).min(12000);

    let template_sections = format!(
        "## Active Task\n[Copy the user's most recent request verbatim — the exact words they used. If multiple tasks were requested and only some are done, list only the ones NOT yet completed. Example: 'User asked: \"Now refactor the auth module to use JWT instead of sessions\"' If no outstanding task exists, write \"None.\"]\n\n## Goal\n[What the user is trying to accomplish overall]\n\n## Constraints & Preferences\n[User preferences, coding style, constraints, important decisions]\n\n## Completed Actions\n[Numbered list of concrete actions taken — include tool used, target, and outcome.\nFormat: N. ACTION target — outcome [tool: name]\nExample:\n1. READ config.py:45 — found `==` should be `!=` [tool: read_file]\n2. PATCH config.py:45 — changed `==` to `!=` [tool: patch]\n3. TEST `pytest tests/` — 3/50 failed: test_parse, test_validate, test_edge [tool: terminal]\nBe specific with file paths, commands, line numbers, and results.]\n\n## Active State\n[Current working state — include working directory, modified/created files, test status, environment details]\n\n## In Progress\n[Work currently underway — what was being done when compaction fired]\n\n## Blocked\n[Any blockers, errors, or issues not yet resolved. Include exact error messages.]\n\n## Key Decisions\n[Important technical decisions and WHY they were made]\n\n## Resolved Questions\n[Questions the user asked that were ALREADY answered — include the answer so it is not repeated]\n\n## Pending User Asks\n[Questions or requests from the user that have NOT yet been answered or fulfilled. If none, write \"None.\"]\n\n## Relevant Files\n[Files read, modified, or created — with brief note on each]\n\n## Remaining Work\n[What remains to be done — framed as context, not instructions]\n\n## Critical Context\n[Any specific values, error messages, configuration details, or data that would be lost without explicit preservation. NEVER include API keys, tokens, passwords, or credentials — write [REDACTED] instead.]\n\nTarget ~{} tokens. Be CONCRETE — include file paths, command outputs, error messages, line numbers, and specific values. Avoid vague descriptions like \"made some changes\" — say exactly what changed.\n\nWrite only the summary body. Do not include any preamble or prefix.",
        budget
    );

    let preamble = "You are a summarization agent creating a context checkpoint. Treat the conversation turns below as source material for a compact record of prior work. Produce only the structured summary; do not add a greeting, preamble, or prefix. Write the summary in the same language the user was using in the conversation — do not translate or switch to English. NEVER include API keys, tokens, passwords, secrets, credentials, or connection strings in the summary — replace any that appear with [REDACTED]. Note that the user had credentials present, but do not preserve their values.";

    let prompt = match previous_summary {
        Some(prev) => format!(
            "{}\n\nYou are updating a context compaction summary. A previous compaction produced the summary below. New conversation turns have occurred since then and need to be incorporated.\n\nPREVIOUS SUMMARY:\n{}\n\nNEW TURNS TO INCORPORATE:\n{}\n\nUpdate the summary using this exact structure. PRESERVE all existing information that is still relevant. ADD new completed actions to the numbered list (continue numbering). Move items from \"In Progress\" to \"Completed Actions\" when done. Move answered questions to \"Resolved Questions\". Update \"Active State\" to reflect current state. Remove information only if it is clearly obsolete. CRITICAL: Update \"## Active Task\" to reflect the user's most recent unfulfilled request — this is the most important field for task continuity.\n\n{}",
            preamble, prev, content_to_summarize, template_sections
        ),
        None => format!(
            "{}\n\nCreate a structured checkpoint summary for the conversation after earlier turns are compacted. The summary should preserve enough detail for continuity without re-reading the original turns.\n\nTURNS TO SUMMARIZE:\n{}\n\nUse this exact structure:\n\n{}",
            preamble, content_to_summarize, template_sections
        ),
    };

    // Inject focus topic guidance
    let prompt = match focus_topic {
        Some(topic) => format!(
            "{}\n\nFOCUS TOPIC: \"{}\"\nThe user has requested that this compaction PRIORITISE preserving all information related to the focus topic above. For content related to \"{}\", include full detail — exact values, file paths, command outputs, error messages, and decisions. For content NOT related to the focus topic, summarise more aggressively (brief one-liners or omit if truly irrelevant). The focus topic sections should receive roughly 60-70% of the summary token budget. Even for the focus topic, NEVER preserve API keys, tokens, passwords, or credentials — use [REDACTED].",
            prompt, topic, topic
        ),
        None => prompt,
    };

    let summary_msg = Message::user(prompt);

    // Retry strategy: transient errors get retries, permanent errors fail fast
    let max_retries = 2;
    let mut last_error: Option<anyhow::Error> = None;

    for attempt in 0..=max_retries {
        if attempt > 0 {
            tracing::info!(
                "Summary generation attempt {} failed, retrying... ({})",
                attempt + 1,
                last_error.as_ref().unwrap()
            );
            tokio::time::sleep(std::time::Duration::from_millis(500 * attempt as u64)).await;
        }

        match transport
            .chat(
                &[summary_msg.clone()],
                &oben_models::CallMode::Fresh(String::new()),
            )
            .await
        {
            Ok(response) => {
                let summary = response.text.trim().to_string();
                if summary.is_empty() {
                    tracing::warn!(
                        "Summary generation via {} returned empty response",
                        transport.name()
                    );
                    let fallback = format!("{}: Empty summary — no content available.", prefix);
                    return Ok(fallback);
                }
                return Ok(format!("{}:{}", prefix, summary));
            }
            Err(e) => {
                let err_str = e.to_string().to_lowercase();
                let _is_transient = err_str.contains("timeout")
                    || err_str.contains("retry")
                    || err_str.contains("rate limit")
                    || err_str.contains("temporarily")
                    || err_str.contains("unavailable")
                    || err_str.contains("connection refused")
                    || err_str.contains("incomplete")
                    || err_str.contains("eof")
                    || err_str.contains("closed")
                    || err_str.contains("stream");
                let is_permanent = err_str.contains("400")
                    || err_str.contains("401")
                    || err_str.contains("403")
                    || err_str.contains("404")
                    || err_str.contains("invalid")
                    || err_str.contains("model not found")
                    || err_str.contains("does not exist");

                last_error = Some(e);

                if is_permanent {
                    tracing::warn!(
                        "Summary generation via {} failed with permanent error (attempt {}): {}",
                        transport.name(),
                        attempt + 1,
                        last_error.as_ref().unwrap()
                    );
                    break;
                }

                tracing::debug!(
                    "Summary generation via {} failed (attempt {}/{}, transient): {}",
                    transport.name(),
                    attempt + 1,
                    max_retries + 1,
                    last_error.as_ref().unwrap()
                );
            }
        }
    }

    // All retries exhausted or permanent error
    let err_msg = last_error
        .as_ref()
        .map(|e| e.to_string())
        .unwrap_or_else(|| "unknown".to_string());

    // Check abort mode: when max_tool_result_tokens is 0, signal abort_on_summary_failure=true
    if config.max_tool_result_tokens == 0 {
        tracing::warn!(
            "Summary generation aborted after {} attempts via {}: {}",
            max_retries + 1,
            transport.name(),
            err_msg
        );
        return Err(anyhow::anyhow!(
            "summary_generation_failed: abort_mode={}",
            err_msg
        ));
    }

    // Fall back to static summary
    let fallback = format!("{}: Earlier turns compacted. LLM summary generation failed ({} attempts). Resume from current system prompt and recent history.", prefix, max_retries + 1);
    tracing::warn!(
        "Summary generation failed after {} attempts via {}: {}. Returning static fallback.",
        max_retries + 1,
        transport.name(),
        err_msg
    );
    Ok(fallback)
}

/// Serialize conversation turns into structured text for the summarizer.
/// Maps to `agent/context_compressor.py::ContextCompressor._serialize_for_summary`.
fn serialize_for_summary(messages: &[Message]) -> String {
    let mut parts = Vec::new();
    const CONTENT_MAX: usize = 6000;
    const CONTENT_HEAD: usize = 4000;
    const CONTENT_TAIL: usize = 1500;

    for msg in messages {
        let role = match msg.role {
            oben_models::MessageRole::System => "SYSTEM",
            oben_models::MessageRole::User => "USER",
            oben_models::MessageRole::Assistant => "ASSISTANT",
            oben_models::MessageRole::Tool => "TOOL",
        };

        let content = match &msg.content {
            MessageContent::Text(s) => s.clone(),
            MessageContent::Image { .. } => "[image attached]".to_string(),
            MessageContent::Parts(parts) => {
                let texts: Vec<String> = parts
                    .iter()
                    .filter_map(|p| match p {
                        MessagePart::Text(s) => Some(s.clone()),
                        _ => None,
                    })
                    .collect();
                if texts.is_empty() {
                    "[multimodal content]".to_string()
                } else {
                    texts.join("\n")
                }
            }
        };

        let trimmed = if content.len() > CONTENT_MAX {
            format!(
                "{}\n...[truncated]...\n{}",
                &content[..CONTENT_HEAD],
                &content[content.len().saturating_sub(CONTENT_TAIL)..]
            )
        } else {
            content
        };

        let entry = format!("[{}]: {}", role, trimmed);
        parts.push(entry);
    }

    parts.join("\n\n")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use oben_models::Message;

    #[test]
    fn test_split_messages_with_token_budget() {
        let config = CompactCofig {
            context_length: 100_000,
            protect_first_n: 2,
            tail_token_budget: 100, // ~400 chars
            tail_min_messages: 2,
            tail_overhead: 1.5,
            ..Default::default()
        };

        // Messages: 10 messages, each ~20 chars
        let messages: Vec<Message> = (0..10)
            .map(|i| Message::user(format!("msg{}", i)))
            .collect();
        let (head, middle, tail) = split_messages(&messages, &config);

        // Head: first 2 messages
        assert_eq!(head.len(), 2);
        // Tail: last ~2 messages (within budget)
        assert!(tail.len() >= 2, "tail should have at least 2 messages");
        // Middle: remaining
        assert_eq!(middle.len(), messages.len() - head.len() - tail.len());
    }

    #[test]
    fn test_split_messages_enforces_tail_min_messages() {
        let config = CompactCofig {
            context_length: 100_000,
            protect_first_n: 2,
            tail_token_budget: 10, // Very small budget
            tail_min_messages: 3,
            tail_overhead: 1.5,
            ..Default::default()
        };

        let messages: Vec<Message> = (0..5).map(|i| Message::user(format!("msg{}", i))).collect();
        let (head, _middle, tail) = split_messages(&messages, &config);

        // Even with small budget, tail should have at least 3 messages
        assert_eq!(tail.len(), 3, "tail should enforce min_messages");
        assert_eq!(head.len(), 2);
    }

    #[test]
    fn test_split_messages_short_message_list() {
        let config = CompactCofig::default();

        // Fewer messages than head + tail protection
        let messages: Vec<Message> = (0..3).map(|i| Message::user(format!("msg{}", i))).collect();
        let (head, middle, tail) = split_messages(&messages, &config);

        // All messages should be in head (none in middle or tail)
        assert!(middle.len() == 0, "no middle for short list");
        assert!(head.len() + tail.len() >= 3, "all messages protected");
    }

    #[test]
    fn test_message_token_estimate_text() {
        let msg = Message::user("a".repeat(400));
        let tokens = message_token_estimate(&msg);
        assert_eq!(tokens, 105); // 400/4 + 5
    }

    #[test]
    fn test_message_token_estimate_image() {
        let msg = Message {
            role: oben_models::MessageRole::User,
            content: oben_models::MessageContent::Image {
                url: "https://example.com/img.jpg".to_string(),
                detail: None,
            },
            id: None,
            tool_call_ids: vec![],
            tool_calls: None,
            reasoning: None,
        };
        assert_eq!(message_token_estimate(&msg), 500);
    }

    #[test]
    fn test_prune_tool_results_deduplicates() {
        let msgs = vec![
            Message::user("hello"),
            Message::tool_result("call-1", "duplicate content"),
            Message::assistant("ok"),
            Message::tool_result("call-2", "duplicate content"), // duplicate
            Message::user("world"),
        ];
        let (pruned, count) = prune_tool_results(&msgs, 10000);
        assert_eq!(pruned.len(), msgs.len());
        assert_eq!(count, 1, "should detect 1 duplicate");

        // The earlier duplicate (index 1) should be replaced with back-reference
        let dup_msg = &pruned[1];
        assert_eq!(dup_msg.role, oben_models::MessageRole::Tool);
        assert!(dup_msg.content.to_text().contains("Duplicate tool output"));
        // The later one (index 3) should be preserved
        assert!(!pruned[3]
            .content
            .to_text()
            .contains("Duplicate tool output"));
    }

    #[test]
    fn test_prune_tool_results_truncates_large_outputs() {
        let long_content = "x".repeat(300);
        let msgs = vec![Message::tool_result("call-1", long_content.clone())];
        let (pruned, count) = prune_tool_results(&msgs, 10000);
        assert_eq!(count, 1, "should truncate 1 large output");
        assert!(pruned[0].content.to_text().contains("300 chars output"));
        assert!(pruned[0].content.to_text().len() < 300);
    }

    #[test]
    fn test_prune_tool_results_json_valid() {
        // Create an assistant message with tool calls containing long JSON args
        let long_json = format!("{{\"key\": \"{}\"}}", "a".repeat(600));
        let tool_call = oben_models::ToolCall {
            id: "call-1".to_string(),
            tool_name: "test_tool".to_string(),
            arguments: serde_json::Value::String(long_json),
        };
        let msgs = vec![Message {
            role: oben_models::MessageRole::Assistant,
            content: MessageContent::Text("calling tool".into()),
            id: None,
            tool_call_ids: vec![],
            tool_calls: Some(vec![tool_call]),
            reasoning: None,
        }];
        let (pruned, count) = prune_tool_results(&msgs, 10000);
        assert_eq!(count, 1);

        // Verify JSON is still valid
        let args = pruned[0].tool_calls.as_ref().unwrap()[0]
            .arguments
            .as_str()
            .unwrap();
        assert!(serde_json::from_str::<serde_json::Value>(args).is_ok());
    }

    #[test]
    fn test_sanitize_tool_pairs_removes_orphaned_results() {
        let assistant_call = oben_models::ToolCall {
            id: "call-1".to_string(),
            tool_name: "test".to_string(),
            arguments: serde_json::Value::String("{}".to_string()),
        };
        let assistant_call2 = oben_models::ToolCall {
            id: "call-2".to_string(),
            tool_name: "test".to_string(),
            arguments: serde_json::Value::String("{}".to_string()),
        };
        let msgs = vec![
            Message {
                role: oben_models::MessageRole::Assistant,
                content: MessageContent::Text("hi".into()),
                id: None,
                tool_call_ids: vec![],
                tool_calls: Some(vec![assistant_call, assistant_call2]),
                reasoning: None,
            },
            Message::tool_result("call-1", "result 1"),
            Message::tool_result("call-99", "orphaned result"), // orphaned
        ];
        let mut messages: Vec<Message> = msgs;
        let (removed, added) = sanitize_tool_pairs(&mut messages);
        assert_eq!(removed, 1, "should remove 1 orphaned result (call-99)");
        assert_eq!(
            added, 1,
            "should add 1 stub for call-2 (no matching result)"
        );
        // Only 1 tool result should remain (call-1 valid, call-99 removed, call-2 stub added)
        let tool_msgs: Vec<_> = messages
            .iter()
            .filter(|m| m.role == oben_models::MessageRole::Tool)
            .collect();
        assert_eq!(
            tool_msgs.len(),
            2,
            "should have call-1 result + call-2 stub"
        );
    }

    #[test]
    fn test_sanitize_tool_pairs_adds_stub_results() {
        let assistant_call = oben_models::ToolCall {
            id: "call-1".to_string(),
            tool_name: "test".to_string(),
            arguments: serde_json::Value::String("{}".to_string()),
        };
        let msgs = vec![Message {
            role: oben_models::MessageRole::Assistant,
            content: MessageContent::Text("hi".into()),
            id: None,
            tool_call_ids: vec![],
            tool_calls: Some(vec![assistant_call]),
            reasoning: None,
        }];
        let mut messages: Vec<Message> = msgs;
        let (removed, added) = sanitize_tool_pairs(&mut messages);
        assert_eq!(removed, 0);
        assert_eq!(added, 1, "should add 1 stub result");
        // Check stub content
        let stub_msg = messages
            .iter()
            .find(|m| {
                m.role == oben_models::MessageRole::Tool
                    && m.content
                        .to_text()
                        .contains("Result from earlier conversation")
            })
            .expect("should find stub tool result");
        assert!(stub_msg.content.to_text().contains("context summary above"));
    }

    #[test]
    fn test_strip_historical_media_replaces_images_before_anchor() {
        // Message at index 0 has an image — should be replaced
        // Message at index 1 has text only — no change
        // Message at index 2 has an image — this is the anchor, preserved
        let msgs = vec![
            Message {
                role: oben_models::MessageRole::User,
                content: MessageContent::Image {
                    url: "data:image/png;base64,AAAA".into(),
                    detail: None,
                },
                id: None,
                tool_call_ids: vec![],
                tool_calls: None,
                reasoning: None,
            },
            Message {
                role: oben_models::MessageRole::User,
                content: MessageContent::Text("look at this".into()),
                id: None,
                tool_call_ids: vec![],
                tool_calls: None,
                reasoning: None,
            },
            Message {
                role: oben_models::MessageRole::User,
                content: MessageContent::Image {
                    url: "data:image/png;base64,CCCC".into(),
                    detail: None,
                },
                id: None,
                tool_call_ids: vec![],
                tool_calls: None,
                reasoning: None,
            },
        ];
        let mut messages: Vec<Message> = msgs;
        let count = strip_historical_media(&mut messages);
        assert_eq!(count, 1, "should strip 1 image from historical message");
        // First message should now be a placeholder
        assert!(messages[0].content.to_text().contains("screenshot removed"));
        // Second message (text only) should be unchanged
        assert_eq!(messages[1].content.to_text(), "look at this");
        // Third message (anchor) should be preserved
        assert!(matches!(messages[2].content, MessageContent::Image { .. }));
    }

    #[test]
    fn test_strip_historical_media_no_anchor() {
        let msgs = vec![Message {
            role: oben_models::MessageRole::User,
            content: MessageContent::Text("hello".into()),
            id: None,
            tool_call_ids: vec![],
            tool_calls: None,
            reasoning: None,
        }];
        let mut messages: Vec<Message> = msgs;
        let count = strip_historical_media(&mut messages);
        assert_eq!(count, 0, "no images to strip");
    }

    #[test]
    fn test_strip_historical_media_anchor_first_message() {
        let msgs = vec![Message {
            role: oben_models::MessageRole::User,
            content: MessageContent::Image {
                url: "data:image/png;base64,AAAA".into(),
                detail: None,
            },
            id: None,
            tool_call_ids: vec![],
            tool_calls: None,
            reasoning: None,
        }];
        let mut messages: Vec<Message> = msgs;
        let count = strip_historical_media(&mut messages);
        assert_eq!(count, 0, "anchor is first message — nothing to strip");
    }

    #[test]
    fn test_strip_historical_media_parts() {
        // User message with mixed text and image parts before the anchor
        let msgs = vec![
            Message {
                role: oben_models::MessageRole::User,
                content: MessageContent::Parts(vec![
                    MessagePart::Text("before image".into()),
                    MessagePart::Image {
                        url: "data:image/png;base64,BBBB".into(),
                        detail: None,
                    },
                    MessagePart::Text("after image".into()),
                ]),
                id: None,
                tool_call_ids: vec![],
                tool_calls: None,
                reasoning: None,
            },
            Message {
                role: oben_models::MessageRole::User,
                content: MessageContent::Image {
                    url: "data:image/png;base64,CC".into(),
                    detail: None,
                },
                id: None,
                tool_call_ids: vec![],
                tool_calls: None,
                reasoning: None,
            },
        ];
        let mut messages: Vec<Message> = msgs;
        let count = strip_historical_media(&mut messages);
        assert_eq!(count, 1, "should strip 1 image from Parts");
        // Check the parts are replaced with text placeholders
        let parts = match &messages[0].content {
            MessageContent::Parts(p) => p,
            _ => panic!("expected Parts"),
        };
        assert_eq!(parts.len(), 3);
        assert!(matches!(&parts[0], MessagePart::Text(t) if t == "before image"));
        assert!(matches!(&parts[1], MessagePart::Text(t) if t.contains("screenshot removed")));
        assert!(matches!(&parts[2], MessagePart::Text(t) if t == "after image"));
    }

    // ─── End-to-end compact_session_messages tests (with mock transport) ──────

    /// Mock transport that returns a predictable summary for testing.
    struct MockTransport {
        summary: String,
    }

    #[async_trait::async_trait]
    impl TransportProvider for MockTransport {
        fn name(&self) -> &str {
            "mock"
        }

        async fn chat(
            &self,
            _messages: &[Message],
            _mode: &oben_models::CallMode,
        ) -> Result<oben_models::TransportResponse> {
            Ok(oben_models::TransportResponse {
                text: self.summary.clone(),
                tool_calls: vec![],
                tokens_used: None,
                reasoning: None,
            })
        }

        async fn stream_chat(
            &self,
            _messages: &[Message],
            _mode: &oben_models::CallMode,
            _delta_callback: oben_models::StreamDeltaCallback,
            _reasoning_callback: Option<oben_models::StreamReasoningCallback>,
        ) -> Result<oben_models::TransportResponse> {
            Ok(oben_models::TransportResponse {
                text: self.summary.clone(),
                tool_calls: vec![],
                tokens_used: None,
                reasoning: None,
            })
        }
    }

    fn build_long_conversation(count: usize) -> Vec<Message> {
        let mut msgs = vec![Message::system("You are a helpful coding assistant dedicated to helping the user with their projects. You always write high-quality code and follow best practices. You explain your reasoning before writing code.")];
        for i in 0..count {
            let question = format!("Question {}: What is the best way to implement a concurrent hashmap in Rust? Please explain the tradeoffs between Mutex<RwLock<HashMap<K,V>>> and DashMap. Consider lock contention, read throughput, write throughput, and memory overhead.", i);
            let answer = format!("Answer {}: Here's a comprehensive comparison:\n1. Mutex<RwLock<HashMap>>: High read throughput but writes block all reads. Simple to use. Good for read-heavy workloads with low contention.\n2. DashMap: Lock-free reads, sharded writes. Best overall performance for concurrent access. Higher memory overhead due to sharding.\n3. crossbeam::SyncQueue: No hashing, ordered operations. Good for pipeline patterns.\nRecommendation: Use DashMap for general-purpose concurrent HashMap. Use std::sync::RwLock if you need exact std::collections::HashMap semantics with read optimization.", i);
            msgs.push(Message::user(question));
            msgs.push(Message::assistant(answer));
            if i % 3 == 0 {
                let tool_call_msg = Message {
                    role: oben_models::MessageRole::Assistant,
                    content: MessageContent::Text(
                        "Let me check the documentation and write some benchmark code.".into(),
                    ),
                    id: None,
                    tool_call_ids: vec![],
                    tool_calls: Some(vec![oben_models::ToolCall {
                        id: format!("call-{i}"),
                        tool_name: "file_read".to_string(),
                        arguments: serde_json::json!({"path": format!("/src/benchmarks/mod{i}.rs", i=i), "content": "x".repeat(500)}),
                    }]),
                    reasoning: None,
                };
                msgs.push(tool_call_msg);
                let tool_result = format!("Benchmark results for mod{}:\nDashMap: 125ns read, 340ns write\nRwLock: 45ns read, 2800ns write (under contention)\nMutex: 42ns read, 2650ns write\n\nThe benchmarks confirm that DashMap provides the best balanced performance for concurrent workloads.", i);
                msgs.push(Message::tool_result(format!("call-{i}"), tool_result));
            }
            if i % 5 == 0 {
                let follow_up = format!("Question {}: Follow-up - What about atomic reference counting? When should I use Arc<Mutex<HashMap>> instead of DashMap?", i);
                let fa = format!("Answer {}: Arc<Mutex<HashMap>> is simpler but has higher lock contention. Only choose this if your access pattern is mostly single-threaded with occasional cross-thread reads, or if you need exact std::collections::HashMap API. DashMap is better for genuinely concurrent scenarios.", i);
                msgs.push(Message::user(follow_up));
                msgs.push(Message::assistant(fa));
            }
        }
        msgs
    }

    #[tokio::test]
    async fn test_compact_session_messages_basic() {
        // Given: a long conversation (20 turns → ~200+ messages)
        let messages = build_long_conversation(20);
        let _original_count = messages.len();
        // With long messages and small context, middle should be non-empty
        let config = CompactCofig {
            context_length: 10_000,
            threshold_percent: 0.75,
            protect_first_n: 2,
            tail_token_budget: 300, // Conservative tail budget
            tail_min_messages: 2,
            tail_overhead: 1.5,
            ..Default::default()
        };
        let transport = MockTransport {
            summary: "## Active Task\nTesting compact.\n## Goal\nEnd-to-end test.\n## Constraints\nMock transport.\n## Completed Actions\nBuilt 20-turn conversation.\n## Active State\nCompacting.\n## Key Decisions\nUse mock transport.\n## Critical Context\nNo sensitive data.".to_string(),
        };

        // When
        let result = compact_session_messages(&transport, &messages, &config, None, None, 0)
            .await
            .unwrap();

        // Then: head is preserved verbatim
        let head = &result.messages[..config.protect_first_n];
        assert_eq!(
            head.len(),
            config.protect_first_n,
            "head should preserve first N messages"
        );
        assert_eq!(
            head[0].content.to_text(),
            messages[0].content.to_text(),
            "system prompt preserved"
        );
        assert_eq!(
            head[1].content.to_text(),
            messages[1].content.to_text(),
            "first user msg preserved"
        );

        // Tail has at least tail_min_messages
        let tail_min = config.tail_min_messages.min(2);
        let tail = &result.messages[result.messages.len().saturating_sub(tail_min)..];
        assert!(
            tail.len() >= tail_min,
            "tail should have at least {} messages",
            tail_min
        );

        // Exactly 1 summary message
        let summary_count = result
            .messages
            .iter()
            .filter(|m| {
                m.role == oben_models::MessageRole::System
                    && m.content.to_text().contains("[CONTEXT COMPACTION")
            })
            .count();
        assert_eq!(summary_count, 1, "should have exactly 1 summary message");

        // Positive savings
        assert!(
            result.stats.savings_pct > 0.0,
            "should show positive savings, got {}",
            result.stats.savings_pct
        );
        assert!(
            result.stats.compacted_tokens < result.stats.original_tokens,
            "compacted tokens ({}) should be less than original ({})",
            result.stats.compacted_tokens,
            result.stats.original_tokens
        );
    }

    #[tokio::test]
    async fn test_compact_session_messages_short_list_no_compaction() {
        // Given: fewer messages than head + tail protection — nothing to compact
        let messages: Vec<Message> = (0..3).map(|i| Message::user(format!("msg{i}"))).collect();
        let transport = MockTransport {
            summary: "summary".to_string(),
        };

        // When
        let result = compact_session_messages(
            &transport,
            &messages,
            &CompactCofig::default(),
            None,
            None,
            0,
        )
        .await
        .unwrap();

        // Then: no middle means no summary, messages pass through unchanged
        assert_eq!(result.stats.original_count, 3);
        assert!(
            !result.stats.summary_generated,
            "no summary should be generated for short lists"
        );
    }

    #[tokio::test]
    async fn test_compact_session_messages_with_previous_summary() {
        // Given: a conversation with a previous compaction summary
        let messages = build_long_conversation(10);
        let previous_summary =
            "## Completed Actions\nPrevious work summary.\n## Active Task\nContinuing work.";
        let transport = MockTransport {
            summary: "## Completed Actions\n1. Previous work summary.\n2. New completed action.\n## Active Task\nNew active task.".to_string(),
        };

        // When
        let result = compact_session_messages(
            &transport,
            &messages,
            &CompactCofig {
                tail_token_budget: 300, // Conservative tail budget
                ..Default::default()
            },
            Some(previous_summary),
            None,
            0,
        )
        .await
        .unwrap();

        // Then: summary should incorporate both previous and new info
        assert!(
            result.stats.summary_generated,
            "should generate summary for 10-turn conversation"
        );
        let summary_text = result
            .messages
            .iter()
            .find(|m| {
                m.role == oben_models::MessageRole::System
                    && m.content.to_text().contains("[CONTEXT COMPACTION")
            })
            .map(|m| m.content.to_text())
            .unwrap_or_default();
        assert!(
            summary_text.contains("Previous work summary"),
            "should preserve previous summary info"
        );
        assert!(
            summary_text.contains("New completed action"),
            "should add new actions"
        );
    }

    #[tokio::test]
    async fn test_compact_session_messages_with_focus_topic() {
        // Given: a conversation where focus_topic should be preserved
        let messages = build_long_conversation(10);
        let transport = MockTransport {
            summary: "## Focus Topic: Rust ownership\nDetailed Rust ownership rules and patterns.\n## Completed Actions\nWorked on ownership.".to_string(),
        };

        // When
        let result = compact_session_messages(
            &transport,
            &messages,
            &CompactCofig {
                tail_token_budget: 300,
                ..Default::default()
            },
            None,
            Some("Rust ownership"),
            0,
        )
        .await
        .unwrap();

        // Then: summary should be generated with focus topic content
        assert!(result.stats.summary_generated, "should generate summary");
        let summary_text = result
            .messages
            .iter()
            .find(|m| {
                m.role == oben_models::MessageRole::System
                    && m.content.to_text().contains("[CONTEXT COMPACTION")
            })
            .map(|m| m.content.to_text())
            .unwrap_or_default();
        assert!(
            summary_text.contains("Rust ownership"),
            "should preserve focus topic details"
        );
    }

    #[tokio::test]
    async fn test_compact_session_messages_preserves_head_verbatim() {
        // Given: conversation starting with system prompt + specific user messages
        let system_msg = Message::system("You are a helpful coding assistant.");
        let first_user = Message::user("What is Rust?");
        let messages = vec![
            system_msg.clone(),
            first_user.clone(),
            Message::assistant("Rust is a systems language."),
            Message::user("Tell me more."),
            Message::assistant("Rust has ownership."),
        ];
        let config = CompactCofig {
            context_length: 500,
            threshold_percent: 0.3, // Very low threshold
            protect_first_n: 2,
            tail_token_budget: 50,
            tail_min_messages: 1,
            ..Default::default()
        };
        let transport = MockTransport {
            summary: "## Summary\nCompacted conversation about Rust.".to_string(),
        };

        // When
        let result = compact_session_messages(&transport, &messages, &config, None, None, 0)
            .await
            .unwrap();

        // Then: first 2 messages (system + first user) are preserved verbatim
        assert_eq!(
            result.messages[0].content.to_text(),
            system_msg.content.to_text()
        );
        assert_eq!(
            result.messages[1].content.to_text(),
            first_user.content.to_text()
        );
    }

    #[tokio::test]
    async fn test_compact_session_messages_with_tool_calls() {
        // Given: conversation with tool calls and results - using longer messages so tail doesn't absorb everything
        let long_tool_call_msg = Message {
            role: oben_models::MessageRole::Assistant,
            content: MessageContent::Text("I need to read the main.rs file to understand the current code structure. Let me examine it and then help you with the next steps.".into()),
            id: None,
            tool_call_ids: vec![],
            tool_calls: Some(vec![oben_models::ToolCall {
                id: "call-abc".to_string(),
                tool_name: "file_read".to_string(),
                arguments: serde_json::json!({"path": "/src/main.rs"}),
            }]),
            reasoning: None,
        };
        let messages = vec![
            Message::system("You are a helpful coding assistant dedicated to helping the user with their projects. You always write high-quality code and follow best practices."),
            Message::user("Please read the main.rs file and tell me what you find."),
            long_tool_call_msg,
            Message::tool_result("call-abc", "The file contains: fn main() { println!(\"hello world\"); }. It is a simple Rust program that prints hello world to the console. The file has no imports and is 10 lines long including the main function with a single print statement."),
            Message::user("Now please modify it to print something more interesting, like the current time or a countdown."),
            Message::assistant("I will modify the main.rs file to include a countdown from 5 to 1, then print a message. This requires adding the std::thread and std::time imports and using a loop with sleep intervals between each count."),
            Message::user("That sounds good. Please go ahead and implement it."),
            Message::assistant("I am now writing the modified main.rs file with the countdown functionality using std::thread::sleep and a for loop from 5 down to 1."),
        ];
        let config = CompactCofig {
            context_length: 500,
            threshold_percent: 0.5,
            protect_first_n: 1,
            tail_token_budget: 50,
            tail_min_messages: 1,
            tail_overhead: 1.2, // Slightly reduce overhead
            ..Default::default()
        };
        let transport = MockTransport {
            summary: "## Summary\nUser asked to read main.rs (which prints hello world), then modify it to include a countdown from 5 to 1 with sleep intervals.".to_string(),
        };

        // When
        let result = compact_session_messages(&transport, &messages, &config, None, None, 0)
            .await
            .unwrap();

        // Then: should survive without panicking, with sanitized tool pairs
        assert!(
            result.stats.summary_generated,
            "should generate summary for this message count"
        );
        // Check that tool_call still has valid JSON args
        for msg in &result.messages {
            if let Some(tool_calls) = &msg.tool_calls {
                for tc in tool_calls {
                    assert!(
                        tc.arguments.is_object(),
                        "tool_call args should still be valid JSON"
                    );
                }
            }
        }
    }
}
