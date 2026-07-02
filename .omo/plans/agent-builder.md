# agent-builder - Work Plan

## TL;DR (For humans)

**What you'll get:** All `Agent::new()` calls replaced with a typed `AgentBuilder` pattern. Child/subagent agents now **share the parent's HookEngine** through `Arc<HookEngine>`, so TUI hooks, nudge hooks, and other lifecycle callbacks fire across the entire subagent turn tree — not just the parent.

**Why this approach:** Builder pattern centralizes Agent construction logic, adds error context wrapping for transport failures, and provides explicit `.with_hooks()` for Arc sharing. Subagents used to get fresh empty HookEngines — now they get the parent's shared engine.

**What it will NOT do:** No WASM plugin loading (that's `wasm-platform-plugins` plan). No new TUI UI changes. No changes to existing Hook traits or hook adapters. No breaking changes to existing Agent lifecycle methods.

**Effort:** Medium
**Risk:** Low — additive-only changes (new builder struct), existing `Agent::new()` preserved as thin delegate, all 171 tests pass.
**Decisions to sanity-check:** None — all decisions made based on existing code patterns (SkillBuilder/ToolBuilder/CredentialPoolBuilder already use the same builder pattern here).

## Completed ✅

All tasks executed and verified:

| Wave | Task | Status | Files |
|------|------|--------|-------|
| 1 | Implement AgentBuilder struct | ✅ | `oben-agent/src/agent_builder.rs`, `lib.rs`, `agent.rs` |
| 2 | TUI integration | ✅ | `oben-tui/src/shared/agent_state.rs` (creates shared HookEngine before spawner + agent, passes it to both) |
| 3 | CLI integration | ✅ | `oben-cli/src/dispatch.rs` (3 sites: run_chat, run_one_shot, goal_start) |
| 4 | Gateway integration | ✅ | `oben-gateway/src/dispatcher.rs` |
| 5 | Subagent hook sharing | ✅ | `oben-agent/src/delegate.rs` — **critical fix**: SubagentSpawner now holds `Arc<HookEngine>`, child agents use `.with_hooks(shared_hooks)` |
| 6 | Final polish & verification | ✅ | TUI lib.rs comment updated, `cargo check --workspace` clean, 171/171 tests pass |

## Verification

- `cargo check --workspace` — **compiles clean, 0 warnings**
- `cargo test --package oben-agent --lib` — **171/171 passed**
- Remaining `Agent::new()` calls in product code: **0** (only 1 doc comment + 1 code comment)

## Remaining

Nothing — all Agent construction now goes through AgentBuilder.
