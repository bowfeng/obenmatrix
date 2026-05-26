# Models & Providers — Parity vs Hermes-Agent

**Scope:** LLM provider support, model normalization, transport dispatch, auth methods, API payload attributes  
**Hermes Reference:** `/Users/ellie/workspace/hermes-agent/hermes_cli/providers.py`, `model_catalog.py`, `model_normalize.py`, `agent/transports/`, `hermes_cli/config.py`  
**obenalien Reference:** `oben-models/src/providers.rs`, `oben-transport/src/`, `oben-config/src/config.rs`

---

## Legend

- **🔴 Critical** — blocks production use (major providers missing, wrong API mode)
- **🟡 High** — important for core functionality (user-config providers, some transports)
- **🟢 Medium** — nice-to-have (niche providers, advanced features)
- **Status**: ✅ Done | ❌ Not Started

---

## Gap Matrix

### A. Supported Providers

| # | Feature | Severity | Status | Issue | Notes |
|---|---------|----------|--------|-------|-------|
| M.1 | OpenAI-compatible transport (`openai_chat`) | ✅ | ✅ | (built-in) | OpenRouter, OpenAI, NovitaAI, vLLM, custom endpoints |
| M.2 | Anthropic Messages API | 🔴 | ✅ | [#44](https://github.com/bowfeng/obenalien/issues/44) | Native `messages/` API with prompt caching, tool use, thinking tokens |
| M.3 | AWS Bedrock native transport | 🟡 | ❌ | [TBD] | `bedrock/runtime` Converse API (oben supports OpenAI-compatible endpoint only) |
| M.4 | Google Gemini transport | 🟡 | ❌ | [TBD] | Gemini API (REST + AIO), Gemini thinking_config |
| M.5 | Codex protocol | 🟢 | ❌ | [TBD] | OpenAI Codex, XAI event-driven protocol |
| M.6 | Provider dispatch registry | 🟡 | ❌ | [TBD] | `get_transport("anthropic_messages")` auto-dispatch, auto-registration |
| M.7 | Provider catalog (models.dev) | 🟡 | ❌ | [TBD] | Provider definitions + metadata from models.dev (109+ providers), 24h disk cache |
| M.8 | User-defined providers (`providers:`) | 🟡 | ❌ | [TBD] | Allow arbitrary provider definitions in config |
| M.9 | Custom providers list | 🟢 | ❌ | [TBD] | `custom_providers:` list in config |
| M.10 | Provider alias system | 🟡 | ✅ (#22) | [TBD] | 50+ aliases: `claude`→`anthropic`, `gpt`→`openai`, `glm`→`zai`, `qwen`→`alibaba`, etc. |

### B. Built-in Provider List

| # | Provider | Severity | Hermes | obenalien | Notes |
|---|----------|----------|--------|-----------|-------|
| P.1 | OpenRouter | ✅ | ✅ | ✅ | OpenAI-compatible |
| P.2 | OpenAI | 🟡 | ✅ | ✅ | OpenAI-compatible |
| P.3 | Anthropic | 🔴 | ✅ | ✅ | `anthropic_messages` transport implemented |
| P.4 | AWS Bedrock | 🟡 | ✅ | ✅ (partial) | OpenAI-compatible endpoint only; missing `bedrock_converse` |
| P.5 | Google Gemini | 🟢 | ✅ | ❌ | Missing transport |
| P.6 | LM Studio | 🟢 | ✅ | ✅ | OpenAI-compatible |
| P.7 | Custom | ✅ | ✅ | ✅ | User-defined base_url |
| P.8 | Nous Portal | 🟢 | ✅ | ✅ | OpenAI-compatible aggregator |
| P.9 | Azure Foundry | 🟢 | ✅ | ❌ | Supports OpenAI + Anthropic modes |
| P.10 | NVIDIA (NIM) | 🟢 | ✅ | ✅ | OpenAI-compatible |
| P.11 | Vercel AI Gateway | 🟢 | ✅ | ✅ | Aggregator |
| P.12 | OpenCode Zen | 🟢 | ✅ | ✅ | Aggregator |
| P.13 | OpenCode Go | 🟢 | ✅ | ❌ | Aggregator |
| P.14 | KiloCode | 🟢 | ✅ | ✅ | Aggregator |
| P.15 | HuggingFace | 🟢 | ✅ | ✅ | Aggregator |
| P.16 | NovitaAI | 🟢 | ✅ | ✅ | OpenAI-compatible |
| P.17 | XAI (Grok) | 🟢 | ✅ | ❌ | Codex protocol |
| P.18 | Arcee | 🟢 | ✅ | ❌ | OpenAI-compatible |
| P.19 | GMI Cloud | 🟢 | ✅ | ❌ | OpenAI-compatible |
| P.20 | GitHub Copilot | 🟢 | ✅ | ❌ | Codex protocol |
| P.21 | OpenAI Codex | 🟢 | ✅ | ❌ | Codex protocol |
| P.22 | **阿里云/Qwen** | 🟡 | ✅ | ✅ | DashScope, Alibaba, Qwen Portal; `alibaba` provider |
| P.23 | **智谱/Zai** | 🟡 | ✅ | ✅ | GLM models; `zai` provider |
| P.24 | **阶跃/StepFun** | 🟡 | ✅ | ✅ | `stepfun` provider |
| P.25 | **MiniMax** | 🟡 | ✅ | ✅ | `minimax` / `minimax-oauth` / `minimax-cn` providers |
| P.26 | **腾讯 TokenHub** | 🟡 | ✅ | ✅ | `tencent-tokenhub` provider |
| P.27 | **小米 MiMo** | 🟡 | ✅ | ✅ | `xiaomi` provider |
| P.28 | **Kimi (Moonshot)** | 🟡 | ✅ | ✅ | `kimi-for-coding` provider; reasoning_effort + thinking_config |

**国内核心提供商全部支持（P.8-P.15 aggregators + P.22-P.28 国内 7 家）**

### C. Transport / Protocol Support

| # | Transport | Severity | Hermes | obenalien | Notes |
|---|-----------|----------|--------|-----------|-------|
| T.1 | `openai_chat` (Chat Completions) | ✅ | ✅ | ✅ | Streaming + SSE, tool calls, usage, per-session request cache |
| T.2 | `anthropic_messages` | 🔴 | ✅ | ✅ | Native `messages/` API, streaming SSE, prompt caching, thinking tokens, tool use |
| T.3 | `bedrock_converse` | 🟡 | ✅ | ❌ | AWS SDK v4 Converse API |
| T.4 | `codex_responses` | 🟢 | ✅ | ❌ | Event-driven (like OpenAI Codex, XAI) |

### D. API Payload Attributes (OpenAI-compatible transport)

These are fields in the JSON body sent to the `/v1/chat/completions` endpoint.

| # | Attribute | Severity | Hermes | obenalien | Hermes Usage |
|---|-----------|----------|--------|-----------|-------------|
| PL.1 | `model` | ✅ | ✅ | ✅ | Provider model name (normalized via `model_normalize.py`) |
| PL.2 | `messages` | ✅ | ✅ | ✅ | System, user, assistant, tool messages (with Codex sanitization) |
| PL.3 | `tools` | ✅ | ✅ | ✅ | OpenAI function call format; Moonshot tool sanitization |
| PL.4 | `temperature` | 🟡 | ✅ | ✅ | Per-provider fixed temperature (Anthropic, Kimi omit); config override |
| PL.5 | `max_tokens` | ✅ | ✅ | ✅ | `max_completion_tokens` for OpenAI; `max_tokens` for others |
| PL.6 | `top_p` | 🟢 | ✅ | ✅ | Per-provider override |
| PL.7 | `top_k` | 🟢 | ✅ | ✅ | Native API support (Qwen, Gemini) |
| PL.8 | `frequency_penalty` | 🟢 | ✅ | ✅ | OpenAI-compatible per-call override |
| PL.9 | `presence_penalty` | 🟢 | ✅ | ✅ | OpenAI-compatible per-call override |
| PL.10 | `logit_bias` | 🟢 | ✅ | ✅ | OpenAI-compatible per-call override |
| PL.11 | `stop_sequences` | 🟢 | ✅ | ✅ | Stop sequence control |
| PL.12 | `response_format` | 🟡 | ✅ | ✅ | JSON mode (`{"type": "json_object"}`) |
| PL.13 | `tool_choice` | 🟡 | ✅ | ✅ | `auto`, `required`, `none`, `{"type": "function", "function": {"name": "x"}}` |
| PL.14 | `stream_options` | 🟢 | ✅ | ✅ | `include_usage: true` (oben has this hardcoded) |
| PL.15 | `timeout` | 🟢 | ✅ | ✅ | Per-call timeout override (BaseTransport.with_timeout) |
| PL.16 | `service_tier` | 🟢 | ✅ | ✅ | Priority Processing for OpenAI (`"auto"`, `"priority"`, `"default"`) |
| PL.17 | `provider_preferences` | 🟢 | ✅ | ✅ | OpenRouter provider routing (`extra_body.provider`) |
| PL.18 | `extra_body` | 🟡 | ✅ | ✅ | Provider-specific fields: `reasoning`, `thinking`, `google`, plugins, tags, vl_high_resolution |
| PL.19 | `user_id` | 🟢 | ✅ | ✅ | OpenRouter usage tracking |
| PL.20 | `metadata` | 🟡 | ✅ | ✅ | Per-call metadata (Qwen session metadata, request tagging) |

### E. Thinking / Reasoning Configuration

These are provider-specific fields that control LLM reasoning/thinking behavior.

| # | Attribute | Severity | Hermes | obenalien | Notes |
|---|-----------|----------|--------|-----------|-------|
| TH.1 | `reasoning_effort` (OpenAI-compatible) | 🟡 | ✅ | ✅ | Top-level: `"low"`, `"medium"`, `"high"`, `"xhigh"` — used by DeepSeek, LM Studio, Kimi, Tencent, GitHub |
| PL.16 | `reasoning.enabled` / `reasoning.effort` (OpenRouter extra_body) | 🟡 | ✅ | ✅ | `extra_body.reasoning = {"enabled": true, "effort": "medium"}` (OpenRouter) |
| PL.17 | `extra_body.thinking.type` | 🟡 | ✅ | ✅ | Kimi: `{"type": "enabled"}` / `{"type": "disabled"}` |
| PL.18 | `thinking_config` (Gemini OpenAI-compatible) | 🟡 | ✅ | ✅ | `{includeThoughts: true, thinkingLevel: "low"/"medium"/"high", thinkingBudget: N}` |
| PL.19 | `prompt_cache` | 🔴 | ✅ | ✅ | Anthropic prompt caching (`cache_markers` in messages, `cache_ttl` config) |
| PL.20 | `anthropic_max_output` | 🟢 | ✅ | ✅ | Max output tokens for Claude via OpenRouter/Nous |
| PL.21 | `ollama_num_ctx` | 🟢 | ✅ | ✅ | Ollama context window override |
| PL.22 | Developer role swap | 🟢 | ✅ | ❌ | System→developer role for GPT-5/Codex models |

### F. Anthropic Payload (separate transport)

| # | Attribute | Severity | Hermes | obenalien | Notes |
|---|-----------|----------|--------|-----------|-------|
| A.1 | `system` (top-level) | 🔴 | ✅ | ✅ | `AnthropicRequest.system` field, not in messages array |
| A.2 | `max_tokens` | 🔴 | ✅ | ✅ | `AnthropicRequest.max_tokens` (required, not optional) |
| A.3 | `tool_choice` | 🟡 | ✅ | ✅ | `AnthropicToolChoice` enum: auto, any, tool, detector |
| A.4 | `thinking` (thinking tokens) | 🔴 | ✅ | ✅ | `AnthropicThinking` struct (struct defined, wired via config TBD) |
| A.5 | Prompt caching markers | 🔴 | ✅ | ❌ | `<cache_control>` in messages |
| A.6 | `stop_sequences` | 🟢 | ✅ | ✅ | `AnthropicRequest.stop_sequences` field |

### G. Model Name Normalization

| # | Feature | Severity | Hermes | obenalien | Notes |
|---|---------|----------|--------|-----------|-------|
| N.1 | Aggregator `vendor/model` format | 🟡 | ✅ | ✅ | `claude-sonnet-4.6` → `anthropic/claude-sonnet-4.6` |
| N.2 | Dots → hyphens (Anthropic) | 🟡 | ✅ | ✅ | `claude-sonnet-4.6` → `claude-sonnet-4-6` |
| N.3 | DeepSeek canonical mapping | 🟡 | ✅ | ✅ | `deepseek-r1` → `deepseek-reasoner`, `deepseek-v3` → `deepseek-chat` |
| N.4 | Copilot model handling | 🟢 | ✅ | ✅ | Special Copilot API model name mapping |
| N.5 | Provider prefix stripping | 🟡 | ✅ | ✅ | Auto-strip matching `provider/model` on native providers |
| N.6 | Case normalization | 🟢 | ✅ | ✅ | e.g. Xiaomi requires lowercase |
| N.7 | Vendor prefix detection | 🟡 | ✅ | ✅ | Detect `claude` → `anthropic`, `gpt` → `openai`, etc. |

### H. Authentication

| # | Auth Method | Severity | Hermes | obenalien | Notes |
|---|-------------|----------|--------|-----------|-------|
| A.1 | API Key (Bearer) | ✅ | ✅ | ✅ | |
| A.2 | OAuth Device Code | 🟢 | ✅ | ❌ | Nous Portal, etc. |
| A.3 | OAuth External | 🟢 | ✅ | ❌ | Copilot, Gemini, XAI |
| A.4 | External Process | 🟢 | ✅ | ❌ | Copilot ACP |
| A.5 | AWS SDK Credentials | 🟡 | ✅ | ❌ | Bedrock |
| A.6 | Env var fallback chain | 🟡 | ✅ | ❌ | Multiple env vars per provider (e.g. `ANTHROPIC_TOKEN`, `CLAUDE_CODE_OAUTH_TOKEN`) |

### I. Model Catalog & Discovery

| # | Feature | Severity | Hermes | obenalien | Notes |
|---|---------|----------|--------|-----------|-------|
| C.1 | Remote model catalog | 🟢 | ✅ | ❌ | `models.dev` manifest, 24h disk cache, per-provider overrides |
| C.2 | Curated model lists | 🟢 | ✅ | ❌ | OpenRouter/Nous curated models, pricing, cache pricing |
| C.3 | `/v1/models` API | ✅ | ✅ | ✅ | Built-in `list_models()` / `find_model()` |

---

## Priority Summary

### Must Have (🔴 Critical)
- T.2: Anthropic Messages transport (prompt caching, thinking tokens, native tool use)
- M.22-28: Domestic providers (阿里云, 智谱, 阶跃, MiniMax, 腾讯, 小米, Kimi)
- PL.6-PL.13: Payload attributes (temperature, top_p, top_k, frequency/presence_penalty, stop_sequences, response_format, tool_choice)
- PL.18: `extra_body` support for provider-specific fields
- TH.1-TH.2: Reasoning/thinking configuration (reasoning_effort, thinking_config, prompt_cache)

### Should Have (🟡 High)
- T.1: Provider dispatch registry
- M.10: Alias system
- N.1-N.7: Model normalization
- P.3: Bedrock native transport
- A.1: Env var fallback chains
- C.1: Model catalog

### Could Have (🟢 Medium)
- T.3-T.4: Codex/Gemini transports
- M.11-M.21: Additional aggregator/cloud providers
- A.2-A.5: OAuth/External auth
- C.2: Curated model lists

---

## Workflow

For each gap:
1. Create GitHub issue referencing this parity file (e.g. `docs/PRD-models-parity.md#P.22`)
2. Create branch: `#<number>-<short-desc>`
3. Implement with BDD tests: Unit → Integration → Live (`oben-scenario-test/`)
4. Open PR: `#<number>: <description>`
5. After merge: close issue, update Status to ✅
