# PRD: ObenAgent — Hermes Agent Rust Port

**Author:** ellie  
**Created:** 2026-05-17  
**Status:** 🟡 In Progress  
**Target:** MVP first (core loop + tools + CLI + streaming), then iterate outward toward full feature parity  
**Language:** Rust (async, multi-threaded, tokio runtime)

---

## Overview

| Metric | Status |
|--------|--------|
| Crates | 9/9 compiling |
| Tests | 121/121 passing |
| CLI commands | 9 subcommands |
| Provider transports | 1/7 (ChatCompletions) |
| Built-in tools | 4/20+ |
| Skill categories | 1/20+ |

**Status: 🟡 Phase 1 — Core Engine (In Progress)**

```
✅ Foundation       Models • Utils • Config • Skills • Memory • Gateway
✅ Engine           ConversationLoop • ContextManager • Budget • Compression
✅ Transport        OpenAI-compatible (streaming + SSE)
✅ CLI              chat run setup config tools skills sessions info models
🟡 Anthropic        Work in progress
🟡 Bedrock          Work in progress
🟡 Gemini           Work in progress
🔴 Browser          Not started
🔴 Voice/STT        Not started
🔴 Cron/Scheduler   Not started
🔴 TUI/Dashboard    Not started
```

### ✅ Done

| Component | Status | Details |
|-----------|--------|---------|
| **Workspace** | ✅ | 9 sibling crates + binary, shared deps, compiles cleanly |
| **oben-models** | ✅ | Message, Tool, Skill, Session, ProviderConfig, TransportProvider trait, ModelInfo/ModelListResponse |
| **oben-utils** | ✅ | Logging (tracing, --verbose), terminal spinner, path security, env helpers, table formatter |
| **oben-config** | ✅ | YAML config, setup wizard (interactive), system prompt defaults, gateway config serialization |
| **oben-core** | ✅ | ConversationLoop, ContextManager, PromptBuilder, ContextCompressor, IterationBudget, streaming support |
| **oben-transport** | ✅ | BaseHTTPTransport, ChatCompletionsTransport (OpenAI-compatible), SSE streaming via eventsource-stream |
| **oben-tools** | ✅ | ToolRegistry (dynamic), shell, read_file, write_file, http_get, search (stub) |
| **oben-memory** | ✅ | MemoryManager (session CRUD + JSON persistence), full-text search, skill curation |
| **oben-skills** | ✅ | SkillLoader (YAML/TXT/MD from disk), SkillManager (enable/disable/auto-use/instruction assembly) |
| **oben-gateway** | ✅ | Gateway struct, PlatformAdapter trait, Incoming/OutgoingMessage, mock adapter support |
| **CLI (obenagent)** | ✅ | 9 subcommands: chat, run, setup, config, tools, skills, sessions, info, models |
| **Models** | ✅ | Discovery via /v1/models, table output, wizard auto-detects max_model_len |
| **Streaming** | ✅ | --stream CLI flag on `chat` and `run` subcommands, SSE per-token output, delta_callback, tool call accumulation across chunks |
| **Tests** | ✅ | 145 tests across 9 crates (oben-models: 30, config: 6, core: 21, transport: 36, tools: 6, memory: 7, skills: 20, gateway: 13, utils: 6). Transport includes 25 unit + 11 integration tests using wiremock against OpenAI-compatible API.

### 🟡 In Progress

| Component | Status | Details |
|-----------|--------|---------|
| **Anthropic transport** | 🟡 | Struct in progress |
| **Bedrock transport** | 🟡 | Struct in progress |
| **Gemini transport** | 🟡 | Struct in progress |
| **Platform adapters** | 🟡 | Telegram, Discord, Slack (trait defined, implementations TBD) |

### 🔴 Post-MVP (Out of Scope for MVP)

> **MVP = core loop + 4 tools + 1 provider + CLI + streaming.** Everything else below is deferred to later milestones.

| Component | Target Phase | Description |
|-----------|-------------|-------------|
| **Anthropic/Bedrock/Gemini** | Phase 2 | Native provider transports (OpenAI-compatible already works) |
| **Platform adapters** | Phase 3 | Telegram, Discord, Slack |
| **Browser/voice/image/tools** | Phase 4 | CUA-driver, STT/TTS, FLUX/DALL-E, cron, MCP |
| **Skill categories** | Phase 4 | 20+ categories, self-improvement loop |
| **TUI / dashboard / i18n** | Phase 5 | Rich terminal UI, web dashboard, multi-language |

---

## Vision

Build a self-improving AI agent in Rust, porting the full functionality of Hermes Agent. It creates and improves skills from experience, supports multiple LLM providers, runs anywhere (VPS, GPU cluster, serverless), and communicates via CLI, Telegram, Discord, Slack, and other platforms.

## Key Design Principles

- **Performance-first** — Rust gives us the speed and memory safety for resource-constrained deployments
- **Multi-provider** — OpenAI-compatible APIs, Anthropic, Bedrock, Gemini, custom endpoints
- **Extensible** — Plugin-style tool system, YAML/TXT skill definitions, modular architecture
- **Run anywhere** — Local, Docker, SSH, Modal, Daytona, Vercel Sandbox

---

## Architecture

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
│   └── providers.rs     # ProviderConfig, TransportProvider trait
│
├── oben-utils/          # Shared utilities
│   ├── logging.rs       # tracing-subscriber initialization
│   ├── terminal.rs      # Spinner, progress indicators
│   ├── path_security.rs # Path traversal prevention
│   └── env_utils.rs     # Environment variable helpers
│
├── oben-config/         # Configuration
│   ├── config.rs        # AppConfig (YAML-based, ~/.oben/config.yaml)
│   ├── defaults.rs      # Default system prompt, provider defaults
│   └── wizard.rs        # Interactive setup wizard (clap + dialoguer)
│
├── oben-core/           # Agent engine
│   ├── conversation.rs  # ConversationLoop — main turn cycle
│   ├── context.rs       # ContextManager — message tracking + token estimation
│   ├── prompt.rs        # PromptBuilder — system prompt + message assembly
│   ├── compression.rs   # ContextCompressor — summary/ttoken_count strategies
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
├── oben-memory/         # Persistent memory
│   ├── manager.rs       # MemoryManager — session CRUD, disk persistence (JSON)
│   ├── search.rs        # search_sessions — full-text search with relevance scoring
│   └── skill_curation.rs # SkillCurator — learns skills from usage patterns
│
└── oben-gateway/        # Messaging gateway
    ├── gateway.rs       # Gateway — route messages from platforms to agent
    └── platform.rs      # PlatformAdapter trait — Telegram/Discord/Slack adapters
```

## Current Status

### ✅ Completed (Scaffolded & Compiling)

| Area | Status | Notes |
|------|--------|-------|
| Workspace setup | ✅ | 9 crates + binary, workspace dependency management |
| Core types | ✅ | Message, Tool, Skill, Session, ProviderConfig |
| Configuration | ✅ | YAML config, setup wizard, system prompt defaults |
| Utils | ✅ | Logging (tracing), terminal spinner, path security |
| Agent engine | ✅ | ConversationLoop, context management, prompt building, compression |
| LLM transport | ✅ | Base HTTP client + ChatCompletionsTransport (OpenAI-compatible) |
| Tool registry | ✅ | Dynamic registration, async handlers, 4 built-in tools |
| Skill system | ✅ | Loader (YAML/TXT/MD), manager (enable/disable/auto-use) |
| Memory | ✅ | Session persistence (JSON), full-text search, skill curation |
| CLI | ✅ | clap-based with chat/run/setup/config/tools/skills/sessions/info |

### 🟡 In Progress

| Area | Status | Notes |
|------|--------|-------|
| Transport: Anthropic | 🟡 | Struct in progress |
| Transport: Bedrock | 🟡 | Struct in progress |
| Transport: Gemini | 🟡 | Struct in progress |
| Tool: Search | 🟡 | Stub — needs provider integration |
| Gateway: Platform adapters | 🟡 | Trait defined, Telegram/Discord/Slack TBD |
| CLI: Interactive TUI | 🟡 | Basic prompt loop exists, full TUI TBD |

### 🔴 Not Yet Implemented

| Area | Priority | Hermes Equivalent | Description |
|------|----------|-------------------|-------------|
| **Provider integrations** | P0 | `agent/transports/` | Anthropic native, AWS Bedrock, Google Gemini, LMStudio |
| **Transport: vLLM/Custom** | ✅ DONE | `hermes_cli/models.py` | Custom endpoint with base_url, local model support (qwen35-local verified) |
| **Tool: Browser automation** | P1 | `tools/browser_dialog_tool.py` | CUA-driver integration for macOS GUI automation |
| **Tool: Voice (STT/TTS)** | P1 | `tools/tts_tool.py`, `tools/transcription_tools.py` | Edge TTS, Whisper, ElevenLabs, OpenAI |
| **Tool: Image generation** | P1 | `agent/image_gen_provider.py` | FLUX, DALL-E, Midjourney integration |
| **Tool: File sync** | P2 | `tools/environments/file_sync.py` | Sync workspace to remote environments |
| **Tool: Cron scheduler** | P2 | `hermes_cli/cron.py` | Schedule tasks via croniter, deliver to any platform |
| **Tool: MCP integration** | P2 | `tools/mcp_oauth.py`, `MCP stdio` | Model Context Protocol server/client |
| **Tool: Vercel/Modal/Daytona** | P2 | `tools/environments/` | Remote environment backends |
| **Platform: Telegram** | P1 | `gateway/telegram.py` | Bot integration with webhooks |
| **Platform: Discord** | P2 | `gateway/discord.py` | Bot with slash commands |
| **Platform: Slack** | P2 | `gateway/slack.py` | Bolt app with webhooks |
| **Platform: WhatsApp/Signal** | P3 | `gateway/whatsapp.py` | Future platform support |
| **Skill: Built-in skills** | P2 | `skills/` (20+ categories) | Devops, github, mcp, media, smart-home, etc. |
| **Agent: Self-improvement** | P2 | `agent/curator.py` | Auto-create skills from repeated tool usage |
| **Agent: Context compression** | P2 | `agent/context_compressor.py` | LLM-based summarization of old messages |
| **Agent: Session search** | P1 | `tools/session_search_tool.py` | FTS5-style search across past conversations |
| **Agent: Background review** | P3 | `agent/background_review.py` | Periodic memory/skill review nudges |
| **Agent: Trajectory compression** | P3 | `trajectory_compressor.py` | Research trajectory compression for training |
| **CLI: Dashboard** | P3 | `web/` (FastAPI + SPA) | Localhost web dashboard |
| **CLI: Proxy mode** | P3 | `hermes_cli/proxy/` | Run agent behind reverse proxy |
| **CLI: Cron commands** | P2 | `hermes_cli/cron.py` | `hermes cron start`, `hermes cron list` |
| **Config: Multi-provider** | P1 | `hermes_cli/models.py` | Configure multiple providers, fallback chains |
| **Config: Platform tokens** | P1 | `hermes_cli/auth.py` | Store API keys for Telegram, Discord, etc. |
| **Config: Tool sets** | P2 | `toolset_distributions.py` | Pre-defined tool bundles (minimal, full, etc.) |
| **I18n** | P3 | `locales/` | Multi-language support |
| **Security audit** | P1 | `tools/tirith_security.py` | Security policy enforcement |

---

## Non-Goals
- Porting the Python UI-tui frontend (will build Rust-native TUI later)
- Keeping any Python dependencies at runtime
- Full feature parity in v0.1 — incremental is fine

## Key Decisions
- Use `tokio` for async runtime
- Use `tracing` + `tracing-subscriber` for logging
- Use `serde` + `serde_yaml` + `serde_json` for serialization
- Use `clap` for CLI
- Use `reqwest` for HTTP
- Use `thiserror` for error handling
- All workspace members share common `rust-version = "1.80"`

### Transport Trait Location
Moved `TransportProvider` to `oben-models::providers` to break a circular dependency between `oben-core` and `oben-transport`.

### Tool Handler Type
Used `Arc<dyn Fn(Value) -> Pin<Box<dyn Future<Output = Result<ToolResult>>> + Send>>` to allow asynchronous tool execution.

### Streaming
Added `eventsource-stream` crate for SSE parsing. CLI has `--stream` flag. Streaming is used for the first LLM call (user's message), subsequent calls (after tool results) use non-streaming to keep tool dispatch deterministic.

## Milestones

### M1: Core Agent Loop ✅
- Workspace, types, config, conversation loop, transport, CLI
- Can run a one-shot prompt and interactive chat with any OpenAI-compatible API

### M2: Provider Coverage 🟡
- Anthropic native, Bedrock, Gemini transports
- Provider switching in CLI (`oben model`)

### M3: Messaging Gateway 🟡
- Telegram integration (webhook + polling)
- Message routing from platforms → agent → response delivery

### M4: Voice & Vision 🟡
- STT (Whisper/faster-whisper) for voice memos
- TTS (Edge TTS / ElevenLabs / OpenAI) for voice responses
- Image understanding for Vision tools

### M5: Skill Ecosystem 🟡
- Port all 20+ Hermes skill categories
- Self-improving skill creation from experience
- Skill marketplace / import from agentskills.io

### M6: Cron & Automation 🔴
- Scheduled tasks (daily reports, nightly backups, etc.)
- Delivery to any platform (Telegram, Discord, email, etc.)

### M7: Research & Training 🔴
- Trajectory compression for ML training
- Batch trajectory generation

### M8: Polish 🔴
- TUI with syntax highlighting, multiline editing, slash commands
- Web dashboard
- Multi-language (i18n)
- Integration tests for full conversation loop

---

## Metrics

| Metric | Target | Current |
|--------|--------|---------|
| Workspace compiles | ✅ 100% | ✅ Passing |
| Provider integrations | 6/7 | 2/7 (ChatCompletions + Custom/vLLM) |
| Built-in tools | 20+ | 4/20 |
| Skill categories | 20+ | 1/20 (general) |
| Platform adapters | 5+ | 0/5 (trait defined) |
| CLI commands | 30+ | 9/30 (`chat, run, setup, config, tools, skills, sessions, info, models`) |
| Tests | 80%+ | 121/121 passing (9/9 crates) |

## Recent Progress

| Date | Status | Notes |
|------|--------|-------|
| 2026-05-18 | ✅ Debug logging | `--verbose`/`-v` flag, `RUST_LOG` env var override. Logs to stderr. |
| 2026-05-18 | ✅ Model discovery | `oben models list` and `oben models info`. Setup wizard auto-detects `max_model_len`. |
| 2026-05-18 | ✅ Streaming | SSE via `eventsource-stream`, `--stream` CLI flag. Text streamed per token for ALL LLM calls (including tool-result-followed-by-LLM calls). |
| 2026-05-18 | ✅ Streaming | SSE streaming support in `oben-transport` via `eventsource-stream`. Delta callbacks. |

---

## Notes

- This is a full feature port — not a rewrite from scratch
- We leverage Rust's strengths: memory safety, async performance, zero-cost abstractions
- The architecture maps directly to Hermes Python modules for ease of migration
- All code follows Rust conventions; doc comments map to Hermes module-level docstrings
