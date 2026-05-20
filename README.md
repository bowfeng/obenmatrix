# ObenAgent

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.80%2B-orange.svg)](https://www.rust-lang.org)
[![Build Status](https://img.shields.io/badge/tests-275%2F275-passing-brightgreen.svg)]()

**ObenAgent** is a self-improving AI agent written in Rust. It creates and evolves skills from experience, supports multiple LLM providers, and can communicate via CLI, terminal chat, and eventually messaging platforms (Telegram, Discord, Slack).



---

## Key Features

| Feature | Status |
|---------|--------|
| 🦀 **Rust Performance** | ✅ Async, multi-threaded, tokio runtime |
| 💬 **Interactive Chat** | ✅ Streaming + non-streaming modes |
| 🔄 **Session Memory** | ✅ JSON persistence, LLM-based compaction |
| 🧠 **Autonomous Goals** | ✅ Plan parsing, judge verdict, turn budgets |
| 🔧 **Tool Registry** | ✅ Shell, file I/O, HTTP, dynamic dispatch |
| 🧹 **Skill Curator** | ✅ Usage tracking, lifecycle (active→stale→archived) |

### In Progress

| Feature | Status |
|---------|--------|
| 🛠️ **Skills** | 🟡 25 skill definitions present, under active development |
| 🔧 **Setup Wizard** | 🟡 Interactive config, needs polish |
| 🔍 **Model Discovery** | 🟡 Provider listing implemented, needs refinement |
| 🤖 **More Providers** | 🟡 OpenAI-compatible ✅ · Anthropic/Bedrock/Gemini planned |
| 📱 **Platform Adapters** | 🟡 Telegram/Discord/Slack trait defined |
| 🌐 **Advanced Tools** | 🔴 Search, Browser, Voice, Image, MCP, Cron |

---

## Getting Started

### Prerequisites

- Rust 1.80+ (`rustup` recommended)
- An OpenAI-compatible API key (OpenAI, Ollama, vLLM, any server)

### Installation

```bash
# Clone the repository
git clone https://github.com/bowfeng/obenagent.git
cd obenagent

# Build
cargo build --release

# Install to ~/.cargo/bin
cargo install --path .
```

### Usage

```bash
# Interactive chat (streaming)
obenagent chat

# One-shot prompt
obenagent run -p "What is the capital of France?"

# Stream output
obenagent run -p "Explain quantum computing" --stream

# List available tools
obenagent tools

# List skills (work in progress)
obenagent skills

# Discover models from your provider (experimental)
obenagent models list

# Manage sessions
obenagent sessions list
obenagent sessions compact [-s session-id]

# Show agent info
obenagent info
```

---

## Architecture

ObenAgent is a Rust workspace with 11 crates:

```
obenagent/               # Root workspace (binary)
├── oben-models/         # Core domain types (messages, tools, skills, sessions, providers)
├── oben-utils/          # Shared utilities (logging, spinner, table formatter)
├── oben-config/         # YAML config, setup wizard, defaults
├── oben-conversation/   # Agent engine — conversation loop, context engine, compression
├── oben-transport/      # LLM transport (OpenAI-compatible ChatCompletions)
├── oben-tools/          # Tool implementations (shell, read, write, HTTP)
├── oben-skills/         # Skill system — loader, manager, 25 built-in categories
├── oben-goals/          # Autonomous loop — plan management, judge verdict, goal state
├── oben-curator/        # Skill lifecycle — usage tracking, scheduler, reports
├── oben-memory/         # Persistent memory — session CRUD, full-text search
└── oben-gateway/        # Messaging gateway — platform adapter trait
```

### Core Loop

1. **ConversationLoop** handles turn-by-turn interaction with an LLM
2. **ContextEngine** manages the message buffer, token tracking, and compaction triggers
3. **ToolRegistry** dispatches tool calls with dynamic dispatch and async support
4. **SkillManager** assembles skill instructions from enabled YAML/TXT/MD files
5. **Curator** tracks skill usage and manages lifecycle states

---

## Configuration

Configuration lives at `~/.obenagent/config.yaml`:

```yaml
model:
  api_key: "sk-..."
  endpoint: "https://api.openai.com/v1"
  model: "gpt-4o"
  max_model_len: 128000

context:
  max_messages: 100
  max_tokens: 128000

max_iterations: 50
```

Or edit `~/.obenagent/config.yaml` manually (setup wizard is coming soon)

---

## Skills

ObenAgent ships with **25 built-in skill categories** covering development, analysis, automation, and more. Skills are defined as YAML, TXT, or MD files — you can also drop your own into the `skills/` directory.

*(Skill system under active development)*

---

## Session Management

Sessions store conversation history and persist to disk as JSON. ObenAgent supports **LLM-based session compaction** to keep context windows manageable:

```bash
# List all sessions
obenagent sessions list

# Compact a session (summarizes older messages via LLM)
obenagent sessions compact -s my-session

# Compact with a focus topic
obenagent sessions compact -s my-session -f "database migration"

# Delete a session
obenagent sessions delete -s my-session
```

---

## Goals (Autonomous Mode)

Run the agent autonomously on a goal — it plans, acts via tools, and iterates:

```bash
obenagent agent
```

The agent:
1. Parses a goal into a plan (subtasks with dependencies)
2. Executes each plan node, using tools as needed
3. Self-evaluates via a judge verdict
4. Retries until the goal is complete or the turn budget is exhausted

---

## Testing

```bash
# Run all tests
cargo test

# Run tests for a specific crate
cargo test -p oben-tools
```

**275 tests** pass across all 11 crates.

---

## Project Status

See [docs/PRD.md](docs/PRD.md) for detailed milestones, progress tracking, and implementation details.

---

## License

This project is licensed under the [MIT License](LICENSE).

---


