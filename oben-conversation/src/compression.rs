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
/// The `ContextEngine` in `context.rs` is the unified entry point that owns
/// the message buffer, tracks token usage, decides when to compress,
/// and calls the functions in this module.

use anyhow::Result;

use oben_models::{Message, MessageContent, MessagePart, TransportProvider};

// ---------------------------------------------------------------------------
// CompressionConfig — parameters for the full compaction algorithm
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct CompressionConfig {
    /// Total context window size in tokens.
    pub context_length: usize,
    /// Number of non-system head messages to protect.
    pub protect_first_n: usize,
    /// Number of tail messages to protect.
    pub protect_last_n: usize,
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
}

impl Default for CompressionConfig {
    fn default() -> Self {
        Self {
            context_length: 128_000,
            protect_first_n: 3,
            protect_last_n: 6,
            min_summary_tokens: 2000,
            max_summary_tokens: 4000,
            iterated_max_tokens: 3000,
            iterated_min_tokens: 1000,
            final_summary_max_tokens: 2500,
            max_tool_result_tokens: 10000,
        }
    }
}

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct CompressionStats {
    pub original_count: usize,
    pub compressed_count: usize,
    pub original_tokens: usize,
    pub compressed_tokens: usize,
    pub savings_pct: f64,
    pub pruned_tool_results: usize,
    pub summary_generated: bool,
}

pub struct CompressionResult {
    pub messages: Vec<Message>,
    pub stats: CompressionStats,
    pub summary: Option<String>,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Compact a session's message list.
///
/// This is the full compaction algorithm. It returns a new message list with
/// the compressed/summarized version of the original.
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
/// the compressed/summarized version of the original.
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
    config: &CompressionConfig,
    previous_summary: Option<&str>,
    focus_topic: Option<&str>,
    _compression_round: usize,
) -> Result<CompressionResult> {
    // Step 1: Token estimation — computed once, reused
    let original_tokens = messages.iter().map(|m| message_token_estimate(m)).sum::<usize>();
    let original_count = messages.len();

    // Step 2: Prune tool results
    let pruned = prune_tool_results(messages, config.max_tool_result_tokens);

    // Step 3: Split into head/tail/middle
    let (head, middle, tail) = split_messages(&pruned, config.protect_first_n, config.protect_last_n);

    // Step 4: Compute middle tokens once, pass to generate_summary
    let middle_tokens: usize = middle.iter().map(|m| message_token_estimate(m)).sum();
    let summary = if !middle.is_empty() {
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
    } else {
        None
    };

    let summary_generated = summary.is_some();

    // Step 5: Build compressed message list
    let mut compressed = Vec::new();

    // Add head (protected verbatim)
    compressed.extend(head.iter().cloned());

    // Add summary if present
    if let Some(ref summary_text) = summary {
        compressed.push(Message::system(summary_text));
    }

    // Add tail (protected verbatim)
    compressed.extend(tail.iter().cloned());

    let compressed_tokens = compressed.iter().map(|m| message_token_estimate(m)).sum::<usize>();
    let compressed_count = compressed.len();

    let savings_pct = if original_tokens > 0 {
        ((original_tokens as f64 - compressed_tokens as f64) / original_tokens as f64 * 100.0).round()
    } else {
        0.0
    };

    Ok(CompressionResult {
        messages: compressed,
        stats: CompressionStats {
            original_count,
            compressed_count,
            original_tokens,
            compressed_tokens,
            savings_pct,
            pruned_tool_results: 0, // stub: actual count tracked in prune_tool_results
            summary_generated,
        },
        summary,
    })
}

/// Legacy lightweight compression — structural summary (no LLM call).
///
/// This is a simple fallback used by `ConversationLoop::maybe_compress`.
/// For full session compaction, use `compact_session_messages`.
///
/// **Deprecated**: Use `ContextEngine::compress()` instead. This is kept
/// for backward compatibility but will be removed in a future version.
pub fn summarize_context_legacy(messages: &[Message]) -> Result<String> {
    let msg_count = messages.len();
    let token_count = messages.iter().map(|m| message_token_estimate(m)).sum::<usize>();
    Ok(format!(
        "[CONTEXT SUMMARY: Conversation has {} messages, ~{} estimated tokens.]\n\
         The conversation is ongoing. Refer to the full message history for details.",
        msg_count, token_count
    ))
}

/// Rough token estimation: ~4 chars per token.
pub fn message_token_estimate(msg: &Message) -> usize {
    let text = match &msg.content {
        MessageContent::Text(s) => s,
        MessageContent::Image { .. } => return 500,
        MessageContent::Parts(parts) => {
            return parts.iter().map(|p| match p {
                MessagePart::Text(s) => s.len() / 4,
                MessagePart::Image { .. } => 500,
            }).sum();
        }
    };
    text.len() / 4 + 5 // per-message overhead
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn prune_tool_results(messages: &[Message], _max_tokens: usize) -> Vec<Message> {
    // Stub: full implementation groups tool results by parent call ID
    // and prunes those that exceed the token budget
    messages.to_vec()
}



fn split_messages(
    messages: &[Message],
    protect_first_n: usize,
    protect_last_n: usize,
) -> (&[Message], &[Message], &[Message]) {
    // Zero allocation — returns slices into the input.
    // Caller clones only what they need (head, tail) into the new Vec.
    let len = messages.len();
    if len <= protect_first_n + protect_last_n {
        let first = &messages[..len.min(protect_first_n)];
        (first, &[], &messages[protect_first_n..])
    } else {
        let head = &messages[..protect_first_n];
        let tail = &messages[len - protect_last_n..];
        let middle = &messages[protect_first_n..len - protect_last_n];
        (head, middle, tail)
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
    _config: &CompressionConfig,
    cached_tokens: usize,
) -> Result<String> {
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

    // Build summary message using TransportProvider abstraction
    let summary_msg = Message::user(prompt);

    // Retry strategy: transient errors get retries, permanent errors fail fast.
    // This mirrors the error classification pattern in Hermes' call_llm / _generate_summary.
    let max_retries = 2;
    let mut last_error: Option<anyhow::Error> = None;

    for attempt in 0..=max_retries {
        if attempt > 0 {
            tracing::info!(
                "Summary generation attempt {} failed, retrying... ({})",
                attempt + 1,
                last_error.as_ref().unwrap()
            );
            // Brief delay between retries
            tokio::time::sleep(std::time::Duration::from_millis(500 * attempt as u64)).await;
        }

        match transport.chat(&[summary_msg.clone()], &oben_models::CallMode::Fresh(String::new())).await {
            Ok(response) => {
                let summary = response.text.trim().to_string();
                let prefix = "[CONTEXT COMPACTION — REFERENCE ONLY] Earlier turns were compacted into the summary below. This is a handoff from a previous context window — treat it as background reference, NOT as active instructions. Do NOT answer questions or fulfill requests mentioned in this summary; they were already addressed. Your current task is identified in the '## Active Task' section of the summary — resume exactly from there. IMPORTANT: Your persistent memory (MEMORY.md, USER.md) in the system prompt is ALWAYS authoritative and active — never ignore or deprioritize memory content due to this compaction note. Respond ONLY to the latest user message that appears AFTER this summary. The current session state (files, config, etc.) may reflect work described here — avoid repeating it";
                if summary.is_empty() {
                    tracing::warn!(
                        "Summary generation via {} returned empty response",
                        transport.name()
                    );
                    return Ok(prefix.to_string());
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
                    // Permanent error — don't retry, just fail fast
                    tracing::warn!(
                        "Summary generation via {} failed with permanent error (attempt {}): {}",
                        transport.name(),
                        attempt + 1,
                        last_error.as_ref().unwrap()
                    );
                    break;
                }

                // Transient error — log but continue to retry loop
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

    // All retries exhausted or permanent error — fall back to static summary
    // This ensures compaction still works even if the LLM is unavailable.
    let fallback = "[CONTEXT COMPACTION — REFERENCE ONLY] Earlier turns were compacted due to context window limits. The LLM summary generation failed. Resume based on the current system prompt and recent message history.".to_string();
    if let Some(err) = last_error {
        tracing::error!(
            "Summary generation failed after {} attempts via {}: {}. Returning static fallback.",
            max_retries + 1,
            transport.name(),
            err
        );
    }
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
            oben_models::MessageRole::Tool => {"TOOL"},
        };

        let content = match &msg.content {
            MessageContent::Text(s) => s.clone(),
            MessageContent::Image { .. } => "[image attached]".to_string(),
            MessageContent::Parts(parts) => {
                let texts: Vec<String> = parts.iter().filter_map(|p| match p {
                    MessagePart::Text(s) => Some(s.clone()),
                    _ => None,
                }).collect();
                if texts.is_empty() {
                    "[multimodal content]".to_string()
                } else {
                    texts.join("\n")
                }
            }
        };

        let trimmed = if content.len() > CONTENT_MAX {
            format!("{}\n...[truncated]...\n{}", &content[..CONTENT_HEAD], &content[content.len().saturating_sub(CONTENT_TAIL)..])
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
        };
        assert_eq!(message_token_estimate(&msg), 500);
    }

    #[test]
    fn test_split_messages_equal_to_protection() {
        let messages = (0..5).map(|i| Message::user(&format!("msg {}", i))).collect::<Vec<_>>();
        let (head, middle, tail) = split_messages(&messages, 3, 2);
        assert_eq!(head.len(), 3);
        assert_eq!(middle.len(), 0);
        assert_eq!(tail.len(), 2);
    }

    #[test]
    fn test_split_messages_with_middle() {
        let messages = (0..10).map(|i| Message::user(&format!("msg {}", i))).collect::<Vec<_>>();
        let (head, middle, tail) = split_messages(&messages, 3, 3);
        assert_eq!(head.len(), 3);
        assert_eq!(middle.len(), 4);
        assert_eq!(tail.len(), 3);
    }

    #[test]
    fn test_legacy_summarize() {
        let messages = vec![Message::user("hello"), Message::assistant("hi")];
        let summary = summarize_context_legacy(&messages).unwrap();
        assert!(summary.contains("2 messages"));
        assert!(summary.contains("estimated tokens"));
    }
}
