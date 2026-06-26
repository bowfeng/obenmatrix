# Conversation Engine — Parity vs Hermes-Agent

**Scope:** `oben-agent` conversation loop  
**Reference:** `/Users/ellie/workspace/hermes-agent/agent/conversation_loop.py`, `context_engine.py`, `context_compression.py`

---

## Gap Matrix

| # | Feature | Severity | Status | Issue | Notes |
|---|---------|----------|--------|-------|-------|
| C.1 | ConversationLoop (core turn cycle) | ✅ | ✅ | (built-in) | Streaming + non-streaming, tool calls |
| C.2 | ContextWindowManager (buffer + token tracking + compression) | ✅ | ✅ | (built-in) | Full compaction algorithm |
| C.3 | Fallback model chain | 🟡 | ✅ | [#27](https://github.com/bowfeng/oben-alien/issues/27) | Auto-activation, configurable fallback list |
| C.4 | Callback system (12+ types) | 🟡 | ✅ | [#27](https://github.com/bowfeng/oben-alien/issues/27) | `on_turn_start`, `on_tool_call`, `on_error`, etc. |
| C.5 | Streaming scrubbers (thinking blocks, memory context) | 🟡 | ✅ | [#27](https://github.com/bowfeng/oben-alien/issues/27) | `StreamingContextScrubber` state machine |
| C.6 | System prompt prefix caching | 🟢 | ✅ | [#27](https://github.com/bowfeng/oben-alien/issues/27) | `SystemPromptCache` with TTL |
| C.7 | Activity tracking with timeout | 🟢 | ✅ | [#27](https://github.com/bowfeng/oben-alien/issues/27) | Track turn counts, timeouts |
| C.8 | Retry with jittered backoff | 🟡 | ✅ | [#25](https://github.com/bowfeng/oben-alien/issues/25) | Configurable retry policies |
| C.9 | Error classification (8 categories) | 🟡 | ✅ | [#25](https://github.com/bowfeng/oben-alien/issues/25) | `ErrorClassification` enum |
| C.10 | Iteration budget with 80%/90% warnings | 🟡 | ✅ | [#25](https://github.com/bowfeng/oben-alien/issues/25) | `IterationBudget` with threshold alerts |
| C.11 | Cross-thread interrupt + steer | 🟡 | ✅ | [#25](https://github.com/bowfeng/oben-alien/issues/25) | `InterruptState` with atomic flag |
| C.12 | Message sanitization (thinking-only drop, user merge) | 🟡 | ✅ | [#25](https://github.com/bowfeng/oben-alien/issues/25) | `MessageSanitizer` |
| C.13 | **Prompt caching** (Anthropic prompt cache hints) | 🟡 | ✅ | — | `cache_type: "ephemeral"` and `prompt_cache_key` in `oben-transport/src/anthropic_messages.rs` |
| C.14 | **Streaming context scrubber** (split `<memory-context>` across deltas) | 🟡 | ✅ | — | `StreamingContextScrubber` state machine in `oben-sessions/src/memory_provider.rs:516-664` |
| C.15 | **Turn nudge** (periodic memory review prompt) | 🟢 | ✅ | [#47](https://github.com/bowfeng/oben-alien/issues/47) | `on_turn_start()` turns-based nudge interval, default 10 turns |

---

## Legend

- **🔴 Critical** — blocks production use
- **🟡 High** — important for core functionality
- **🟢 Medium** — nice-to-have
- **Status**: ✅ Done | ❌ Not Started

**Workflow:** Open issue → branch (`#<number>-<desc>`) → implement → PR → close issue.
