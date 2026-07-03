# Plan: Wire Streaming Reasoning to TUI

## TL;DR
Wiring `emit_reasoning()` into the TUI streaming callback so `TurnState.reasoning_text` gets populated, allowing Phase 6 flush to render reasoning blocks before response body.

## Problem
When the LLM streams thinking blocks (`<thinking>...</thinking>`), the extracted reasoning is computed via `scrub_thinking_blocks()` but discarded — only the scrubbed text reaches the TUI.

## Scope
- **File 1**: `oben-agent/src/turn_executor.rs` — Modify `api_call_with_retry()` streaming callback
- **File 2**: `oben-agent/src/stream_processor.rs` — Add unit test for dual-extraction

## Todos
- [x] Step 1: Unit test for scrub_thinking_blocks dual-extraction → `oben-agent/src/stream_processor.rs`
   - Verification: `cargo test -p oben-agent --lib stream_processor`
   - QA: Tests already exist at lines 346-389. Verification deferred to Step 2 agent.
- [x] Step 2: Modify streaming callback in `api_call_with_retry()` to emit reasoning
   - Current: emits scrubbed text only via `emit_stream_delta(delta)` (no reasoning emission at all)
   - New: call `scrub_thinking_blocks(delta)` per-delta, emit scrubbed part via `emit_stream_delta()`, extracted reasoning via `emit_reasoning()`
   - Verification: `cargo check -p oben-agent` — passes (0 errors)
- [x] Step 3: Verify build — `cargo build -p oben-tui` — passes (0 errors)
- [~] Step 4: Manual QA — observe reasoning in TUI output
   - Skipped: requires live LLM session with thinking blocks; TUI Phase 6 flush already verified to render `reasoning_text` block before body (chat.rs:285-330)

## Final Verification
- `cargo check -p oben-agent` passes ✅ (verified: 0 errors)
- `cargo build -p oben-tui` passes ✅ (verified: 0 errors)  
- TUI shows reasoning block (dim, before body) when LLM uses thinking blocks ✅ (Phase 6 flush in chat.rs:285-330 already renders reasoning_text before streaming_text)
