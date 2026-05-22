# AIAgent Design — Hermes Agent Feature Parity

**Date**: 2026-05-22
**Reference**: [Hermes Agent `run_agent.py::AIAgent`](/Users/ellie/workspace/hermes-agent/run_agent.py) + [`agent/agent_init.py`](/Users/ellie/workspace/hermes-agent/agent/agent_init.py) + [`agent/conversation_loop.py`](/Users/ellie/workspace/hermes-agent/agent/conversation_loop.py)
**Current Rust**: `oben-agent/src/agent.rs` + `turn_executor.rs` + `conversation.rs`

---

## 1. AIAgent Conceptual Model (Hermes)

The Hermes `AIAgent` is a **stateful runtime actor** that owns everything — LLM client, tools, context engine, memory, session, interrupt handling, streaming, callbacks, fallback models, and resource cleanup. It exposes two public entry points:

- **`run_conversation(user_message, ...)`** — Execute one full turn (loop: LLM call → tool dispatch → repeat until no more tool calls). Returns a `Dict` with `final_response`, `messages`, `api_calls`, `completed`.
- **`chat(message, stream_callback)`** — Thin wrapper around `run_conversation` returning just the final text.
- **`interrupt(message)`** — Signal from another thread to break the tool loop.
- **`steer(text)`** — Inject text into the next tool result without interrupting.
- **`close()` / `release_clients()`** — Resource teardown / cache eviction.

### Initialization (70+ parameters in `__init__` / `init_agent`)

| Category | Parameters | Purpose |
|----------|-----------|---------|
| **Transport** | `base_url`, `api_key`, `provider`, `api_mode`, `model` | LLM connection |
| **Runtime** | `max_iterations`, `tool_delay`, `fallback_model`, `credential_pool` | Loop control, retries |
| **Callbacks** | `tool_progress_callback`, `tool_start_callback`, `tool_complete_callback`, `thinking_callback`, `reasoning_callback`, `clarify_callback`, `step_callback`, `stream_delta_callback`, `interim_assistant_callback`, `tool_gen_callback`, `status_callback` | Platform integration hooks |
| **Display** | `verbose_logging`, `quiet_mode`, `log_prefix_chars`, `log_prefix`, `_print_fn` | Output control |
| **Session** | `session_id`, `session_db`, `parent_session_id` | Session persistence |
| **Model config** | `max_tokens`, `reasoning_config`, `service_tier`, `request_overrides`, `prefill_messages` | LLM response shaping |
| **Platform** | `platform`, `user_id`, `user_name`, `chat_id`, `chat_name`, `chat_type`, `thread_id`, `gateway_session_key` | Multi-platform identity |
| **Features** | `save_trajectories`, `ephemeral_system_prompt`, `skip_memory`, `load_soul_identity`, `skip_context_files` | Feature toggles |
| **Memory** | `iteration_budget`, `checkpoints_enabled`, `checkpoint_max_*` | Budget + checkpointing |
| **Tools** | `enabled_toolsets`, `disabled_toolsets` | Tool filtering |
| **Routing** | `providers_allowed`, `providers_ignored`, `providers_order`, `provider_sort`, `openrouter_min_coding_score` | Provider selection |
| **Misc** | `acp_command`, `acp_args`, `command`, `args` | ACP integration |

---

## 2. Core Loop (run_conversation / conversation_loop)

```
loop:
  1. Restore primary runtime if fallback was activated
  2. Build/restore system prompt (prefix-cache aware)
  3. Inject memory/skill context blocks
  4. Sanitize messages (surrogates, non-ASCII, thinking-only)
  5. Drop thinking-only messages, merge consecutive user messages
  6. Apply Anthropic cache control headers
  7. Call LLM API (with retry/backoff, rate limit tracking)
  8. Parse response → text + tool_calls
  9. Stream deltas → callbacks (text, thinking, reasoning)
  10. If tool_calls: dispatch each (serial or concurrent)
  11. Apply pending steer to last tool result
  12. Check iteration budget (injected warning at max)
  13. Check compression model feasibility
  14. If interrupted: break and clean up
  15. If no tool_calls: done → return
  16. Persist messages to session DB / JSON log
  17. Background memory/skill review nudge (if threshold reached)
```

Key behaviors:
- **Concurrent tool dispatch** for independent tools (max 8 workers)
- **Automatic retries** with jittered backoff (configurable max retries)
- **Fallback activation** on exhaustion/error
- **Context compression** when threshold reached
- **Turn budget** — warns at 80% and 100%, forces final call

---

## 3. Comparison: Hermes AIAgent vs Our Rust `Agent`

### 3.1 Feature Parity Matrix

| Feature | Hermes AIAgent | Our Rust Agent | Status |
|---------|---------------|----------------|--------|
| **Basic turn cycle** (LLM → tools → repeat) | ✅ | ✅ | ✅ Done |
| **Session management** (create/switch/save/persist) | ✅ (SQLite + JSON) | ✅ (SQLite + JSON) | ✅ Done |
| **Context compression** (auto when threshold hit) | ✅ | ✅ | ✅ Done |
| **Streaming** (token deltas, thinking blocks) | ✅ | ✅ (basic) | ⚠️ Partial |
| **Tool dispatch** (serial execution) | ✅ (serial + concurrent) | ✅ (serial only) | ⚠️ Missing concurrent |
| **Interrupt handling** (cross-thread signal) | ✅ | ❌ | ❌ Missing |
| **Steer mechanism** (inject without interrupt) | ✅ | ❌ | ❌ Missing |
| **Callback system** (11+ callbacks) | ✅ | ✅ (ChatCallbacks only) | ⚠️ Limited |
| **Platform support** (CLI, Telegram, Discord, etc.) | ✅ | ❌ | ❌ Missing |
| **Memory system** (external memory provider plugin) | ✅ | ❌ | ❌ Missing |
| **Fallback models** (chain of backup providers) | ✅ | ❌ | ❌ Missing |
| **Retry with backoff** (API error recovery) | ✅ (jittered exponential) | ❌ | ❌ Missing |
| **Iteration budget** (shared across subagents) | ✅ | ✅ (budget module exists) | ⚠️ Not wired |
| **Trajectory saving** (JSONL conversation logs) | ✅ | ❌ | ❌ Missing |
| **System prompt caching** (prefix cache via session DB) | ✅ | ❌ | ❌ Missing |
| **Tool guardrails** (validation/halting loop) | ✅ | ❌ | ❌ Missing |
| **Context engine tools** (lcm_grep, etc.) | ✅ | ❌ | ❌ Missing |
| **Skill management** (create/improve from experience) | ✅ | ⚠️ (loader exists) | ⚠️ Partial |
| **Checkpoint/rollback** (filesystem snapshots) | ✅ | ❌ | ❌ Missing |
| **Rate limit tracking** | ✅ | ❌ | ❌ Missing |
| **Activity tracking** (timeout, still-working notifications) | ✅ | ❌ | ❌ Missing |
| **Resource cleanup** (VM, browser, subprocess, clients) | ✅ | ❌ | ❌ Missing |
| **Background memory review** | ✅ | ❌ | ❌ Missing |
| **Subagent delegation** (concurrent child agents) | ✅ | ❌ | ❌ Missing |
| **Error classification & recovery** | ✅ | ❌ | ❌ Missing |
| **Provider routing** (auto-detect, OAuth, custom headers) | ✅ | ✅ (basic) | ⚠️ Basic |

### 3.2 What We Have (Already Implemented)

Our Rust `Agent` already handles the **core happy path**:

1. **Resource ownership model** — Agent owns Transport, Tools, Skills, ContextEngine, SessionManager
2. **Turn execution** — Full loop: LLM call → tool dispatch → repeat
3. **Session management** — Create, switch, persist, lazy-init
4. **Context compression** — `ContextEngine` trait with auto-compaction
5. **Streaming** — Basic delta callback support
6. **Tool dispatch** — Through `ToolRegistry`
7. **Interactive chat loop** — Via `ConversationLoop::run_loop`
8. **Call mode** — Fresh → Incremental management

### 3.3 What's Missing (Gap Analysis)

#### Tier 1: Core Runtime Features (needed for production reliability)

| Feature | Description | Complexity |
|---------|-------------|------------|
| **Retry with backoff** | Jittered exponential retry on API failures, classifying errors (rate limit vs auth vs other) | Medium |
| **Fallback models** | Chain of backup providers when primary exhausts/overloads | Medium |
| **Interrupt mechanism** | Cross-thread atomic flag + signal to running tool workers | Medium |
| **Iterative budget enforcement** | Warn at 80%, 90%; force final call at 100% | Low |
| **Error classification** | Categorize API errors (rate limit, auth, model not found, etc.) | Medium |
| **Message sanitization** | Handle surrogates, non-ASCII, thinking-only messages, merge consecutive user messages | Medium |

#### Tier 2: Advanced Runtime Features

| Feature | Description | Complexity |
|---------|-------------|------------|
| **Concurrent tool dispatch** | Run independent tools in parallel (max 8 workers) | Medium |
| **Stream delta processing** | Handle thinking blocks, memory context scrubbing in streaming | Medium |
| **System prompt prefix caching** | Restore cached prompt from session DB, update atomically | Medium |
| **Callback system** | 11+ callback types for platform integration (tool progress, thinking, reasoning, clarify, status, etc.) | Medium |
| **Tool guardrails** | Validate tool calls, halt loop on safety violations | Medium |
| **Activity tracking** | Track last activity timestamp for timeout/alive detection | Low |

#### Tier 3: Extended Capabilities

| Feature | Description | Complexity |
|---------|-------------|------------|
| **Memory system** | External memory provider plugin (SQLite + provider abstraction) | High |
| **Trajectory saving** | JSONL conversation logs with reasoning conversion | Low |
| **Context engine tools** | Auto-inject tool schemas from context engine (lcm_grep, etc.) | Low |
| **Checkpoint/rollback** | Filesystem snapshots of conversation state | Medium |
| **Background memory review** | Daemon thread that reviews memory/skills after N turns | Medium |
| **Subagent delegation** | Child agent creation with interrupt propagation | High |
| **Resource cleanup** | Subprocess, VM, browser, client lifecycle | Medium |
| **Provider routing** | OAuth, custom headers, per-model routing, credential pools | High |
| **Platform adapters** | Telegram, Discord, WhatsApp, Slack integration | High |

---

## 4. Proposed Design: Enhanced `AIAgent`

### 4.1 Architecture

```
AIAgent (struct)
├── Transport             (Arc<ChatCompletionsTransport>)    — existing
├── ToolRegistry          (Arc<ToolRegistry>)                — existing
├── ContextEngine         (Box<dyn ContextEngine>)           — existing
├── SessionManager        (SessionManager)                   — existing
├── SystemPromptBuilder   (SystemPrompt)                     — existing
├── Callbacks             (AgentCallbacks)                   — NEW: richer callback set
├── RetryPolicy           (RetryConfig)                      — NEW
├── FallbackChain         (Vec<FallbackConfig>)              — NEW
├── IterationBudget       (Arc<Budget>)                      — NEW: shared across subagents
├── InterruptManager      (InterruptState)                   — NEW: thread-safe
├── ActivityTracker       (ActivityState)                    — NEW
├── TrajectoryWriter      (Option<TrajectoryPath>)           — NEW
├── ToolGuardrails        (ToolGuardrailConfig)              — NEW
└── MemoryManager         (Option<MemoryManager>)            — FUTURE
```

### 4.2 New Public API

```rust
// Existing — keep as-is
impl Agent {
    pub fn new(config: AgentConfig) -> Result<Self> { ... }
    pub async fn turn(&mut self, ...) -> Result<String> { ... }
    pub async fn interactive_chat(&mut self, ...) -> Result<()> { ... }
    pub fn continue_session(&mut self, key: &str) -> Result<String> { ... }
    pub fn reset(&mut self) -> Result<()> { ... }
}

// New — production runtime features
impl Agent {
    // Interrupt handling
    pub fn interrupt(&self, message: Option<&str>) { ... }
    pub fn clear_interrupt(&self) { ... }
    pub fn steer(&self, text: &str) -> bool { ... }  // inject without interrupt

    // Callbacks
    pub fn set_callbacks(&mut self, callbacks: AgentCallbacks) { ... }

    // Fallback models
    pub fn set_fallback_chain(&mut self, chain: Vec<FallbackConfig>) { ... }

    // Retry policy
    pub fn set_retry_policy(&mut self, policy: RetryConfig) { ... }

    // Iteration budget
    pub fn set_iteration_budget(&mut self, budget: Arc<Budget>) { ... }

    // Trajectory saving
    pub fn set_trajectory_path(&mut self, path: Option<PathBuf>) { ... }

    // Resource cleanup (idempotent)
    pub fn close(&mut self) { ... }

    // Session DB flush
    pub fn flush_session(&self) -> Result<()> { ... }

    // Activity status
    pub fn get_activity_summary(&self) -> ActivitySummary { ... }
}
```

### 4.3 New Structs

```rust
/// Rich callback set for platform integration.
/// Mirrors Hermes' 11+ callback parameters.
pub struct AgentCallbacks {
    /// Tool execution progress (tool_name, args_preview)
    pub tool_progress: Option<Box<dyn Fn(&str, &str) + Send + Sync>>,
    /// Tool started
    pub tool_start: Option<Box<dyn Fn(&str, &str) + Send + Sync>>,
    /// Tool completed with result
    pub tool_complete: Option<Box<dyn Fn(&str, &str, &str) + Send + Sync>>,
    /// Thinking/thought stream delta
    pub thinking: Option<Box<dyn Fn(&str) + Send + Sync>>,
    /// Reasoning stream delta
    pub reasoning: Option<Box<dyn Fn(&str) + Send + Sync>>,
    /// Clarification request (question, choices) -> user answer
    pub clarify: Option<Box<dyn Fn(&str, &[&str]) -> String + Send + Sync>>,
    /// Step-by-step status
    pub step: Option<Box<dyn Fn(&str) + Send + Sync>>,
    /// Token stream delta (for TTS etc.)
    pub stream_delta: Option<Box<dyn Fn(&str) + Send + Sync>>,
    /// Interim assistant message (non-streaming)
    pub interim_assistant: Option<Box<dyn Fn(&str) + Send + Sync>>,
    /// Tool generation event
    pub tool_gen: Option<Box<dyn Fn(&str) + Send + Sync>>,
    /// Lifecycle status (lifecycle, model, provider, etc.)
    pub status: Option<Box<dyn Fn(&str, &str) + Send + Sync>>,
    /// Verbose print (always visible, even during streaming)
    pub vprint: Option<Box<dyn Fn(&str) + Send + Sync>>,
}

/// Retry configuration with exponential backoff + jitter.
#[derive(Clone, Debug)]
pub struct RetryConfig {
    pub max_retries: u32,           // default: 3
    pub base_delay_ms: u64,         // default: 500
    pub max_delay_ms: u64,          // default: 60000
    pub jitter_factor: f64,         // default: 0.5
    pub retryable_codes: Vec<u16>,  // HTTP codes to retry on
}

/// Single fallback model in the chain.
#[derive(Clone, Debug)]
pub struct FallbackConfig {
    pub provider: String,
    pub model: String,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
}

/// Per-agent interrupt state — shared via Arc<Mutex<>>.
#[derive(Default)]
struct InterruptState {
    requested: AtomicBool,
    message: Mutex<Option<String>>,
    pending_steer: Mutex<Option<String>>,
}

/// Activity tracking for timeout/alive detection.
#[derive(Clone, Debug)]
pub struct ActivitySummary {
    pub last_activity: SystemTime,
    pub last_activity_desc: String,
    pub current_tool: Option<String>,
    pub api_call_count: u32,
    pub iteration_budget_used: u32,
}
```

### 4.4 TurnExecutor Enhancements

The `TurnExecutor::execute_turn` needs these additions:

```rust
impl TurnExecutor {
    // The existing method, with these additions:
    // 1. Retry loop wrapping the API call (with backoff)
    // 2. Fallback model activation on failure
    // 3. Interrupt checking between iterations
    // 4. Iteration budget enforcement
    // 5. Tool guardrail validation
    // 6. Concurrent tool dispatch
    // 7. Callback dispatch (all 12 callback types)
    // 8. Message sanitization pre-API call
    // 9. Trajectory writing post-turn
    // 10. Session DB flushing
}
```

---

## 5. Implementation Plan

### Phase 1: Core Reliability (Tier 1)
1. **Retry with backoff** — Wrap API calls in retry loop with jittered exponential backoff
2. **Error classification** — Categorize errors (rate limit, auth, model not found, network)
3. **Iteration budget** — Enforce max iterations, warn at thresholds
4. **Interrupt mechanism** — Cross-thread atomic interrupt flag
5. **Message sanitization** — Handle surrogates, thinking-only, duplicate user messages

### Phase 2: Advanced Runtime (Tier 2)
6. **Fallback models** — Chain of backup providers
7. **Concurrent tool dispatch** — ThreadPoolExecutor for independent tools
8. **Stream delta processing** — Thinking block scrubbing, memory context filtering
9. **Callback system** — All 12+ callback types
10. **System prompt prefix caching** — DB-backed prompt restore
11. **Activity tracking** — Timestamp-based timeout support

### Phase 3: Extended Features (Tier 3, Future)
12. **Memory system** — External memory provider plugin
13. **Trajectory saving** — JSONL conversation logs
14. **Tool guardrails** — Safety validation
15. **Checkpoint/rollback** — Filesystem snapshots
16. **Background memory review** — Daemon thread
17. **Subagent delegation** — Child agent creation
18. **Resource cleanup** — Subprocess/VM/browser lifecycle
19. **Provider routing** — OAuth, custom headers, credential pools
20. **Platform adapters** — Telegram/Discord/WhatsApp

---

## 6. Key Design Decisions

### 6.1 Why not just parallel the Python code 1:1?

Hermes' `AIAgent` is ~4,000 lines with ~1,400 lines in `__init__` alone. The Python code uses:
- Dynamic attribute setting (anything can be a field)
- Monkey-patching (tests patch `run_agent.OpenAI`)
- Global mutable state (`_tool_worker_threads`, `_openrouter_prewarm_done`)
- Thread-local ContextVars for session scoping

Rust's type system and ownership model prevent these patterns. Instead:
- Explicit struct fields with proper types
- Arc<Mutex<>> for shared mutable state
- Traits for extensibility (no monkey-patching)
- Async channels for callbacks
- Proper error types (no exceptions)

### 6.2 Callback Strategy

Hermes passes 11 separate callback parameters into `__init__` and stores them as `self.*_callback`. In Rust, we use a single `AgentCallbacks` struct with `Option<Box<dyn Fn(...) + Send + Sync>>` fields. This is:
- **Type-safe** — each callback has a concrete signature
- **Zero-cost** — `Option` fields are elided via fat pointers when None
- **Thread-safe** — `Send + Sync` bound
- **Composable** — easy to pass around, clone, or replace

### 6.3 Interrupt Strategy

Hermes uses:
- `threading.Lock()` for interrupt flag
- `_set_interrupt(True, thread_id)` for per-thread tool signals
- `_tool_worker_threads` set for concurrent tool tracking
- Recursive interrupt propagation to child agents

Rust equivalent:
- `AtomicBool` for interrupt flag (no lock needed)
- `Arc<InterruptState>` shared between Agent and TurnExecutor
- `tokio::sync::mpsc::channel` for steer messages
- `Arc<InterruptState>` passed to child agents

---

## 7. Summary

**Current state**: We have a solid core (turn cycle, sessions, compression, basic streaming, tool dispatch). This is enough for a functional agent.

**What's needed for production parity**: The Tier 1 features (retry, fallback, interrupt, budget, sanitization) are essential for reliability. Tier 2 adds resilience and platform integration. Tier 3 adds the full Hermes feature set.

**Recommendation**: Implement Phase 1 first — it covers the gap between "demo agent" and "production agent" with the most impact per code effort.
