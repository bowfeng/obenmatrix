# PRD: ObenAgent — Hermes Agent Rust Port

**Author:** ellie  
**Created:** 2026-05-17  
**Status:** 🟡 In Progress — P2 Skills & Curator  
**Target:** MVP first (core loop + tools + CLI + streaming), then iterate outward toward full feature parity  
**Language:** Rust (async, multi-threaded, tokio runtime)

---

## Tracking Documents

| Document | Purpose |
|----------|--------|
| [`docs/PRD.md`](PRD.md) | **This file** — architecture, milestones, overall progress |
| [`docs/PRD-*.parity.md`](PRD-*.md) | **Per-crate parity trackers** — every gap between Hermes and ObenAgent, with GitHub issue links (cli, conversation, session, tools, transport, skills, goals, gateway, utils) |
| [`docs/design/aiagent-design.md`](design/aiagent-design.md) | **Architecture design** — Rust-specific design decisions for parity features |
| `docs/adr/` | **Architectural decisions** — key design choices and their rationale |

**Workflow:** Feature gaps are tracked in `docs/PRD-{area}-parity.md`. Each gap gets a GitHub issue. Implementation happens in a branch, PR is opened, and after merge the issue is closed.

---

## Overview

| Metric | Status |
|--------|--------|
| Crates | 14/14 compiling |
| Tests | 506/507 passing (489 unit + 17 live, 1 live failing) |
| CLI subcommands | 10 |
| Provider transports | 1/7 (OpenAI-compatible ChatCompletions) |
| Built-in tools | 18 |
| Skill categories | 25/20+ ✅ |

**Status: 🟡 Phase 1 — Core Engine**

```
✅ M1: Core Agent Loop       Workspace, types, config, conversation loop, transport, CLI
✅ M2: Streaming & Discovery  SSE, model discovery, wizard auto-detection
✅ M3: Goal Loop              Autonomous loop, plan parsing, judge verdict, state machine
✅ M4: Skills & Curator       25 skill categories, SkillLoader, Curator lifecycle
✅ M5: Core Engine Parity     Retry/backoff, error classification, interrupt/steer, sanitization
                              Fallback chain, callbacks, scrubbers, prompt cache, activity tracking
                              Session: write concurrency, schema expansion, lineage, FTS5, titles
                              ⚠ Session rotation on compression (schema ready, not wired)
✅ TUI: Started               Chat, config, sessions, setup panels + style system
🟡 M6: Multi-Provider         Anthropic native, Bedrock, Gemini, transport registry
🟡 M7: Platform Integrations  Telegram, Discord, Slack adapters + routing
🟡 M8: Plugin System          Full extensibility framework (discovery, hooks, providers, slash cmds)
🟡 M9: Advanced Tools         Search, browser, voice, image, delegate, kanban, computer use
🟢 M10: Polish                TUI completion, web dashboard, skills hub, config enhancements, I18n
```

---

## Recent Progress

| Date | Status | Notes |
|------|--------|-------|
| 2026-05-22 | ✅ Session Parity | **Write concurrency**: BEGIN IMMEDIATE + jittered retry + WAL checkpoint + NFS fallback. **Schema expansion**: 14 new columns (billing/caching/API tracking) + declarative reconciliation. **Compression lineage**: end_reason-aware walking + orphan cleanup + ghost pruning. **Trigram FTS5**: CJK/Thai search. **Title management**: sanitization + dedup + lineage resolution. Closes #30, #32, #33, #37. |
| 2026-05-22 | ✅ Phase 2: Advanced Runtime | Fallback model chain, rich callback system (12+ callback types), streaming scrubbers (thinking blocks, memory context), system prompt prefix caching, activity tracking. Closes #27 |
| 2026-05-22 | ✅ Tier 1: Core Reliability | Retry with jittered backoff, error classification, iteration budget with 80%/90% warnings, cross-thread interrupt + steer, message sanitization. Closes #25 |
| 2026-05-18 | ✅ LLM-based summarization | `generate_summary()` no longer a stub — actually calls LLM via `reqwest` with structured prompt template (Active Task, Goal, Constraints, Completed Actions, etc.), iterative updates, and focus topic support. Falls back to static placeholder on LLM failure. CLI: `oben sessions compact [-s SESSION] [-f FOCUS]` |
| 2026-05-23 | 🔄 Workspace refactor | `oben_conversation` → `oben-agent` (122 unit tests, expanded with retry/fallback/interrupt/sanitize/stream processor/scrubbers/callbacks/concurrent dispatch). Root binary → thin wrapper delegating to `oben-cli`. New `oben-tui` crate with chat/config/sessions/setup panels + style system. |

## Done ✅

| Crate | Tests | Status | Details |
|-------|-------|--------|---------|
| **oben-models** | 21 | ✅ | Message, Tool, Skill, Session, ProviderConfig, TransportProvider trait, ModelInfo, ModelListResponse, roundtrip JSON/YAML serialization, image content parts |
| **oben-utils** | 6 | ✅ | Logging (tracing + `--verbose`/`RUST_LOG`), terminal spinner, path security, env helpers, table formatter |
| **oben-config** | 6 | ✅ | YAML config, setup wizard (interactive), system prompt defaults, gateway config serialization, model discovery, roundtrip save/load |
| **oben-agent** | 122 | ✅ | ConversationLoop, ContextEngine (buffer + token tracking + compression trigger + thrashing detection), full compaction algorithm (message pruning, tool result dedup/truncation, split enforcement), PromptBuilder with identity/skills/volatile blocks + prompt cache, IterationBudget (warnings, grace period), error classifier (8 categories), jittered exponential backoff retry, fallback model chain, cross-thread interrupt + steer, message sanitization (thinking-only drop, user merge, surrogate stripping), streaming scrubbers (thinking blocks, memory context), char-level UTF-8 streaming output, callback system (12+ types), concurrent tool dispatch (serial for destructive) |
| **oben-transport** | 51 | ✅ | BaseHTTPTransport, ChatCompletionsTransport (OpenAI-compatible), SSE streaming via `eventsource-stream`, unit + integration tests |
| **oben-tools** | 87 | ✅ | ToolRegistry + auto-registration, terminal (fg/bg + mgmt), read_file, write_file, http_get, web_search, search_files (ripgrep), patch (fuzzy), web_extract (SSRF + HTML), vision_analyze (image download + base64 encoding + OpenAI/Anthropic API call), memory (add/replace/remove + scan), clarify, todo (JSON store), code_execution (sandbox), osv_check, skill (list/view), 87 unit tests |
| **oben-sessions** | 44 | ✅ | SessionDB (SQLite-backed session state engine with FTS5, message windows, lineage resolution), Rich Search (discover/scroll/browse shapes), Bounded MemoryStore (file locking, atomic writes, injection scanning, frozen snapshots). Legacy JSONL SessionManager preserved for backwards compatibility.
| **oben-skills** | 70 | ✅ | SkillLoader (recursive SKILL.md discovery, YAML/TXT/MD), SkillManager (enable/disable/auto-use/instruction assembly + preprocessing config), frontmatter parsing, platform matching, tags/config/conditions extraction, qualified name parsing, external dirs support, skill_preprocessing (template vars ${SKILL_DIR}/${SESSION_ID}, inline shell !`cmd` expansion), 70 unit tests |
| **oben-goals** | 68 | ✅ | PlanNode (tree, builder, artifacts), PlanState (find/count/markdown/save/load), judge verdict parser, GoalState (turn budget, parse failures auto-pause), plan parser, node complete/failure parser, roundtrip JSON/Markdown |
| **oben-gateway** | 13 | ✅ | Gateway struct, PlatformAdapter trait, Incoming/OutgoingMessage, mock adapter support |
| **oben-curator** | 17 | ✅ | Usage tracking (use/view/patch counts), lifecycle states (active→stale→archived), scheduler (pause/resume), report generation (text + JSON) |

**Total: 489 unit tests passing across 13 crates**

---

## Workspace Structure

```
obenalien/               # Root workspace (binary — thin wrapper)
├── Cargo.toml           # Workspace config + root package
├── src/main.rs          # Binary entry point → delegates to oben-cli
│
├──oben-agent/           # Agent engine (ex-oben_conversation, expanded)
│   ├── conversation.rs  # ConversationLoop — main turn cycle (streaming + non-streaming)
│   ├── context.rs       # ContextEngine — unified: buffer, real token tracking, should_compress(), compress()
│   ├── prompt.rs        # PromptBuilder — system prompt + message assembly
│   ├── compact.rs       # Message pruning, tool result sanitization, split enforcement
│   ├── compact_context.rs # ContextEngine with thrashing detection and error state
│   ├── system_prompt.rs # System prompt builder with identity, skills, volatile blocks
│   ├── system_prompt_cache.rs # Prompt cache with TTL
│   ├── budget.rs        # IterationBudget — turn limits, warnings, grace
│   ├── error_classifier.rs # Error classification (8 categories: auth/rate_limit/model_not_found/server_error/bad_request/network/timeout/other)
│   ├── retry.rs         # Jittered exponential backoff retry
│   ├── fallback.rs      # Fallback model chain with auto-activation
│   ├── interrupt.rs     # Cross-thread interrupt + steer mechanism
│   ├── message_sanitize.rs # Thinking-only drop, user merge, non-ASCII/surrogate stripping
│   ├── stream_processor.rs # Streaming scrubbers (thinking blocks, memory context)
│   ├── turn_executor.rs # Char-level UTF-8 streaming output
│   ├── callbacks.rs     # Callback system (12+ types)
│   ├── concurrent_dispatch.rs # Tool dispatch: serial for destructive, concurrent otherwise
│   ├── transport.rs     # Re-exports TransportProvider from oben-models
│   └── lib.rs
│
├──oben-cli/             # CLI subcommand implementation
│   ├── cli.rs           # Clap command definitions, arg parsing
│   ├── dispatch.rs      # Command routing to crate logic
│   └── lib.rs
│
├──oben-tui/             # Terminal UI (panes/widgets)
│   ├── panels/          # Chat, config, sessions, setup panels
│   ├── widgets/         # Style system, shared UI components
│   └── lib.rs
│
├──oben-models/          # Core domain types
│   ├── messages.rs      # Message, MessageContent, MessageRole
│   ├── tools.rs         # Tool, ToolCall, ToolResult, ToolBuilder
│   ├── skills.rs        # Skill definition, SkillBuilder
│   ├── session.rs       # Conversation session storage
│   └── providers.rs     # ProviderConfig, TransportProvider trait, ModelInfo
│
├──oben-utils/           # Shared utilities
│   ├── logging.rs       # tracing-subscriber initialization
│   ├── terminal.rs      # Spinner, progress indicators
│   ├── path_security.rs # Path traversal prevention
│   ├── env_utils.rs     # Environment variable helpers
│   └── table.rs         # Table formatter for CLI output
│
├──oben-config/          # Configuration
│   ├── config.rs        # AppConfig (YAML-based, ~/.obenalien/config.yaml)
│   ├── defaults.rs      # Default system prompt, provider defaults
│   └── wizard.rs        # Interactive setup wizard (clap + dialoguer)
│
├──oben-transport/       # LLM transport implementations
│   ├── base.rs          # BaseTransport — HTTP client, request/response types
│   └── chat_completions.rs # ChatCompletionsTransport — OpenAI-compatible API
│
├──oben-tools/           # Tool implementations
│   ├── registry.rs      # ToolRegistry — dynamic tool registration/dispatch
│   ├── terminal.rs      # Terminal tool (fg/bg + management)
│   ├── read_write.rs    # read_file / write_file tools
│   ├── web.rs           # http_get tool
│   ├── search.rs        # Web search (stub — configurable provider)
│   ├── search_files.rs  # File search via ripgrep
│   ├── patch.rs         # Fuzzy file patching
│   ├── web_extract.rs   # HTML content extraction (SSRF protection)
│   ├── vision_analyze.rs # Image download + base64 encoding + analysis
│   ├── memory.rs        # Memory tool (add/replace/remove + scan)
│   ├── clarify.rs       # Clarification prompt tool
│   ├── todo.rs          # Todo list (JSON store)
│   ├── code_execution.rs # Code execution (sandboxed)
│   ├── osv_check.rs     # OSV vulnerability check
│   └── skill.rs         # Skill management tool
│
├──oben-skills/          # Skill system
│   ├── loader.rs        # SkillLoader — reads YAML/TXT/MD from disk
│   └── system.rs        # SkillManager — enable/disable, auto-use, instruction assembly
│
├──oben-goals/           # Goal tracking, plan management, judge loop
│   ├── plan.rs          # PlanNode (tree with builder), artifacts
│   ├── plan_parser.rs   # parse_plan_from_markdown()
│   ├── plan_state.rs    # PlanState — CRUD, markdown, save/load
│   ├── judge.rs         # Judge verdict types
│   ├── verdict.rs       # parse_judge_response()
│   ├── goal_loop.rs     # run_goal_loop(), create_plan_from_goal()
│   ├── goal_loop/goal_state.rs  # GoalState, GoalStatus
│   └── goal_loop/transport.rs   # Goal transport trait
│
├──oben-curator/         # Skill lifecycle management
│   ├── usage.rs         # UsageRecord, usage tracking (use/view/patch)
│   ├── lifecycle.rs     # LifecycleState (active/stale/archived/pinned)
│   ├── curator.rs       # Curator orchestrator + CuratorState (scheduler)
│   └── report.rs        # Human-readable + JSON report generation
│
├──oben-sessions/        # Persistent memory & session management
│   ├── manager.rs       # SessionDB (SQLite-backed session state engine with FTS5, message windows, lineage) + legacy SessionManager (JSONL backwards compat)
│   ├── search.rs        # RichSearch (discover/scroll/browse shapes, FTS5-backed)
│   └── skill_curation.rs # MemoryStore (bounded entries, file locking, atomic writes, injection scanning, frozen snapshots)
│
└──oben-gateway/         # Messaging gateway
    ├── gateway.rs       # Gateway — route messages from platforms to agent
    └── platform.rs      # PlatformAdapter trait — Telegram/Discord/Slack adapters
```

---

## Vision

Build a self-improving AI agent in Rust, porting the full functionality of Hermes Agent. It creates and improves skills from experience, supports multiple LLM providers, runs anywhere (VPS, GPU cluster, serverless), and communicates via CLI, Telegram, Discord, Slack, and other platforms.

---

## Key Design Principles

- **Performance-first** — Rust gives us the speed and memory safety for resource-constrained deployments
- **Multi-provider** — OpenAI-compatible APIs, Anthropic, Bedrock, Gemini, custom endpoints
- **Extensible** — Plugin-style tool system, YAML/TXT skill definitions, modular architecture
- **Run anywhere** — Local, Docker, SSH, Modal, Daytona, Vercel Sandbox

---

## Key Decisions

### Transport Trait Location
Moved `TransportProvider` to `oben-models::providers` to break a circular dependency between `oben_agent` and `oben-transport`.

### Tool Handler Type
Used `Arc<dyn Fn(Value) -> Pin<Box<dyn Future<Output = Result<ToolResult>>> + Send>>` to allow asynchronous tool execution.

### Goal Loop Closure Pattern
`run_goal_loop` takes a generic async closure `F: FnMut(&str) -> Fut` for maximum flexibility, avoiding a crate dependency on `oben_agent`.

### Streaming Crate
Chose `eventsource-stream` for native `reqwest` compatibility. CLI has `--stream` flag.

### Callback Sharing
Used `Arc<Mutex<F>>` in `run_turn_with_streaming` to share callbacks across streaming chunks.

---

## Milestones

### M1: Core Agent Loop ✅
- Workspace, types, config, conversation loop, transport, CLI
- Can run a one-shot prompt and interactive chat with any OpenAI-compatible API

### M2: Streaming & Model Discovery ✅
- SSE via `eventsource-stream`, `--stream` CLI flag
- `oben models list` and `oben models info`
- Wizard auto-detects `max_model_len` from `/v1/models`

### M3: Goal Loop ✅
- Autonomous agent loop with plan parsing, judge verdict, state machine
- Plan in system prompt, immune to context truncation
- `obenalien agent` subcommand

### M4: Skills & Curator ✅
- 25 built-in skill categories matching Hermes Agent's `skills/` layout
- SkillLoader reads YAML/TXT/MD from disk
- Curator crate: usage tracking, lifecycle (active→stale→archived), scheduler

### M5: Core Engine Parity ✅
- **Tier 1 — Core Reliability:** retry with jittered backoff, error classification (8 categories), iteration budget with 80%/90% warnings, cross-thread interrupt + steer, message sanitization (thinking-only drop, user merge, surrogate stripping)
- **Phase 2 — Advanced Runtime:** fallback model chain with auto-activation, rich callback system (12+ types), streaming scrubbers (thinking blocks, memory context), system prompt prefix caching (TTL-based), activity tracking with timeout, concurrent tool dispatch (serial for destructive, concurrent otherwise)
- **Session parity:** write concurrency (BEGIN IMMEDIATE + jittered retry + WAL checkpoint), schema expansion (14 cols for billing/caching/API tracking), compression lineage (end_reason-aware walking + orphan cleanup + ghost pruning), trigram FTS5 (CJK/Thai search), title management (sanitization + dedup + lineage resolution), persistence on all error paths
- Design doc: `docs/design/aiagent-design.md`
- Parity docs: `docs/PRD-conversation-parity.md` (12/15 done), `docs/PRD-session-parity.md` (7/9 done)

### M6: Multi-Provider Transport 🟡
- **Anthropic native Messages API** (🔴 critical) — prompt caching, tool use, thinking tokens, native `messages/` endpoint
- **AWS Bedrock transport** — `bedrock/runtime` for Claude/Mistral/Llama
- **Google Gemini transport** — Gemini REST + AIO APIs
- **Transport trait + registry** — `get_transport("anthropic_messages")` dispatch, auto-registration per provider
- **Prompt cache hints** (Anthropic `cache_type: ephemeral`) — cache hit tracking
- Parity doc: `docs/PRD-transport-parity.md` (1/6 done)

### M7: Platform Integrations 🟡
- **Telegram adapter** (🔴 critical) — webhook + polling, file handling, per-platform session isolation
- **Discord adapter** (🔴 critical) — bot, slash commands
- **Slack adapter** (🔴 critical) — RTM + Socket Mode
- **WhatsApp adapter** — WA Web API
- **Delivery routing** — platform-aware message delivery
- **Slash command routing** — `/pause`, `/resume`, `/status` via gateway
- **Pairing** — user ↔ platform registration
- Parity doc: `docs/PRD-gateway-parity.md` (1/11 done)

### M8: Extensibility Framework (Plugin System) 🟡
- **PluginManager** — Central discovery & lifecycle, 4-source scanning (bundled, user, project, pip entry-points), YAML manifest parsing, load gating by kind/source
- **PluginContext** — Registration API: tools, hooks, slash commands, skills, providers, platforms, message injection, LLM facade
- **Hook system** — 17 lifecycle hooks (`pre_tool_call`, `post_tool_call`, `transform_llm_output`, `on_session_start/end`, `pre_gateway_dispatch`, `pre_approval_request`, etc.), `invoke_hook()` with per-callback error isolation, pre_tool_call blocking, context injection, LLM output transformation
- **Provider traits** — `ImageGenProvider`, `VideoGenProvider`, `WebSearchProvider`, `BrowserProvider`, `MemoryProvider`, `ContextEngine`, `ProviderProfile` — each with registry, config-driven selection, `is_available()` gating
- **Plugin configuration** — `plugins.enabled` (opt-in allow-list), `plugins.disabled` (deny-list), kind-based load gating (bundled backends auto-load, user plugins gated)
- **Plugin slash commands** — `/cmd` registration with async handling (30s timeout), TUI toolset grouping, conflict resolution against built-in commands
- **Tool whitelisting** — Thread-local per-thread tool restriction for sub-agent threads
- **Plugin skills** — Qualified names (`plugin:skill`), lookup/resolution, system prompt integration
- **Plugin introspection** — `list_plugins()`, debug logging (`HERMES_PLUGINS_DEBUG`)
- Parity doc: `docs/PRD-plugin-parity.md` (0/14 done)

### M9: Advanced Tools 🟡
- **Search provider** (🟡 high) — DuckDuckGo, Brave configurable backends
- **Browser automation** (🟡 high) — CUA-driver for macOS GUI automation
- **Voice** (🟡 high) — STT/TTS (Whisper, Edge TTS, ElevenLabs)
- **Image generation** (🟡 high) — FLUX, DALL-E, Midjourney backends (via plugin providers)
- **Delegate tool** (🟡 high) — Subagent delegation via `delegate_tool`
- **Kanban tools** (🟡 high) — Task management board
- **Computer use** (🟡 high) — macOS GUI control
- **MCP integration** (🟢 medium) — Model Context Protocol server/client
- **Cron scheduler** (🟢 medium) — Scheduled task delivery
- **Video generation** (🟢 medium) — Video generation backends
- **Home Assistant** (🟢 medium) — Smart home integration
- **Mixture of Agents** (🟢 medium) — Multi-agent collaboration
- Parity doc: `docs/PRD-tools-parity.md` (14/26 done)

### M10: Polish & Platform Features 🟢
- **TUI completion** — Syntax highlighting, multiline editing, slash commands, plugin toolset display, plugin introspection panel
- **Web dashboard** — Session browser, tool call visualization, plugin manager UI
- **Config enhancements** — Multi-provider config with fallback chains, platform token management, backup/restore (session + skill), doctor/diagnostics, profile management (named config sets), MCP config
- **Skills hub** — Install skills from GitHub, URL, skill bundles, remote sync, provenance tracking, guardian validation, skill commands
- **Goals advanced** — Plan decomposition (`kanban_decompose`), swarm planning, checkpoint manager, session recap
- **Utilities** — Rate limit tracking, usage pricing, credential management, trajectory compressor, checkpoint system, clipboard integration
- **I18n** — Multi-language support (locales)
- Parity docs: `docs/PRD-cli-parity.md` (3/11 done), `docs/PRD-skills-parity.md` (5/12 done), `docs/PRD-goals-parity.md` (5/10 done), `docs/PRD-utils-parity.md` (4/15 done)

---

## Not Yet Implemented

| Area | Priority | Hermes Equivalent | Description |
|------|----------|-------------------|-------------|
| **Provider integrations** | P0 | `agent/transports/` | Anthropic native, AWS Bedrock, Google Gemini |
| **Platform adapters** | P1 | `gateway/` | Telegram, Discord, Slack, WhatsApp/Signal |
| **Tool: Search** | P1 | `tools/search_tool.py` | Configurable search provider (DuckDuckGo, Brave, etc.) |
| **Tool: Browser** | P1 | `tools/browser_dialog_tool.py` | CUA-driver for macOS GUI automation |
| **Tool: Voice** | P1 | `tools/tts_tool.py` | STT/TTS (Whisper, Edge TTS, ElevenLabs) |
| **Tool: Image** | P1 | `agent/image_gen_provider.py` | FLUX, DALL-E, Midjourney |
| **Tool: MCP** | P2 | `tools/mcp_oauth.py` | Model Context Protocol server/client |
| **Tool: Cron** | P2 | `hermes_cli/cron.py` | Scheduled tasks delivery |
| **Tool: File sync** | P2 | `tools/environments/` | Remote workspace sync |
| **Skill install** | P2 | `skills/` hub | Install skills from GitHub, URL, or agentskills.io |
| **Config: Multi-provider** | P1 | `hermes_cli/models.py` | Provider fallback chains |
| **Config: Platform tokens** | P1 | `hermes_cli/auth.py` | API keys for Telegram, Discord, etc. |
| **TUI / Dashboard** | P2 | `web/` | Terminal UI panels (started: chat/config/sessions/setup), web dashboard |
| **I18n** | P3 | `locales/` | Multi-language support |

---

## Non-Goals
- Porting the Python UI-tui frontend (will build Rust-native TUI later)
- Keeping any Python dependencies at runtime
- Full feature parity in v0.1 — incremental is fine

---

## Metrics

| Metric | Target | Current |
|--------|--------|---------|
| Workspace compiles | ✅ 100% | ✅ 14/14 crates |
| Tests | 80%+ | ✅ 506/507 passing (14 crates) |
| Provider transports | 6+ | 1/7 (ChatCompletions) — M6 targets Anthropic/Bedrock/Gemini |
| Built-in tools | 25+ | 18 (terminal, read, write, http_get, web_search, search_files, patch, web_extract, vision_analyze, memory, clarify, todo, code_execution, osv_check, skill, plus more) + auto-registration — M9 targets 25+ |
| Skill categories | 20+ | ✅ 25/25 implemented |
| Curator | 1 | ✅ Complete (usage, lifecycle, scheduler) |
| Platform adapters | 5+ | 0/5 (trait defined) — M7 targets Telegram/Discord/Slack/WhatsApp |
| CLI commands | 30+ | 10 (`chat, run, setup, config, tools, skills, sessions, info, models, agent`) — M10 adds plugin management, backup, doctor, cron, profiles |

---

## Notes

- This is a full feature port — not a rewrite from scratch
- We leverage Rust's strengths: memory safety, async performance, zero-cost abstractions
- The architecture maps directly to Hermes Python modules for ease of migration
- All code follows Rust conventions; doc comments map to Hermes module-level docstrings
