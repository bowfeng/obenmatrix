---
slug: reasoning-fix-streaming
status: drafting
intent: unclear
pending-action: write .omo/plans/reasoning-fix-streaming.md
approach: Add StreamReasoningCallback to TransportProvider trait; update all transports to emit reasoning_content via this callback; wire turn_executor to pass hooks.emit_reasoning
---

# Draft: reasoning-fix-streaming

## Components (topology ledger)
| id | outcome (one line) | status | evidence path |
|---|---|---|---|
| C1 | Add StreamReasoningCallback type + update TransportProvider trait | active | `oben-models/src/providers.rs:487-582` |
| C2 | Update blanket impl for Arc<T> | active | `oben-models/src/providers.rs:492-525` |
| C3 | Update ChatCompletionsTransport to emit reasoning during streaming | active | `oben-transport/src/chat_completions.rs:848-881` |
| C4 | Update AnthropicMessagesTransport (no-op for now) | active | `oben-transport/src/anthropic_messages.rs:847` |
| C5 | Update GeminiMessagesTransport (no-op for now) | active | `oben-transport/src/gemini.rs:842` |
| C6 | Update Transport dispatch impl (forward reasoning callback) | active | `oben-transport/src/dispatch.rs:184-196` |
| C7 | Update turn_executor to pass reasoning callback to hooks | active | `oben-agent/src/turn_executor.rs:381-395` |
| C8 | Update all test mocks (no-op implementations) | active | 9 test mock impls across 8 files |

## Open assumptions (announced defaults)
| assumption | adopted default | rationale | reversible? |
|---|---|---|---|
| reasoning_callback parameter type | `Option<StreamReasoningCallback>` (user chose) | Flexibility; transports can emit or skip | Yes - can make required later |
| scrub_thinking_blocks() kept for inline tags | Yes | It handles inline `<thinking>`...`<ee>` blocks in text; API-level `reasoning_content` comes via callback separately | No - these are orthogonal paths |
| No scrubbing on reasoning content | Yes | API-level `reasoning_content` is clean; no inline tags | Yes - add scrubbing later if needed |
| Gemini/Anthropic skip reasoning emission | Yes (no-op) | These transports don't currently parse streaming reasoning_content; trait supports future emission | Yes - implement when transport gains reasoning parsing |

## Findings (cited - path:lines)

### Current flow (BROKEN):
1. `turn_executor.rs:api_call_with_retry()` (line 381-394): Creates `StreamDeltaCallback` that wraps `scrub_thinking_blocks(text)`. If scrubbing extracts inline thinking tags, emits via `hooks.emit_reasoning()`.
   - **Gap**: Only works for inline `<thinking>`...`</thinking>` blocks in the text delta. Does NOT handle API-level `reasoning_content`.

2. `chat_completions.rs:stream_chat()` (line 768-958): 
   - Line 848-851: Extracts `reasoning_delta = delta.reasoning_content.as_deref().unwrap_or("")`
   - Line 853-855: Accumulates into `final_reasoning` (local variable)
   - Line 874-878: Calls `delta_callback(text)` for content text only
   - Line 952-957: Returns `reasoning: if final_reasoning.is_empty() { None } else { Some(final_reasoning) }` in final response
   - **Critical bug**: Never calls `emit_reasoning()` for `reasoning_delta` during streaming! Only returns in final `TransportResponse`.

3. `chat_completions.rs:stream_chat()` (line 857-873): The text extraction logic correctly strips `reasoning_content` from the text that goes to `delta_callback`, preventing duplicate emission from inline scrubbing.

### Anthropic (line 847): No streaming reasoning_content support. Delta types only have `text` and `input_json`. Returns `reasoning: None`.

### Gemini (line 842): No streaming reasoning support. Returns `reasoning: None`.

### Hook path (working):
- `turn_executor.rs:389`: `hooks.emit_reasoning(reasoning_text)` — only fires for inline scrubbed thinking blocks
- `hooks/runtime.rs:200`: `emit_reasoning()` broadcasts to all streaming hooks
- `hooks/tui.rs`: `TuiStreamingAdapter.on_reasoning()` appends to `TurnState.reasoning_text` — works fine, just never receives data

## Decisions (with rationale)

### Decision 1: Add optional StreamReasoningCallback to stream_chat()
**Choice**: `reasoning_callback: Option<StreamReasoningCallback>` on `TransportProvider::stream_chat()`
**Rationale**: Transport extracts reasoning at API level during streaming; without a separate callback, there's no way to emit reasoning to hooks during streaming. The user chose Option; non-optional would require more changes in test mocks.
**Tradeoffs**: Optional adds a `if let` per reasoning delta. Negligible overhead.

### Decision 2: Only ChatCompletionsTransport emits reasoning (others no-op)
**Choice**: `chat_completions.rs` emits reasoning; `anthropic_messages.rs` and `gemini.rs` accept but don't emit.
**Rationale**: Only OpenAI-compatible API supports `reasoning_content` field in streaming deltas (as of current API). Anthropic and Gemini transports don't currently parse streaming reasoning fields.
**Tradeoffs**: Future Anthropic/Gemini reasoning support requires same pattern.

### Decision 3: scrub_thinking_blocks() stays for inline tags
**Choice**: Keep existing inline `<thinking>`...`<ee>` scrubbing in `turn_executor.rs:381-394`.
**Rationale**: Two separate reasoning paths: (1) API-level `reasoning_content` via new callback, (2) inline `<thinking>` tags in text via scrubber. They're orthogonal and don't conflict since the transport already strips reasoning_content from text (line 857-873).
**Tradeoffs**: The scrubber handles edge case where models produce inline tags instead of API reasoning_content.

### Decision 4: No scrubbing on reasoning callback content
**Choice**: Pass raw `reasoning_delta` to reasoning callback without scrub_thinking_blocks.
**Rationale**: API-level `reasoning_content` is already extracted from the thinking block by the API; no inline tags to scrub. Adding scrubbing would be double-processing.
**Tradeoffs**: If a future model mixes inline tags with API reasoning_content, they'd be missed. Unlikely scenario.

## Scope IN
- Add `StreamReasoningCallback` type alias in `oben-models/src/providers.rs`
- Update `TransportProvider` trait's `stream_chat()` to accept `Option<StreamReasoningCallback>`
- Update blanket `Arc<T>` impl
- Update `ChatCompletionsTransport::stream_chat()` to emit reasoning deltas via callback
- Update `AnthropicMessagesTransport::stream_chat()` to accept parameter (no-op emit)
- Update `GeminiMessagesTransport::stream_chat()` to accept parameter (no-op emit)
- Update `Transport` enum dispatch impl (forward reasoning callback)
- Update `turn_executor.rs:api_call_with_retry()` to create and pass reasoning callback
- Update ALL test mocks (9 implementations across 8 files)

## Scope OUT (Must NOT have)
- Modify existing test behavior or assertions
- Add new functionality beyond streaming reasoning emission
- Update `scrub_thinking_blocks()` logic (kept as-is)
- Handle reasoning in non-streaming `chat()` method
- Support reasoning for non-OpenAI transports (Anthropic/Gemini no-op placeholders)
- Add tests (code compiles + checks pass = sufficient for this fix)

## Open questions
- None identified. The root cause is clear; fix approach is well-defined.

## Approval gate
status: awaiting-approval
pending-action: Present brief to user and wait for explicit OK to implement
