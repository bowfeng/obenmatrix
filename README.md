# ObenAgent

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.80%2B-orange.svg)](https://www.rust-lang.org)
[![Build Status](https://img.shields.io/badge/tests-819%2F819-passing-brightgreen.svg)]()

**ObenAgent** is a self-improving AI agent written in Rust. It creates and evolves skills from experience, supports multiple LLM providers, and can communicate via CLI, terminal chat, and eventually messaging platforms (Telegram, Discord, Slack).

## 🚀 New in v0.2: Multi-Agent Collaboration

ObenAgent now supports running **multiple independent agents with role-based separation**:

- **Isolated Data**: Each agent has its own session memory, goals, and skill usage tracking
- **Role-Based Teams**: Define agents like "Researcher", "Writer", "Analyst" with custom roles
- **Inter-Agent Communication**: Topic-based pub/sub messaging for collaboration
- **Config-Driven**: Define all agents in a single YAML configuration file



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
| 👥 **Multi-Agent Systems** | ✅ Independent agents with role-based isolation |
| 📡 **Inter-Agent Messaging** | ✅ Topic-based pub/sub communication |
| 🏗️ **Agent Registry** | ✅ Manager for multiple concurrent agents |

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
git clone https://github.com/bowfeng/openalien.git
cd obenmatrix

# Build
cargo build --release

# Install to ~/.cargo/bin
cargo install --path .
```

### Usage

```bash
# Interactive chat (streaming)
obenmatrix chat

# One-shot prompt
obenmatrix run -p "What is the capital of France?"

# Stream output
obenmatrix run -p "Explain quantum computing" --stream

# List available tools
obenmatrix tools

# List skills (work in progress)
obenmatrix skills

# Discover models from your provider (experimental)
obenmatrix models list

# Manage sessions
obenmatrix sessions list
obenmatrix sessions compact [-s session-id]

# Show agent info
obenmatrix info
```

### Multi-Agent Mode

Define multiple agents with different roles in your config:

```yaml
agents:
  - name: "manager"
    role: "Orchestrates tasks and coordinates workers"
    model: "openai/gpt-4o"
    tools: ["web_search", "http_get"]
  - name: "researcher"
    role: "Finds information and analyzes data"
    model: "anthropic/claude-3-5-sonnet"
    tools: ["web_search"]
  - name: "writer"
    role: "Creates documents and reports"
    model: "openai/gpt-4o"
    tools: ["write_file", "read_file"]
```

Run the TUI with a specific agent:

```bash
# Start manager agent (sessions in ~/.obenmatrix/agents/manager/sessions.db)
obenmatrix tui --agent manager

# Start researcher agent (sessions in ~/.obenmatrix/agents/researcher/sessions.db)
obenmatrix tui --agent researcher

# Start worker agent (sessions in ~/.obenmatrix/agents/worker/sessions.db)
obenmatrix tui --agent worker
```

**Each agent has completely isolated data:**
- Session DB: `~/.obenmatrix/agents/<name>/sessions.db`
- Memories: `~/.obenmatrix/agents/<name>/memories/`
- Goals: `~/.oben-goals/<name>/`
- Usage stats: `~/.agents/<name>-usage_tracking.yaml`

---

## Architecture

ObenAgent is a Rust workspace with 11 crates:

```
obenmatrix/               # Root workspace (binary)
├── oben-models/         # Core domain types (messages, tools, skills, sessions, providers)
├── oben-utils/          # Shared utilities (logging, spinner, table formatter)
├── oben-config/         # YAML config, setup wizard, defaults
├── oben-agent/   # Agent engine — conversation loop, ContextWindowManager, compression
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

### Single Agent Mode

Default config lives at `~/.config/obenmatrix/config.yaml`
and data at `~/.obenmatrix/`.

```yaml
model:
  kind: OpenRouter
  model: "qwen/qwen3-235b:free"
  api_key: "sk-or-..."

context:
  max_messages: 100
  compression: summary
```

### Multi-Agent Mode

Define multiple agents in your config:

```yaml
agents:
  - name: "manager"
    role: "Orchestrates tasks and coordinates workers"
    model: "openai/gpt-4o"
    tools: ["web_search", "http_get"]
  - name: "researcher"
    role: "Finds information and analyzes data"
    model: "anthropic/claude-3-5-sonnet"
    tools: ["web_search"]
```

**Isolated Data Per Agent:**
- Session DB: `~/.obenmatrix/agents/<name>/sessions.db`
- Memories: `~/.obenmatrix/agents/<name>/memories/`
- Goals: `~/.oben-goals/<name>/`
- Usage stats: `~/.agents/<name>-usage_tracking.yaml`

```yaml
model:
  kind: OpenRouter
  model: "qwen/qwen3-235b:free"
  api_key: "sk-or-..."

context:
  max_messages: 100
  compression: summary
```

Edit config manually or run `oben setup` to use the wizard.

---

## Skills

ObenAgent ships with **25 built-in skill categories** covering development, analysis, automation, and more. Skills are defined as YAML, TXT, or MD files — you can also drop your own into the `skills/` directory.

*(Skill system under active development)*

---

## Session Management

Sessions store conversation history and persist to disk as JSON. ObenAgent supports **LLM-based session compaction** to keep context windows manageable:

```bash
# List all sessions
obenmatrix sessions list

# Compact a session (summarizes older messages via LLM)
obenmatrix sessions compact -s my-session

# Compact with a focus topic
obenmatrix sessions compact -s my-session -f "database migration"

# Delete a session
obenmatrix sessions delete -s my-session
```

---

## Goals (Autonomous Mode)

Run the agent autonomously on a goal — it plans, acts via tools, and iterates:

```bash
obenmatrix agent
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

**819 tests** pass across all 11 crates.

---

## Project Status

See [docs/PRD.md](docs/PRD.md) for detailed milestones, progress tracking, and implementation details.

---

## License

This project is licensed under the [MIT License](LICENSE).

---

## 🏗️ Multi-Agent System Architecture

ObenAgent's multi-agent system is built on several key components:

### Agent Registry (`oben-gateway`)
Manages multiple agent instances with:
- `insert(name, agent)` - Register new agents
- `get(name)` - Retrieve agents by name
- `lookup_by_role(role)` - Find agents by role

### Topic-Based Messaging (`oben-gateway::messaging`)
Publish-subscribe communication pattern:
- `publish(topic, message)` - Send to all subscribers
- `subscribe(topic)` - Receive messages from a topic
- `broadcast(message)` - Send to all subscribers across topics

### Data Isolation
Each agent has independent storage:
- Sessions: `~/.obenmatrix/agents/<name>/sessions.db`
- Memories: `~/.obenmatrix/agents/<name>/memories/`
- Goals: `~/.oben-goals/<name>/`
- Usage: `~/.agents/<name>-usage_tracking.yaml</name>`

### Usage Example
```yaml
# config.yaml
agents:
  - name: "manager"
    role: "Orchestrates tasks and coordinates workers"
    model: "openai/gpt-4o"
  - name: "worker"
    role: "Handles specific subtasks"
    model: "anthropic/claude-3-5-sonnet"
```

```bash
# Start manager agent
obenmatrix tui --agent manager

# Start worker agent (in another terminal)
obenmatrix tui --agent worker
```

Agents communicate via messaging topics while keeping their data completely isolated.


