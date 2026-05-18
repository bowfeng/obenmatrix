# PRD: Hermes Agent → Rust Port

> **Status:** 🟡 In Progress  
> **Started:** 2026-05-17  
> **Target:** Complete port of [nousresearch/hermes-agent](https://github.com/nousresearch/hermes-agent) v0.14.0  
> **Scope:** 100% Rust, no Python dependencies at runtime

---

## 1. Objective

Port the full Hermes Agent codebase to Rust, preserving:
- All toolset integrations (shell, read/write, web, search, etc.)
- All provider support (OpenAI, Anthropic, Bedrock, etc.)
- Gateway platforms (Telegram, Discord, Slack, etc.)
- Memory system (session search, skill curation)
- Skill loader (YAML/TXT)
- Context compression & budget tracking
- Self-improvement loop (create/improve skills from experience)
- Setup wizard & config management

## 2. Architecture

```
obenagent/               # Workspace root (binary crate)
├── Cargo.toml           # Workspace manifest
├── src/main.rs          # CLI entry + wire everything together
├──oben-models/          # Core types: Message, Tool, Skill, Session, Provider
├──oben-utils/           # Logging, terminal, path security, env
├──oben-config/          # YAML config, setup wizard, defaults
├──oben-core/            # Conversation loop, context mgmt, compression, budget
├──oben-transport/       # Provider transport layer (chat completions, etc.)
├──oben-tools/           # Tool registry + built-in tools (shell, rw, web, search)
├──oben-memory/          # Session storage, cross-session search, skill curation
├──oben-skills/          # Skill loader & system
└──oben-gateway/         # Platform adapters (Telegram, Discord, Slack, etc.)
```

## 3. Implementation Phases

### Phase 0: Foundation ✅ (Done)
- [x] Workspace scaffold (9 crates + binary)
- [x] `oben-models` — core type definitions (Message, Tool, ToolCall, ToolResult, Skill, Session, Provider)
- [x] `oben-utils` — logging, terminal spinner, path security, env helpers
- [x] `oben-config` — YAML config loader, setup wizard, defaults
- [x] Build pipeline — compiles and runs

### Phase 1: Core Engine 🟡 (In Progress)
- [x] Basic conversation loop skeleton
- [x] Context manager (message tracking)
- [x] Prompt builder
- [x] Iteration budget
- [ ] Full conversation loop (model call → tool dispatch → retries → compression → post-turn hooks)
- [ ] Compression algorithm (summarization / truncation)
- [ ] Self-improvement loop (skill creation & improvement)
- [ ] Cron / autonomous mode
- [ ] ACP adapter (Agent Communication Protocol)

### Phase 2: Toolset Integration 🟡 (In Progress)
- [x] Tool registry
- [x] Shell tool (with path security)
- [x] Read/Write file tools
- [x] HTTP tool
- [x] Search tools
- [ ] Web tools (browser automation, scraping)
- [ ] Code tools (LSP, diff, patch)
- [ ] Git tools
- [ ] Email tools
- [ ] Calendar / scheduling tools
- [ ] Custom plugin system

### Phase 3: Provider Transport 🟢 (Partial)
- [x] OpenAI-compatible chat completions transport
- [ ] Anthropic API transport
- [ ] Bedrock transport
- [ ] Ollama / local model transport
- [ ] Provider routing & fallback
- [ ] Cost tracking & budget enforcement per provider
- [ ] Multi-model chaining

### Phase 4: Memory & Skills 🟡 (Partial)
- [x] Memory manager (session storage)
- [ ] Session search (vector / keyword)
- [ ] Skill curation (load, improve, create)
- [ ] Skill system (YAML/TXT loader → active skills in context)
- [ ] Cross-session recall
- [ ] Experience replay

### Phase 5: Gateway & Platforms 🟡 (Partial)
- [x] Gateway skeleton
- [ ] Telegram adapter
- [ ] Discord adapter
- [ ] Slack adapter
- [ ] Webhook adapter
- [ ] MCP server adapter
- [ ] Multi-platform message routing

### Phase 6: CLI & UX 🟡 (Partial)
- [x] CLI skeleton with clap
- [x] Commands: chat, run, setup, config, tools, skills, sessions, info
- [ ] Full interactive chat (ANSI, spinner, pretty output)
- [ ] TUI / rich terminal UI
- [ ] Session persistence & resume
- [ ] Hot-reload config
- [ ] Batch runner (CLI)
- [ ] Security advisories
- [ ] Backup / restore

### Phase 7: Advanced Features 🔴 (Not Started)
- [ ] Autonomous AI agent (always-on, self-directed)
- [ ] Mini-SWE runner
- [ ] Cron job system
- [ ] Plugin system (hermes-cli plugins)
- [ ] Docker / Nix packaging
- [ ] Localization (locales)
- [ ] ACPR (agent communication protocol) registry
- [ ] Bedrock / Anthropic specific adapters

## 4. Non-Goals
- Porting the Python UI-tui frontend (will build Rust-native TUI later)
- Keeping any Python dependencies at runtime
- Full feature parity in v0.1 — incremental is fine

## 5. Key Decisions
- Use `tokio` for async runtime
- Use `tracing` + `tracing-subscriber` for logging
- Use `serde` + `serde_yaml` + `serde_json` for serialization
- Use `clap` for CLI
- Use `reqwest` for HTTP
- Use `thiserror` for error handling
- All workspace members share common `rust-version = "1.80"`

## 6. Progress Tracker

| Date | Status | Notes |
|------|--------|-------|
| 2026-05-17 | 🟡 In Progress | Workspace scaffolded, Phase 0 complete, Phase 1-3 partially done |

---

*This PRD serves as the master tracking document for the Hermes Agent Rust port. Update progress as you go.*
