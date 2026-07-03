# reasoning-fix-streaming - Work Plan

## TL;DR (For humans)

**What you'll get:** Reasoning blocks stream in the TUI during LLM output. Currently, when the LLM streams thinking blocks, the TUI shows an empty reasoning block because reasoning content is accumulated by the transport but never emitted to the TUI during streaming — it only appears in the final response.

**Why this approach:** The transport already correctly separates `reasoning_content` at the API level during streaming; it just doesn't have a way to emit it to hooks. Adding an optional `StreamReasoningCallback` parameter to `stream_chat()` is the minimal interface change — one parameter on an async method, no changes to `TransportResponse` struct or any existing behavior.

**What it will NOT do:** No test changes (current tests already work); no `scrub_thinking_blocks()` updates (kept for inline tag handling); no reasoning support for Anthropic/Gemini (no-op placeholders).

**Effort:** Short
**Risk:** Low — one optional parameter on an async method, no behavior changes to existing code paths, no struct mutations
**Decisions I made for you:** (1) `Option<StreamReasoningCallback>` — user chose optional (2) Only `ChatCompletionsTransport` emits reasoning; Anthropic/Gemini accept parameter but don't emit (3) `scrub_thinking_blocks()` kept for inline `<thinking>` tags — separate path from API-level `reasoning_content`

Your next move: Approve then run `$start-work` to execute. High-accuracy review (Momus) offered.

---

> TL;DR (machine): Short risk-low, 8 source files + 8 test files, add StreamReasoningCallback to TransportProvider trait, emit reasoning during streaming in ChatCompletionsTransport, wire turn_executor to pass callback

## Scope
### Must have
- Add `StreamReasoningCallback` type alias to `oben-models/src/providers.rs`
- Add `Option<StreamReasoningCallback>` parameter to `TransportProvider::stream_chat()` trait
- Update blanket `Arc<T>` impl (pass reasoning through)
- Update `ChatCompletionsTransport::stream_chat()` to emit reasoning_content via callback
- Update `AnthropicMessagesTransport::stream_chat()` to accept parameter (no-op — no streaming reasoning parsing)
- Update `GeminiMessagesTransport::stream_chat()` to accept parameter (no-op — no streaming reasoning parsing)
- Update `Transport` enum dispatch impl to forward reasoning parameter
- Update `TestTransport` (registry.rs) to accept parameter (no-op)
- Update `turn_executor.rs:api_call_with_retry()` to create `hooks.emit_reasoning` callback and pass it
- Update ALL test mocks (11 impls across 10 files)

### Must NOT have (guardrails, anti-slop, scope boundaries)
- No changes to `TransportResponse` struct (reasoning field already exists)
- No changes to existing non-streaming `chat()` method
- No changes to `scrub_thinking_blocks()` logic (kept for inline `<thinking>` handling)
- No test logic changes (test mocks just accept new optional param)
- No new dependencies or feature flags
- No behavioral changes to `TuiStreamingAdapter` or `TuiState`

## Verification strategy
> Zero human intervention - all verification is agent-executed.
- Test decision: none (this is a wiring fix; code compiles and checks pass)
- Evidence: `cargo check -p oben-models` + `cargo check -p oben-transport` + `cargo check -p oben-agent` + `cargo check -p oben-tui`

## Execution strategy
### Parallel execution waves
> 4 waves with parallel subagents per wave.

### Dependency matrix
| Todo | Depends on | Blocks | Can parallelize with |
| --- | --- | --- | --- |
| 1. Trait + type alias | — | 2,3,4,5,6,7,8 | (must be first) |
| 2. ChatCompletionsTransport | 1 | 7 | 3,4,5,6 |
| 3. AnthropicMessagesTransport | 1 | — | 2,4,5,6 |
| 4. GeminiMessagesTransport | 1 | — | 2,3,5,6 |
| 5. Dispatch impl | 1 | — | 2,3,4,6 |
| 6. Registry test impl | 1 | — | 2,3,4,5 |
| 7. Turn executor wiring | 1,2,3,4,5,6 | 8 | (last non-test) |
| 8. Test mocks | 1 | — | (parallel with everything) |
| 9. Compilation check | All 1-8 | Success | — |
| F1-F4. Final verification | 9 | Release | all |

## Todos
> Implementation + Test = ONE todo. Never separate.
<!-- APPEND TASK BATCHES BELOW THIS LINE WITH edit/apply_patch - never rewrite the headers above. -->
- [ ] 1. Add StreamReasoningCallback type + update TransportProvider trait
  What to do: Add `pub type StreamReasoningCallback = Box<dyn FnMut(&str) + Send>;` after line 488. Add `reasoning_callback: Option<StreamReasoningCallback>` parameter to `stream_chat()` trait method (line 547-552). Update blanket impl (line 505-512) to forward `reasoning_callback` to `(**self).stream_chat(...)`. Update doc comment.
  Must NOT do: Change `StreamDeltaCallback` type signature. Change `TransportResponse` struct. Change non-streaming `chat()` method.
  Parallelization: Wave 1 — single task, must complete before all others
  References: `oben-models/src/providers.rs:487-582`
  Acceptance criteria: `cargo check -p oben-models` passes with zero errors
  QA scenarios: happy: `cargo check -p oben-models`; failure: trait impl mismatch for any implementor
  Commit: Y | feat(trait): add StreamReasoningCallback to TransportProvider

- [ ] 2. Update ChatCompletionsTransport to emit reasoning via callback
  What to do: Update `stream_chat()` signature to accept `Option<StreamReasoningCallback>`. In the SSE loop (around line 851), after extracting `reasoning_delta = delta.reasoning_content`, if `reasoning_callback.is_some() && !reasoning_delta.is_empty()`, call `reasoning_callback.as_mut().unwrap()(reasoning_delta)`. Remove duplicate accumulation of reasoning (it's already emitted via callback; keep final_reasoning for TransportResponse backwards-compat).
  Must NOT do: Change text delta emission logic. Remove reasonong accumulation in final_response.
  Parallelization: Wave 2 | Blocks: 7 (turn_executor wiring needs this to actually emit) | Can parallelize with: 3,4,5,6
  References: `oben-transport/src/chat_completions.rs:848-881`, `oben-transport/src/chat_completions.rs:768-773`
  Acceptance criteria: `cargo check -p oben-transport` passes. `reasoning_delta` from `delta.reasoning_content` is emitted via callback for each non-empty delta.
  QA scenarios: happy: `cargo check -p oben-transport`; failure: reasoning_callback is None (optional guard)
  Commit: Y | feat(transport): emit reasoning_content via callback during streaming

- [ ] 3. Update AnthropicMessagesTransport to accept parameter (no-op emit)
  What to do: Update `stream_chat()` signature to accept `Option<StreamReasoningCallback>` parameter. No-op: do NOT call the callback (Anthropic doesn't currently parse streaming reasoning_content in this transport). The parameter exists for future-proofing.
  Must NOT do: Call the reasoning callback (Anthropic's streaming API doesn't have reasoning_content support in this transport).
  Parallelization: Wave 2 | Can parallelize with: 2,4,5,6
  References: `oben-transport/src/anthropic_messages.rs:847-852`, `oben-transport/src/anthropic_messages.rs:801-845`
  Acceptance criteria: `cargo check -p oben-transport` passes. No compilation errors from new parameter.
  QA scenarios: happy: `cargo check -p oben-transport`; failure: signature mismatch from trait update
  Commit: Y | feat(transport): add StreamReasoningCallback param to Anthropic (no-op)

- [ ] 4. Update GeminiMessagesTransport to accept parameter (no-op emit)
  What to do: Same as Anthropiс — update `stream_chat()` signature to accept `Option<StreamReasoningCallback>`. No-op: do NOT call the callback (Gemini doesn't support streaming reasoning in this transport).
  Must NOT do: Call the reasoning callback.
  Parallelization: Wave 2 | Can parallelize with: 2,3,5,6
  References: `oben-transport/src/gemini.rs:807-842`
  Acceptance criteria: `cargo check -p oben-transport` passes.
  QA scenarios: happy: `cargo check -p oben-transport`
  Commit: Y | feat(transport): add StreamReasoningCallback param to Gemini (no-op)

- [ ] 5. Update Transport dispatch impl
  What to do: Update `Transport::stream_chat()` to accept `Option<StreamReasoningCallback>` and forward it to each variant's inner transport. Both `Transport::OpenAIChat` and `Transport::Anthropic` arms pass it through.
  Must NOT do: Add new variants to Transport enum.
  Parallelization: Wave 2 | Can parallelize with: 2,3,4,6
  References: `oben-transport/src/dispatch.rs:184-198`
  Acceptance criteria: `cargo check -p oben-transport` passes with new impl.
  QA scenarios: happy: `cargo check -p oben-transport`; failure: mismatched signature across enum arms
  Commit: Y | refactor(transport): forward StreamReasoningCallback in dispatch

- [ ] 6. Update all test mocks (10 files, 11 impls)
  What to do: Add `Option<StreamReasoningCallback>` parameter to every `TransportProvider::stream_chat()` impl. No-op: all mocks just return `TransportResponse { reasoning: None }` — do NOT call the callback.
  Files: `oben-agent/src/compact.rs:1278`, `oben-agent/src/compact_context.rs:920`, `oben-agent/tests/delegate_tests.rs:46`, `oben-agent/tests/session_rotation.rs:40`, `oben-agent/tests/reset_session.rs:43`, `oben-transport/src/registry.rs:241`, `oben-transport/tests/integration.rs:215,250,283,321,347,382`, `oben-transport/tests/anthropic_messages_test.rs:188,256,304,332`
  Must NOT do: Change any test logic or assertions.
  Parallelization: Wave 2 | Can parallelize with: 2,3,4,5
  References: `oben-agent/src/compact.rs:1278`, `oben-agent/src/compact_context.rs:920-926`, `oben-agent/tests/delegate_tests.rs:28-55`, `oben-agent/tests/session_rotation.rs:23-55`, `oben-agent/tests/reset_session.rs:26-55`, `oben-transport/src/registry.rs:223-253`, `oben-transport/tests/integration.rs:215-382`, `oben-transport/tests/anthropic_messages_test.rs:188-332`
  Acceptance criteria: `cargo check` passes for all packages with zero impl errors.
  QA scenarios: happy: `cargo check --workspace`; failure: any impl signature mismatch
  Commit: Y | test(mocks): add StreamReasoningCallback param to all test impls

- [ ] 7. Wire turn_executor to pass reasoning callback
  What to do: In `api_call_with_retry()` (line 381-395), create a `StreamReasoningCallback` that calls `hooks.emit_reasoning(reasoning)`. Pass this as `Some(callback)` to `transport.stream_chat()`. Keep existing `scrub_thinking_blocks()` delta callback unchanged — it only handles inline `<thinking>` tags, separate path from API-level `reasoning_content`.
  Must NOT do: Remove existing scrub_thinking_blocks logic. Replace delta callback with reasoning callback.
  Parallelization: Wave 3 | Blocks: verification | Can parallelize with: nothing (depends on waves 1-6)
  References: `oben-agent/src/turn_executor.rs:381-395`, `oben-agent/src/hooks/runtime.rs:200-205`
  Acceptance criteria: `cargo check -p oben-agent` passes. Both streaming callbacks (delta + reasoning) passed to `transport.stream_chat()`.
  QA scenarios: happy: `cargo check -p oben-agent`; failure: closure capture mismatch of hooks
  Commit: Y | fix(agent): wire emit_reasoning callback to stream_chat for reasoning_content

- [ ] 8. Full compilation check across all packages
  What to do: Run `cargo check -p oben-models && cargo check -p oben-transport && cargo check -p oben-agent && cargo check -p oben-tui && cargo check --workspace`. Fix any errors.
  Parallelization: Wave 3 | Blocked by: 7
  References: All updated files above
  Acceptance criteria: ALL `cargo check` commands pass with zero errors and zero warnings
  QA scenarios: happy: clean build; failure: any remaining impl mismatch or type error
  Commit: N (verification only)

## Final verification wave
> Runs in parallel after ALL todos. ALL must APPROVE. Surface results and wait for the user's explicit okay before declaring complete.
- [ ] F1. Plan compliance audit
- [ ] F2. Code quality review
- [ ] F3. Real manual QA
- [ ] F4. Scope fidelity

## Commit strategy

## Success criteria
