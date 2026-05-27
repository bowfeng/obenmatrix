# Transport & Provider — Parity vs Hermes-Agent

**Scope:** LLM transport implementations in `oben-transport`  
**Reference:** `/Users/ellie/workspace/hermes-agent/agent/transports/`

---

## Gap Matrix

| # | Feature | Severity | Status | Issue | Notes |
|---|---------|----------|--------|-------|-------|
| T.1 | OpenAI-compatible `ChatCompletionsTransport` | ✅ | ✅ | (built-in) | Streaming + SSE, tool calls, usage tracking |
| T.2 | Anthropic native Messages API | 🔴 | ✅ | [#44](https://github.com/bowfeng/oben-alien/issues/44) | Native `messages/` API, streaming SSE, prompt caching, thinking tokens, tool use |
| T.3 | AWS Bedrock transport | 🟡 | ❌ | [TBD] | `bedrock/runtime` client for Claude/Mistral/Llama |
| T.4 | Google Gemini transport | 🟡 | ✅ (#68) | [68](https://github.com/bowfeng/oben-alien/issues/68) | Gemini native API (REST), REST + AIO |
| T.5 | Codex / OpenAI Codex protocol | 🟢 | ❌ | [TBD] | Event-driven protocol |
| T.6 | Transport trait + registry pattern | 🟡 | ✅ (#63) | [63](https://github.com/bowfeng/oben-alien/issues/63) | `register_transport()`, `get_transport()`, `unregister_transport()`, `list_transport_names()`, lazy auto-discovery of built-in transports |

---

## Legend

- **🔴 Critical** — blocks production use
- **🟡 High** — important for core functionality
- **🟢 Medium** — nice-to-have
- **Status**: ✅ Done | ❌ Not Started

**Workflow:** Open issue → branch (`#<number>-<desc>`) → implement → PR → close issue.
