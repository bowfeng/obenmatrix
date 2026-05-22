# PRD: ObenAgent — Hermes Agent Rust Port

**Author:** ellie  
**Created:** 2026-05-17  
**Status:** 🟡 In Progress — P2 Skills & Curator  
**Target:** MVP first (core loop + tools + CLI + streaming), then iterate outward toward full feature parity  
**Language:** Rust (async, multi-threaded, tokio runtime)

---

## Overview

| Metric | Status |
|--------|--------|
| Crates | 11/11 compiling |
| Tests | 406/406 passing |
| CLI subcommands | 9 |
| Provider transports | 1/7 (OpenAI-compatible ChatCompletions) |
| Built-in tools | 15 |
| Skill categories | 25/20+ ✅ |

**Status: 🟡 Phase 1 — Core Engine**

```
✅ Foundation       Models • Utils • Config • Skills • Memory • Gateway • Curator
✅ Engine           ConversationLoop • ContextEngine • Budget • Compression (merged)
✅ Sessions         ober-session: lifecycle + LLM compaction algorithm
✅ Transport        OpenAI-compatible (streaming + SSE)
✅ CLI              chat run setup config tools skills sessions info models
✅ Goals            Autonomous loop with plan parsing, judge verdict, state machine
✅ Curator          Skill lifecycle management (active→stale→archived)
🔴 Provider        Anthropic • Bedrock • Gemini • Custom endpoints
🔴 Platform        Telegram • Discord • Slack adapters
🔴 Advanced Tools   Search • Browser • Voice • Image • MCP • Cron
🔵 Web extract — HTML content extraction with SSRF protection
🔵 Vision analyze — image download + base64 encoding + analysis
🔵 Memory — persistent curated memory (MEMORY.md + USER.md), add/replace/remove
🔴 TUI / Dashboard  Not started
```

---

## Recent Progress

| Date | Status | Notes |
|------|--------|-------|
| 2026-05-18 | ✅ LLM-based summarization | `generate_summary()` no longer a stub — actually calls LLM via `reqwest` with structured prompt template (Active Task, Goal, Constraints, Completed Actions, etc.), iterative updates, and focus topic support. Falls back to static placeholder on LLM failure. CLI: `oben sessions compact [-s SESSION] [-f FOCUS]` |

## Done ✅

| Crate | Tests | Status | Details |
|-------|-------|--------|---------|
| **oben-models** | 34 | ✅ | Message, Tool, Skill, Session, ProviderConfig, TransportProvider trait, ModelInfo, ModelListResponse |
| **oben-utils** | 6 | ✅ | Logging (tracing + `--verbose`/`RUST_LOG`), terminal spinner, path security, env helpers, table formatter |
| **oben-config** | 6 | ✅ | YAML config, setup wizard (interactive), system prompt defaults, gateway config serialization, model discovery |
| **oben_conversation** | 27 | ✅ | ConversationLoop, ContextEngine (buffer + token tracking + compression trigger), full compaction algorithm (compact_session_messages, config_from_app, CompactCofig/Result), PromptBuilder, streaming + non-streaming turns |
| **oben-transport** | 64 | ✅ | BaseHTTPTransport, ChatCompletionsTransport (OpenAI-compatible), SSE streaming via `eventsource-stream`, 53 unit + 11 integration tests with wiremock |
| **oben-tools** | 95 | ✅ | ToolRegistry + auto-registration, terminal (fg/bg + mgmt), read_file, write_file, http_get, web_search, search_files (ripgrep), patch (fuzzy), web_extract (SSRF + HTML), vision_analyze (image base64), memory (add/replace/remove + scan), clarify, todo (JSON store), code_execution (sandbox), osv_check, skill (list/view), 95 unit tests |
| **oben-sessions** | 28 | ✅ | SessionDB (SQLite-backed session state engine with FTS5, message windows, lineage resolution), Rich Search (discover/scroll/browse shapes), Bounded MemoryStore (file locking, atomic writes, injection scanning, frozen snapshots). Legacy JSONL SessionManager preserved for backwards compatibility.
| **oben-skills** | 70 | ✅ | SkillLoader (recursive SKILL.md discovery, YAML/TXT/MD), SkillManager (enable/disable/auto-use/instruction assembly + preprocessing config), frontmatter parsing, platform matching, tags/config/conditions extraction, qualified name parsing, external dirs support, skill_preprocessing (template vars ${SKILL_DIR}/${SESSION_ID}, inline shell !`cmd` expansion), 70 unit tests |
| **oben-goals** | 30 | ✅ | PlanNode (tree, builder, artifacts), PlanState (find/count/markdown/save/load), judge verdict parser, GoalState (turn budget), plan parser, node complete/failure parser |
| **oben-gateway** | 13 | ✅ | Gateway struct, PlatformAdapter trait, Incoming/OutgoingMessage, mock adapter support |
| **oben-curator** | 17 | ✅ | Usage tracking (use/view/patch counts), lifecycle states (active→stale→archived), scheduler (pause/resume), report generation (text + JSON) |

**Total: 392 tests passing across 11 crates**

---

## Workspace Structure

```
obenagent/               # Root workspace (binary)
├── Cargo.toml           # Workspace config + root package
├── src/main.rs          # CLI entry point (clap-based)
│
├── oben-models/         # Core domain types
│   ├── messages.rs      # Message, MessageContent, MessageRole
│   ├── tools.rs         # Tool, ToolCall, ToolResult, ToolBuilder
│   ├── skills.rs        # Skill definition, SkillBuilder
│   ├── session.rs       # Conversation session storage
│   └── providers.rs     # ProviderConfig, TransportProvider trait, ModelInfo
│
├── oben-utils/          # Shared utilities
│   ├── logging.rs       # tracing-subscriber initialization
│   ├── terminal.rs      # Spinner, progress indicators
│   ├── path_security.rs # Path traversal prevention
│   ├── env_utils.rs     # Environment variable helpers
│   └── table.rs         # Table formatter for CLI output
│
├── oben-config/         # Configuration
│   ├── config.rs        # AppConfig (YAML-based, ~/.obenagent/config.yaml)
│   ├── defaults.rs      # Default system prompt, provider defaults
│   └── wizard.rs        # Interactive setup wizard (clap + dialoguer)
│
├── oben_conversation/   # Agent engine
│   ├── conversation.rs  # ConversationLoop — main turn cycle (streaming + non-streaming)
│   ├── context.rs       # ContextEngine — unified: buffer, real token tracking, should_compress(), compress()
│   ├── prompt.rs        # PromptBuilder — system prompt + message assembly
│   ├── compression.rs   # Full compaction: compact_session_messages(), config_from_app(), CompactCofig/Result/Stats
│   ├── budget.rs        # IterationBudget — turn limits per conversation
│   └── transport.rs     # Re-exports TransportProvider from oben-models
│
├── oben-transport/      # LLM transport implementations
│   ├── base.rs          # BaseTransport — HTTP client, request/response types
│   └── chat_completions.rs # ChatCompletionsTransport — OpenAI-compatible API
│
├── oben-tools/          # Tool implementations
│   ├── registry.rs      # ToolRegistry — dynamic tool registration/dispatch
│   ├── shell.rs         # Shell tool — safe command execution
│   ├── read_write.rs    # read_file / write_file tools
│   ├── web.rs           # http_get tool
│   └── search.rs        # Web search (stub — configurable provider)
│
├── oben-skills/         # Skill system
│   ├── loader.rs        # SkillLoader — reads YAML/TXT/MD from disk
│   └── system.rs        # SkillManager — enable/disable, auto-use, instruction assembly
│
├── oben-goals/          # Goal tracking, plan management, judge loop
│   ├── plan.rs          # PlanNode (tree with builder), artifacts
│   ├── plan_parser.rs   # parse_plan_from_markdown()
│   ├── plan_state.rs    # PlanState — CRUD, markdown, save/load
│   ├── judge.rs         # Judge verdict types
│   ├── verdict.rs       # parse_judge_response()
│   ├── goal_loop.rs     # run_goal_loop(), create_plan_from_goal()
│   ├── goal_loop/goal_state.rs  # GoalState, GoalStatus
│   └── goal_loop/transport.rs   # Goal transport trait
│
├── oben-curator/        # Skill lifecycle management
│   ├── usage.rs         # UsageRecord, usage tracking (use/view/patch)
│   ├── lifecycle.rs     # LifecycleState (active/stale/archived/pinned)
│   ├── curator.rs       # Curator orchestrator + CuratorState (scheduler)
│   └── report.rs        # Human-readable + JSON report generation
│
├── oben-sessions/       # Persistent memory & session management
│   ├── manager.rs       # SessionDB (SQLite-backed session state engine with FTS5, message windows, lineage) + legacy SessionManager (JSONL backwards compat)
│   ├── search.rs        # RichSearch (discover/scroll/browse shapes, FTS5-backed)
│   └── skill_curation.rs # MemoryStore (bounded entries, file locking, atomic writes, injection scanning, frozen snapshots)
│
└── oben-gateway/        # Messaging gateway
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
Moved `TransportProvider` to `oben-models::providers` to break a circular dependency between `oben_conversation` and `oben-transport`.

### Tool Handler Type
Used `Arc<dyn Fn(Value) -> Pin<Box<dyn Future<Output = Result<ToolResult>>> + Send>>` to allow asynchronous tool execution.

### Goal Loop Closure Pattern
`run_goal_loop` takes a generic async closure `F: FnMut(&str) -> Fut` for maximum flexibility, avoiding a crate dependency on `oben_conversation`.

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
- `obenagent agent` subcommand

### M4: Skills & Curator ✅
- 25 built-in skill categories matching Hermes Agent's `skills/` layout
- SkillLoader reads YAML/TXT/MD from disk
- Curator crate: usage tracking, lifecycle (active→stale→archived), scheduler

### M5: Provider Coverage 🟡
- Anthropic native transport
- AWS Bedrock transport
- Google Gemini transport

### M6: Messaging Gateway 🟡
- Telegram integration (webhook + polling)
- Message routing from platforms → agent → response delivery

### M7: Advanced Tools 🔴
- Search provider integration
- Browser automation (CUA-driver)
- Voice (STT/TTS), image generation
- MCP integration, cron scheduler
- Vercel/Modal/Daytona environments

### M8: Polish 🔴
- TUI with syntax highlighting, multiline editing, slash commands
- Web dashboard
- Multi-language (i18n)

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
| **TUI / Dashboard** | P3 | `web/` | Terminal UI, web dashboard |
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
| Workspace compiles | ✅ 100% | ✅ 11/11 crates |
| Tests | 80%+ | ✅ 406/406 passing (11 crates) |
| Provider transports | 6+ | 1/7 (ChatCompletions) |
| Built-in tools | 20+ | 15 (terminal, read, write, http_get, web_search, search_files, patch, web_extract, vision_analyze, memory, clarify, todo, code_execution, osv_check, skill) + auto-registration |
| Skill categories | 20+ | ✅ 25/25 implemented |
| Curator | 1 | ✅ Complete (usage, lifecycle, scheduler) |
| Platform adapters | 5+ | 0/5 (trait defined) |
| CLI commands | 30+ | 9/30 (`chat, run, setup, config, tools, skills, sessions, info, models, agent`) |

---

## Recent Progress

| Date | Status | Notes |
|------|--------|-------|
| 2026-05-17 | ✅ Debug logging | `--verbose`/`-v` flag, `RUST_LOG` env var override |
| 2026-05-17 | ✅ Model discovery | `oben models list/info`, wizard auto-detects `max_model_len` |
| 2026-05-17 | ✅ Full streaming | SSE via `eventsource-stream`, per-token callbacks, tool call accumulation |
| 2026-05-17 | ✅ Goal loop | Autonomous agent loop with plan parsing, judge verdict, state machine |
| 2026-05-17 | ✅ P2 Skills | 25 built-in skill categories matching Hermes Agent's `skills/` layout |
| 2026-05-17 | ✅ Curator crate | Usage tracking, lifecycle management (active→stale→archived), scheduler |
| 2026-05-19 | ✅ SQLite sessions | SessionDB (SQLite + FTS5), Rich Search (discover/scroll/browse), MemoryStore (atomic writes, injection scanning). 28 tests. Legacy JSONL fallback preserved. |

---

## Notes

- This is a full feature port — not a rewrite from scratch
- We leverage Rust's strengths: memory safety, async performance, zero-cost abstractions
- The architecture maps directly to Hermes Python modules for ease of migration
- All code follows Rust conventions; doc comments map to Hermes module-level docstrings
