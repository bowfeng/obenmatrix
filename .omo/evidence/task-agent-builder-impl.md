# AgentBuilder Implementation Evidence

## Task Summary

Implemented `AgentBuilder` with builder-style methods that wraps `Agent::new()` constructor, providing better error context and optional shared `HookEngine` support.

## Changes Made

### 1. Created `oben-agent/src/agent_builder.rs`
- `pub struct AgentBuilder` with fields: `config`, `system_prompt`, `tools`, `hooks` (all `Option<T>`)
- Builder methods:
  - `new()` — creates empty builder with all fields `None`
  - `with_config(AppConfig)` — sets config (required)
  - `with_system_prompt(String)` — sets system prompt (required)
  - `with_tools(Arc<ToolRegistry>)` — sets tools (required)
  - `with_hooks(Arc<HookEngine>)` — sets optional shared hooks
  - `build()` — async, returns `anyhow::Result<Agent>` with error context wrapping

### 2. Modified `oben-agent/src/lib.rs`
- Added `pub mod agent_builder;` declaration
- Added `pub use agent_builder::AgentBuilder;` re-export

### 3. Modified `oben-agent/src/agent.rs`
- `Agent::new()` now delegates to `AgentBuilder::new().with_config(...).with_system_prompt(...).with_tools(...).build().await`
- Made `Agent` struct fields `pub(crate)` for cross-module access
- Made `eager_load_active_session()` `pub(crate)` for agent_builder access
- Added `use crate::agent_builder::AgentBuilder;` import
- Added `use crate::hooks::HookEngine;` import

### 4. Modified `oben-tui/src/shared/agent_state.rs`
- Replaced direct `Agent::new()` call with `AgentBuilder::new()...build()` pattern
- Added `AgentBuilder` to the `use oben_agent::{...}` import

## Error Context Wrapping

Transport initialization failures are wrapped with:
```
"connection failed — check your model config (endpoint, api_key, model)"
```

Session store failures are wrapped with:
```
"failed to initialize session store"
```

## Verification

### Compilation
```
cargo check -p oben-agent    # PASS
cargo check -p oben-tui      # PASS
```

### Tests
```
cargo test --package oben-agent --lib   # 171 PASSED
```

### Acceptance Criteria
| # | Criterion | Status |
|---|-----------|--------|
| 1 | `oben-agent/src/agent_builder.rs` exists with `pub struct AgentBuilder` | ✅ |
| 2 | `AgentBuilder` has methods `new()`, `with_config()`, `with_system_prompt()`, `with_tools()`, `with_hooks()`, `build()` | ✅ |
| 3 | `build()` returns `Result<Agent>` with `anyhow::Context` wrapping | ✅ |
| 4 | TUI `SharedAgentState::init()` uses `AgentBuilder` | ✅ |
| 5 | `Agent::new()` delegates to `AgentBuilder` | ✅ |
| 6 | `pub mod agent_builder;` in `lib.rs` | ✅ |
| 7 | `cargo check -p oben-agent` passes | ✅ |
| 8 | `cargo check -p oben-tui` passes | ✅ |
