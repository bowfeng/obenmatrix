/// Stream delta processing — scrubbers for thinking blocks, memory context, etc.

/// Stateful scrubber for reasoning/thinking tags in streamed deltas.
pub struct StreamingThinkScrubber {
    in_think_block: bool,
    buffer: String,
}

impl StreamingThinkScrubber {
    pub fn new() -> Self {
        Self {
            in_think_block: false,
            buffer: String::new(),
        }
    }

    pub fn scrub_delta(&mut self, delta: &str) -> String {
        if self.in_think_block {
            // We're inside a thinking block — strip everything until closing tag
            if delta.contains("</think") {
                let end = delta.find("</think").unwrap_or(delta.len());
                let after = &delta[end + "</think".len()..];
                self.in_think_block = false;
                if !after.is_empty() {
                    self.buffer.push_str(after);
                }
                return String::new();
            }
            return String::new();
        }

        // Not in a think block — check for opening tag
        if delta.contains("thinking") {
            let start = delta.find("thinking").unwrap_or(delta.len());
            let before = &delta[..start];
            if !before.is_empty() {
                self.buffer.push_str(before);
            }
            let after_open = &delta[start + "thinking".len()..];
            if after_open.contains("</think") {
                let end = after_open.find("</think").unwrap_or(after_open.len());
                let after_close = &after_open[end + "</think>".len()..];
                if !after_close.is_empty() {
                    self.buffer.push_str(after_close);
                }
                return String::new();
            }
            self.in_think_block = true;
            return String::new();
        }

        // Normal delta — just return it
        self.buffer.push_str(delta);
        delta.to_string()
    }

    pub fn into_buffer(self) -> String {
        self.buffer
    }

    pub fn reset(&mut self) {
        self.in_think_block = false;
        self.buffer.clear();
    }
}

impl Default for StreamingThinkScrubber {
    fn default() -> Self {
        Self::new()
    }
}

/// Stateful scrubber for <memory-context> spans split across stream deltas.
pub struct StreamingContextScrubber {
    in_memory_block: bool,
    buffer: String,
}

impl StreamingContextScrubber {
    pub fn new() -> Self {
        Self {
            in_memory_block: false,
            buffer: String::new(),
        }
    }

    pub fn scrub_delta(&mut self, delta: &str) -> String {
        if self.in_memory_block {
            // We're inside a memory block — strip everything until closing tag
            if delta.contains("</memory-context>") {
                let end = delta.find("</memory-context>").unwrap_or(delta.len());
                let after = &delta[end + "</memory-context>".len()..];
                self.in_memory_block = false;
                if !after.is_empty() {
                    self.buffer.push_str(after);
                }
                return String::new();
            }
            return String::new();
        }

        if delta.contains("<memory-context>") {
            let start = delta.find("<memory-context>").unwrap_or(delta.len());
            let before = &delta[..start];
            if !before.is_empty() {
                self.buffer.push_str(before);
            }
            let after_open = &delta[start + "<memory-context>".len()..];
            if after_open.contains("</memory-context>") {
                let end = after_open.find("</memory-context>").unwrap_or(after_open.len());
                let after_close = &after_open[end + "</memory-context>".len()..];
                if !after_close.is_empty() {
                    self.buffer.push_str(after_close);
                }
                return String::new();
            }
            self.in_memory_block = true;
            return String::new();
        }

        // Normal delta
        self.buffer.push_str(delta);
        delta.to_string()
    }

    pub fn into_buffer(self) -> String {
        self.buffer
    }

    pub fn reset(&mut self) {
        self.in_memory_block = false;
        self.buffer.clear();
    }
}

impl Default for StreamingContextScrubber {
    fn default() -> Self {
        Self::new()
    }
}

/// Scrub a single text string of thinking blocks (non-streaming).
///
/// Returns scrubbed text with content between `thinking...</think>` pairs removed.
/// The removed blocks are returned as reasoning text for separate display.
/// If a `thinking` tag is not closed, the entire text is preserved
/// (we don't want to silently drop user-visible content).
pub fn scrub_thinking_blocks(text: &str) -> (String, Option<String>) {
    let preview: String = text.chars().take(80).collect();
    tracing::debug!(
        "scrub_thinking_blocks: input len={}, first_80={:?}",
        text.len(),
        preview
    );
    let mut result = String::new();
    let mut reasoning_parts: Vec<String> = Vec::new();
    let mut remaining = text.to_string();

    while let Some(start) = remaining.find("thinking") {
        let before = &remaining[..start];
        if !before.is_empty() {
            // "thinking" appears somewhere later in the text — preserve
            // the full original input because we can't tell whether this
            // is an actual thinking block or just natural language.
            return (text.to_string(), None);
        }
        // "thinking" is at position 0 of this delta — extract reasoning.
        let after_open = &remaining[start + "thinking".len()..];
        match after_open.find("</thinking>") {
            Some(end) => {
                // Closed block — extract the reasoning between thinking...
                // and the closing tag, then continue processing what follows.
                let reasoning_text = &after_open[..end];
                if !reasoning_text.trim().is_empty() {
                    reasoning_parts.push(reasoning_text.to_string());
                }
                // Move past the closed thinking block
                remaining = after_open[end + "</thinking>".len()..].to_string();
            }
            None => {
                // Unclosed thinking block — strip "thinking" and collect
                // whatever reasoning content follows in case the block
                // closes in a subsequent delta.
                if !after_open.trim().is_empty() {
                    reasoning_parts.push(after_open.to_string());
                }
                // "thinking" was removed; remaining content is the
                // reasoning text itself, so clear remaining to avoid
                // re-processing the already-extracted content.
                remaining.clear();
            }
        }
    }
    result.push_str(&remaining);
    if result.is_empty() && reasoning_parts.is_empty() {
        // No thinking block found at all — return original text.
        return (text.to_string(), None);
    }
    
    let reasoning = if reasoning_parts.is_empty() {
        None
    } else {
        Some(reasoning_parts.join("\n\n"))
    };
    
    (result, reasoning)
}

/// Scrub a single text string of thinking blocks (non-streaming).
/// Legacy interface that discards extracted reasoning.
pub fn scrub_thinking_blocks_only(text: &str) -> String {
    scrub_thinking_blocks(text).0
}

/// Scrub a single text string of memory context blocks.
pub fn scrub_memory_context(text: &str) -> String {
    let mut result = String::new();
    let mut remaining = text.to_string();

    while let Some(start) = remaining.find("<memory-context>") {
        let before = &remaining[..start];
        result.push_str(before);
        let after_open = &remaining[start + "<memory-context>".len()..];
        if let Some(end) = after_open.find("</memory-context>") {
            let after_close = &after_open[end + "</memory-context>".len()..];
            remaining = after_close.to_string();
        } else {
            // Unclosed memory block → preserve entire text
            return text.to_string();
        }
    }
    result.push_str(&remaining);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── scrub_thinking_blocks tests ──────────────────────────────────

    #[test]
    fn test_scrub_strips_tags() {
        let text = "thinkinglet me think</thinking>visible";
        let (scrubbed, reasoning) = scrub_thinking_blocks(&text);
        assert_eq!(scrubbed, "visible");
        assert_eq!(reasoning, Some("let me think".to_string()));
    }

    #[test]
    fn test_scrub_preserves_text_outside() {
        // "thinking" in the middle → preserve full input.
        let text = "firstthinkingblock</thinking> second";
        let (scrubbed, reasoning) = scrub_thinking_blocks(text);
        assert_eq!(scrubbed, "firstthinkingblock</thinking> second");
        assert_eq!(reasoning, None);
    }

    #[test]
    fn test_scrub_multiple_blocks() {
        // Mid-content thinking → preserve full input (only position-0 stripped).
        let text = "AthinkingB</thinking>CthinkingD</thinking>E";
        let (scrubbed, reasoning) = scrub_thinking_blocks(text);
        assert_eq!(scrubbed, "AthinkingB</thinking>CthinkingD</thinking>E");
        assert_eq!(reasoning, None);
    }

    #[test]
    fn test_scrub_no_tags() {
        let (scrubbed, reasoning) = scrub_thinking_blocks("just text");
        assert_eq!(scrubbed, "just text");
        assert_eq!(reasoning, None);
    }

    #[test]
    fn test_scrub_unclosed_thinking_preserves_text() {
        // Thinking at position 0 is stripped; mid-content text is preserved.
        let text = format!("thinkingunclosed");
        let (scrubbed, reasoning) = scrub_thinking_blocks(&text);
        assert_eq!(scrubbed, "");
        assert_eq!(reasoning, Some("unclosed".to_string()));

        // Unclosed thinking in the middle of text — preserve full input.
        let text = "hello thinking about this";
        let (scrubbed, reasoning) = scrub_thinking_blocks(&text);
        assert_eq!(scrubbed, "hello thinking about this");
        assert_eq!(reasoning, None);
    }

    // ── StreamingThinkScrubber tests ─────────────────────────────────

    #[test]
    fn test_strips_opening_tag() {
        let mut s = StreamingThinkScrubber::new();
        assert_eq!(s.scrub_delta("thinking"), "");
        assert_eq!(s.into_buffer(), "");
    }

    #[test]
    fn test_strips_entire_block_across_deltas() {
        let mut s = StreamingThinkScrubber::new();
        assert_eq!(s.scrub_delta("thinking"), "");
        assert_eq!(s.scrub_delta("content"), "");
        assert_eq!(s.scrub_delta("</think"), "");
        assert_eq!(s.scrub_delta("visible"), "visible");
        assert_eq!(s.into_buffer(), "visible");
    }

    #[test]
    fn test_buffer_accumulates_outside_text() {
        let mut s = StreamingThinkScrubber::new();
        s.scrub_delta("before");
        assert_eq!(s.into_buffer(), "before");
    }

    // ── scrub_memory_context tests ───────────────────────────────────

    #[test]
    fn test_scrub_memory_strips_block() {
        let text = "<memory-context>secret</memory-context>after";
        assert_eq!(scrub_memory_context(text), "after");
    }

    #[test]
    fn test_scrub_memory_preserves_outside() {
        let text = format!("before<memory-context>secret</memory-context>after");
        assert_eq!(scrub_memory_context(&text), "beforeafter");
    }

    #[test]
    fn test_scrub_memory_no_block() {
        assert_eq!(scrub_memory_context("no block here"), "no block here");
    }

    // ── StreamingContextScrubber tests ───────────────────────────────

    #[test]
    fn test_scrub_memory_blocks_across_deltas() {
        let mut s = StreamingContextScrubber::new();
        assert_eq!(s.scrub_delta("<memory-context>"), "");
        assert_eq!(s.scrub_delta("hidden"), "");
        assert_eq!(s.scrub_delta("</memory-context>"), "");
        assert_eq!(s.scrub_delta("visible"), "visible");
        assert_eq!(s.into_buffer(), "visible");
    }

    // ── Streaming dual-extraction tests ────────────────────────────────
    // These exercises are the contract for api_call_with_retry's
    // scrub_thinking_blocks() split: each delta should yield both a
    // scrubbed delta and optional reasoning fragments.

    #[test]
    fn test_scrub_thinking_blocks_streaming_opening() {
        // Delta that opens a thinking block: "thinkinglet me reason..."
        let text = "thinkinglet me reason about the problem";
        let (scrubbed, reasoning) = scrub_thinking_blocks(text);
        assert_eq!(scrubbed, "");
        assert_eq!(reasoning, Some("let me reason about the problem".to_string()));
    }

    #[test]
    fn test_scrub_thinking_blocks_streaming_followup() {
        // Follow-up delta with just visible text (no thinking blocks):
        // " Here's what I found..."
        let text = " Here's what I found:";
        let (scrubbed, reasoning) = scrub_thinking_blocks(text);
        assert_eq!(scrubbed, " Here's what I found:");
        assert_eq!(reasoning, None);
    }
    #[test]
    fn test_scrub_thinking_blocks_streaming_multiple_blocks() {
        // Delta with two consecutive thinking blocks in one payload (edge case)
        // Uses format: thinking[content]
        // Format: thinking[content]</thinking>thinking[content]</thinking>[visible]
        let text = "thinkingstep 1</thinking>thinkingstep 2</thinking>visible";
        let (scrubbed, reasoning) = scrub_thinking_blocks(text);
        assert_eq!(scrubbed, "visible");
        assert_eq!(reasoning, Some("step 1\n\nstep 2".to_string()));
    }
    #[test]
    fn test_scrub_thinking_blocks_streaming_unclosed_preserves() {
        // Unclosed thinking block (edge case for streaming where tags span deltas):
        // "thinkingpartial thought" should extract reasoning, scrub thinking
        let text = "thinkingpartial thought";
        let (scrubbed, reasoning) = scrub_thinking_blocks(text);
        assert_eq!(scrubbed, "");
        assert_eq!(reasoning, Some("partial thought".to_string()));
    }
}
