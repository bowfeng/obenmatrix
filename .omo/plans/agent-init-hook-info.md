# agent-init-hook-info - Work Plan

## TL;DR (For humans)

**What you'll get:** Two cleanups — strip skill logic from the nudge review system, and replace the broken `set_memory_tools` mutation with a proper `on_agent_init` hook that injects tool names at startup.

**Why this approach:** Currently `has_memory_tools` is set to `false` at construction and never flipped, so nudge is dead code. The right fix is an `AgentInit` trait that gets called after the tool registry is populated. Removing skill dep simplifies the system and matches reality (skills aren't loaded in production yet).

**What it will NOT do:** Add SkillCatalog to production paths. Introduce `AgentRuntime`. Break any existing API — all changes are additive (new trait with default impl) or internal (nudge config).

**Effort:** Quick
**Risk:** Low — additive `AgentInit` trait has default impl, no breaking changes. Nudge changes are internal-only.
**Decisions to sanity-check:** `should_trigger_nudge` drops `has_memory_tools` param (moved to NudgeHook internal state via `on_agent_init`). `skill_nudge_interval` is stripped entirely from `NudgeConfig`.

Your next move: approve the plan.

---

> TL;DR (machine): <1 line - effort, risk, deliverables>

## Scope
### Must have
1. `AgentInit` trait with default empty impl in `Hook` trait hierarchy
2. `HookInitInfo` struct with `HashSet<String>` tool/skill name fields
3. All 7 hook type traits (`AgentLoopHooks`, `TurnLifecycleHooks`, etc.) inherit `AgentInit`
4. `HookEngine::init_hooks()` broadcasts to all registered hooks
5. `NudgeHook` implements `on_agent_init` to set `has_memory_tools` from `HookInitInfo`
6. Remove `skill_nudge_interval`, `skill_enabled()`, all skill branches from nudge system
7. `should_trigger_nudge` drops `has_memory_tools` param
8. `SharedAgentState::init` calls `hook_engine.init_hooks(&info)` after hook engine build
9. `trigger_nudge` in agent.rs drops skill params

### Must NOT have
- Introduce `AgentRuntime` struct
- Add SkillCatalog to production code paths
- Change breaking APIs — `AgentInit` has default impl on Hook
- Touch WASM hooks or other platforms
- Add skill name loading (skills not loaded in production anyway)

## Verification strategy
> Zero human intervention - all verification is agent-executed.
- Test decision: tests-after + run existing tests
- Evidence: compile output, test output, grep checks for removed symbols
- `cargo test -p oben-agent` must pass

## Execution strategy
### Parallel execution waves
> Wave 1: Core trait additions (kind.rs) — no deps
> Wave 2: Nudge cleanup (nudge.rs) — depends on wave 1 for trait structure understanding
> Wave 3: NudgeHook runtime + HookEngine init (runtime.rs) — depends on wave 2
> Wave 4: HookEngine + HookBuilder wiring + agent.rs + agent_state.rs — depends on wave 3

### Dependency matrix
| Todo | Depends on | Blocks | Can parallelize with |
| --- | --- | --- | --- |
| 1. AgentInit trait + HookInitInfo | none | all | none (foundation) |
| 2. Strip skill from NudgeConfig | none | 3,4 | can start while 1 runs |
| 3. NudgeHook on_agent_init | 1,2 | 4 | depends on both 1,2 |
| 4. HookEngine init + HookBuilder | 1,2 | 5 | depends on both 1,2 |
| 5. Wire init_hooks into agent_state.rs | 4 | none | last |

## Todos
> Implementation + Test = ONE todo. Never separate.
<!-- APPEND TASK BATCHES BELOW THIS LINE WITH edit/apply_patch - never rewrite the headers above. -->
- [ ] 1. Add `AgentInit` trait and `HookInitInfo` to `hooks/kind.rs`
  What to do / Must NOT do: Add `HookInitInfo { tool_names: HashSet<String> }` struct. Add `fn on_agent_init(&self, info: &HookInitInfo)` to `Hook` trait with default empty body. All 7 hook type traits extend `Hook + AgentInit`. Keep trait minimal — no extra methods on HookInitInfo for now.
  References: `oben-agent/src/hooks/kind.rs:241-319`
  Acceptance criteria: `cargo check -p oben-agent` passes (additive — default impl means no breaking changes)
  QA: `cargo check -p oben-agent` — expect zero errors
  Commit: Y | feat(hooks): add AgentInit trait and HookInitInfo for runtime injection to hooks
- [ ] 2. Strip skill logic from `NudgeConfig` and `nudge.rs` free functions
  What to do / Must NOT do: Remove `skill_nudge_interval` field from `NudgeConfig`. Remove `skill_enabled()` method. `enabled()` becomes just `memory_nudge_interval > 0`. Remove `has_memory_tools` param from `should_trigger_nudge` — keep only `(turns_since_nudge, interval, is_resumed_session)`. Simplify `build_nudge_prompt(memory_enabled)` to single memory-review-only prompt (remove 4-branch match). Update ALL tests accordingly. Delete `from_config_internal` on NudgeHook (dead code).
  References: `oben-agent/src/nudge.rs:14-56` (config), `oben-agent/src/nudge.rs:73-136` (functions), `oben-agent/src/nudge.rs:140-215` (tests)
  Acceptance criteria: `cargo test -p oben-agent -- nudge` passes; grep `skill_nudge_interval|skill_enabled` in nudge.rs returns zero results
  QA: `cargo test -p oben-agent nudge` — all tests pass with simplified prompts
  Commit: Y | refactor(nudge): remove skill interval from NudgeConfig and skill branches from prompts
- [ ] 3. Rewrite `NudgeHook` to use `on_agent_init` and remove all dead code
  What to do / Must NOT do: Remove `has_memory_tools` field. Remove `set_memory_tools()`, `set_turn_count()`, `set_sub_turn_callback()`, `sub_turn_callback` field, `collect_callbacks()`, `turn_count: AtomicUsize` field. Implement `AgentInit::on_agent_init` to set `has_memory_tools` from `info.tool_names.contains("memory")`. `on_post_turn` uses `turn_count` parameter (not self-atomic counter). Simplify to just the threshold check + callback invocation pattern (callback stays but is None for now; hook engine wires it later).
  References: `oben-agent/src/hooks/runtime.rs:13-84`
  Acceptance criteria: `cargo check -p oben-agent` passes; zero references to `set_memory_tools|set_turn_count|set_sub_turn_callback|collect_callbacks` remain
  QA: `cargo check -p oben-agent`
  Commit: Y | refactor(nudge-hook): replace mutation-based init with on_agent_init, remove dead code
- [ ] 4. Wire `hook_engine.init_hooks()` in `HookEngine` + update `NudgeHook` on_post_turn to trigger hook engine run
  What to do / Must NOT do: Add `pub fn init_hooks(&self, info: &HookInitInfo)` to `HookEngine` that iterates all 7 hook collections calling `hook.on_agent_init(info)`. Add `pub fn init_hooks(&self, info: &HookInitInfo)` to `HookEngine` that iterates all 7 hook collections calling `hook.on_agent_init(info)`. In `NudgeHook` on_post_turn: after threshold check and prompt build, call `hook_engine.run_hook_turn()` to execute nudge sub-turn (this wires the nudge into full turn execution).
  References: `oben-agent/src/hooks/runtime.rs:90-123, 258-301` (HookEngine methods)
  Acceptance criteria: HookEngine has init_hooks available
  QA: `cargo check -p oben-agent`
  Commit: Y | feat(hooks): add HookEngine::init_hooks to broadcast agent init; NudgeHook triggers via hook engine
- [ ] 5. Wire `init_hooks` into all callers: SharedAgentState, CLI, gateway
  What to do / Must NOT do: In `SharedAgentState::init` (TUI), after building hook engine, create `HookInitInfo` and call `hooks.init_hooks(&info)`. In CLI dispatch.rs: two call sites need init_hooks. In gateway main.rs: init_hooks after HookBuilder build. In CLI dispatch.rs: remove skill_interval param from trigger_nudge calls. In `agent.rs` trigger_nudge: remove skill_interval param, remove skill_enabled logic, simplify build_nudge_prompt to memory-only version.
  References: `oben-tui/src/shared/agent_state.rs:136-138`, `oben-cli/src/dispatch.rs:127,195`, `oben-gateway/src/main.rs:152-162`, `oben-agent/src/agent.rs:676-816`
  Acceptance criteria:
  - `cargo test -p oben-agent` passes
  - `cargo check -p oben-tui -p oben-cli -p oben-gateway` passes
  - No remaining references to `skill_nudge_interval|set_memory_tools|sub_turn_callback|set_turn_count|collect_callbacks` in any production code
  QA: Full workspace check: `cargo check -p oben-agent -p oben-tui -p oben-cli -p oben-gateway`
  Commit: Y | refactor: wire init_hooks across all entry points, remove skill from trigger_nudge

## Final verification wave
> Runs in parallel after ALL todos. ALL must APPROVE. Surface results and wait for the user's explicit okay before declaring complete.
- [ ] F1. Plan compliance audit
- [ ] F2. Code quality review
- [ ] F3. Real manual QA
- [ ] F4. Scope fidelity

## Commit strategy
Single commit: `feat(agent/hooks): remove skill dep from nudge, add on_agent_init lifecycle injection`

## Success criteria
- `cargo test -p oben-agent` passes with no warnings
- `cargo check -p oben-tui` and `cargo check -p oben-gateway` pass
- No references to `set_memory_tools`, `skill_nudge_interval`, `skill_enabled` remain in production code
- `AgentInit` trait is the only new public trait introduced on the hook system
- NudgeHook's `has_memory_tools` is correctly set at init time from the tool registry
