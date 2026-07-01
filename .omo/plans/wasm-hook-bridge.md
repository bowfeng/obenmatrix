# WASM Hook Bridge - Work Plan

## TL;DR (For humans)

**What you'll get:** Let WASM plugins register the same kinds of hooks that TUI and config-based adapters already use — intercepting tool calls, streaming output, turn lifecycle, session events, and system status. Plugins call `hook_on_pre_turn()`, `hook_on_tool_start()`, etc., and the agent broadcasts those events through the existing `HookEngine` dispatch.

**Why this approach:** The bridge pattern (WASM callbacks → `Box<dyn HookTrait>` → same `HookEngine` queue) reuses the entire existing dispatch path with zero changes to the broadcast loop. No new traits, no new dispatch code. Each hook trait kind is a small adapter that calls the WASM component's exported function.

**What it will NOT do:** No AgentLoop/InterruptLifecycle hooks (startup-only, not runtime-useful). No hot-reload. No WASM-side SDK (guest bindings). Real WASM instantiation is a stub until the full component model lands — the bridge proves the trait-wrapping path works now. Strings only across the boundary.

Effort: **Medium**
Risk: **Medium** — hook crash isolation is the main load-bearing safety property
Decisions to sanity-check: (1) 5 hook categories (skip AgentLoop/Interrupt). (2) Gateway wires hooks into HookBuilder before `Agent::new()`. (3) WASM-trap → abort hook, agent continues.

Your next move: approve, or run high-accuracy review. Full execution detail follows below.

---

> TL;DR (machine): medium effort, medium risk. 8 todos across 3 waves. WIT definition → bridge+engine integration → gateway wiring → integration test. Zero human intervention verification.

## Scope

### Must have
1. **WIT hook interface** (`oben-wasm/wit/hook.wit`) — guest exports for 5 hook categories (TurnLifecycle, ToolLifecycle, Streaming, SystemEvents, SessionLifecycle) with `id`, `priority`, and each hook method signature
2. **WasmHookBridge** (`oben-wasm/src/hook-bridge.rs`) — takes a `&wasmtime::Component` and `&wasmtime::Store`, calls exported WASM functions, wraps results into `Box<dyn X>` for each of the 5 HookEngine categories
3. **WasmHookRegistry** (`oben-wasm/src/hook-bridge.rs`) — manages WASM component pre-compilation, produces hook wrappers, tracks registered hook IDs
4. **HookEngine integration** (`oben-agent/src/hooks/runtime.rs`) — new `register_wasm_hooks(&self, hooks: impl IntoIterator<Item = Box<dyn Hook>>)` method that pushes into all 7 queues
5. **HookBuilder helper** (`oben-agent/src/hooks.rs`) — `with_wasm_hooks(&mut self, wasm_hooks: Vec<Box<dyn Hook>>)` that batches-injects
6. **Cargo.toml wiring** — `oben-wasm` adds `wasmtime` (already present) + optional `oben-agent` for trait references in tests
7. **Gateway integration** (`oben-gateway/src/main.rs`) — after WASM plugin loader, discover and register WASM hooks into the hook builder before agent construction
8. **Integration test** — mock WASM component that calls hook exports; verify HookEngine dispatches to registered wrappers

### Must NOT have (guardrails, anti-slop, scope boundaries)
- AgentLoopHooks (startup-only, not runtime-useful)
- InterruptLifecycleHooks (Ctrl+C, not plugin-useful)
- Hot-reload / live update of hooks
- WASM-side SDK / guest bindings (future deliverable)
- Real WASM instantiation in bridge.rs (stub/scaffold: just prove traits + error isolation work)
- Cross-plugin hook ordering beyond Rust insertion-order
- Binary data across WASM boundary (strings only)
- Hook priority override from plugin metadata (default `priority()` = 100)

## Verification strategy
> Zero human intervention - all verification is agent-executed.
- Test decision: **tests-after** (implement, then write failing test → verify passes) + unit tests on bridge logic
- Framework: Rust `#[test]` + `#[tokio::test]` for async bridge
- Evidence: `.omo/evidence/task-<N>-wasm-hook-bridge.md` per todo
- **Critical evidence**: `cargo test -p oben-agent --lib` + `cargo check` passes after all changes; no new compiler warnings in modified files

## Execution strategy

### Parallel execution waves
| Wave | Contents | Can parallelize? |
|------|----------|-----------------|
| 1 | WIT definition, WasmHookBridge struct + error types | No dependencies on any existing code |
| 2 | HookKind adapters (7 adapter impls wrapping WASM exports), WasmHookRegistry | Blocked by Wave 1 |
| 3 | HookEngine/HookBuilder integration, Cargo.toml changes | Blocked by Waves 1+2 |
| 4 | Gateway integration (main.rs), WASM plugin loader extension | Blocked by Wave 3 |
| 5 | Integration tests + unit tests | Blocked by Wave 3 (tests need the API) |
| 6 | Final verification | After all todos |

### Dependency matrix
| Todo | Depends on | Blocks | Can parallelize with |
|------|-----------|--------|---------------------|
| T1: WIT hook.wit | — | T2, T3 | — |
| T2: WasmHookBridge + error types | T1 | T3, T4 | — |
| T3: 5 HookKind adapters | T2 | — | T4 |
| T4: WasmHookRegistry | T2, T3 | T5 | — |
| T5: HookEngine/HookBuilder integration | T3, T4 | T6 | — |
| T6: Cargo.toml wiring | T5 | T7 | — |
| T7: Gateway integration | T6 | T8 | — |
| T8: Tests + evidence | T6 | — | — |
| F1-F4: Verification | T8 | — | all parallel |

## Todos

> Implementation + Test = ONE todo. Never separate.

- [x] 1. Define WIT hook interface — five guest export categories
  What to do / Must NOT do:
  - Create `oben-wasm/wit/hit.wit` with the WIT world definition
  - Define 5 exported interfaces: `turn`, `tool`, `streaming`, `system`, `session`
  - Each interface exports functions matching the Rust trait method signatures:
    - `turn`: `on-pre-turn`, `on-post-turn(response: string, success: bool)`
    - `tool`: `on-tool-gen(name: string, call-id: string)`, `on-tool-start(name: string, args: string)`, `on-tool-complete(name: string, args: string, result: string)`, `on-tool-error(name: string, args: string, error: string)`, `on-tool-progress(name: string, preview: string)`
    - `streaming`: `on-stream-delta(text: string)`, `on-thinking(text: string)`, `on-reasoning(text: string)`, `on-interim-assistant(text: string)`
    - `system`: `on-status(level: string, message: string)`
    - `session`: `on-session-rotate(parent-id: string, child-id: string)`, `on-compression-start(count: u32)`, `on-compression-complete(status: string)`
  - Export metadata: `plugin-id` (string), `plugin-priority` (u32, default 100)
  - Package: `package oben:wasm;`
  - Must NOT: define host functions (host calls WASM, not the reverse)
  - Must NOT: define AgentLoop or InterruptLifecycle in WIT

  Parallelization: Wave 1 | Blocked by: — | Blocks: T2, T3
  References: `oben-wasm/wit/platform.wit` (as reference for WIT style), `oben-wasm/src/lib.rs:34-43` (WIT world placeholder)
  Acceptance criteria: `wasm-tools print wit/hook.wit` succeeds; file exports exactly 5 category interfaces + 2 metadata exports
  QA: `wasm-tools lint wit/hook.wit` — happy: lint clean, Evidence .omo/evidence/task-1-wasm-hook-bridge.md
  Commit: Y | feat(wasm): define hook WIT interface for WASM plugin hook callbacks

- [x] 2. Create WasmHookBridge struct + WasmHost interface + error types
  What to do / Must NOT do:
  - In `oben-wasm/src/hook-bridge.rs` (new file), create `WasmHookError` enum: Compilation, Instantiation, Call, MissingExport, Unreachable
  - `type Result<T> = std::result::Result<T, WasmHookError>;`
  - Define `WasmHookBridge` struct holding: `Component`, `Store`, `Linker`, `StoreMut`
  - Define `WasmHost` trait with methods: `on_pre_turn()`, `on_post_turn()`, `on_tool_gen()`, `on_tool_start()`, `on_tool_complete()`, `on_tool_error()`, `on_tool_progress()`, `on_stream_delta()`, `on_thinking()`, `on_reasoning()`, `on_interim_assistant()`, `on_status()`, `on_session_rotate()`, `on_compression_start()`, `on_compression_complete()`, `on_interrupt_requested()`, `on_interrupted()`, `on_loop_start()`, `on_loop_end()`
  - Implement `WasmHookBridge::new(comp: Component) -> Result<Self>` — creates linker with host exports, instantiates
  - Implement `WasmHookBridge::store_mut()` → `&mut Store`
  - Implement `WasmHookBridge::try_from_store(store: &mut Store) -> Option<GuestExports>` — retrieves the guest exports handle
  - Implement `WasmHookBridge::try_call_<hook_name>(&self, store: &mut Store, args...) -> WasmResult<()>` — calls each exported function, catches wasmtime traps into `WasmHookError`
  - Each `try_call_*` is `#[inline(never)]` — the trap-to-error conversion is the critical safety path
  - Inside `try_call_*`: call `guest.exports.<hook_name>(store, args...)`, catch `Err(e)` → `WasmHookError::Call(format!("export '{}' trapped: {}", fn_name, e))`
  - Must NOT: implement full trait wrapping yet (that's T3/T4)
  - Must NOT: depend on `oben-agent` crate (only wasmtime types)

  Parallelization: Wave 1 | Blocked by: — | Blocks: T3
  References: `oben-wasm/src/host.rs` (scaffold linker), `oben-wasm/src/runtime.rs` (WasmRuntime, Component), `oben-wasm/src/bridge.rs` (existing pattern), `oben-wasm/src/error.rs` (WasmError enum)
  Acceptance criteria: File compiles standalone; struct exists with required methods; linker creation + instantiation works with a real compiled component
  QA: `cargo check -p oben-wasm` — happy: compiles with 0 warnings on new file. Evidence: .omo/evidence/task-2-wasm-hook-bridge.md
  Commit: Y | feat(wasm): add WasmHost trait + WasmHookBridge struct for calling WASM hook exports

- [x] 3. Implement 7 hook adapter wrappers — each trait impl calls through the bridge
  What to do / Must NOT do:
  - Create `oben-wasm/src/hook-bridge.rs` additions and `oben-wasm/src/wasm-hooks.rs` (new module for adapters)
  - Define **7 adapter structs**, each holding an `Arc<WasmHookBridge>`:
    - `WasmAgentLoopAdapter: AgentLoopHooks` — calls `bridge.try_call_on_loop_start(store)` / `try_call_on_loop_end(store, outcome)`
    - `WasmTurnLifecycleAdapter: TurnLifecycleHooks` — calls `try_call_on_pre_turn()` / `try_call_on_post_turn(response, success)`
    - `WasmToolLifecycleAdapter: ToolLifecycleHooks` — calls the 5 tool methods (gen, start, complete, error, progress)
    - `WasmStreamingAdapter: StreamingHooks` — calls 4 streaming methods (delta, thinking, reasoning, interim)
    - `WasmSystemEventsAdapter: SystemEventsHooks` — calls `on_status(level, message)`
    - `WasmSessionLifecycleAdapter: SessionLifecycleHooks` — calls 3 session methods (rotate, compress-start, compress-complete)
    - `WasmInterruptLifecycleAdapter: InterruptLifecycleHooks` — calls 2 interrupt methods (requested, interrupted)
  - **Each adapter MUST implement `Hook` trait** — `fn id(&self) -> &str` returns format!("wasm-{plugin_name}-{hook_kind}"), `fn priority() -> u32` returns plugin's configured priority
  - **Each adapter MUST implement its hook trait with `tracing::instrument` and trap-to-anyhow**:
    ```rust
    fn on_pre_turn(&self) {
        if let Err(e) = self.bridge.wrap_call("on_pre_turn", |b, s| b.try_call_on_pre_turn(s)) {
            tracing::warn!(hook = %self.id(), error = %e, "WASM hook on_pre_turn failed");
        }
    }
    ```
  - **The `wrap_call` helper** (on each adapter):
    - Clone bridge Arc
    - `let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| ...))`
    - If trap/Error → `tracing::warn!`, return (do NOT propagate)
    - If Ok(()) → return
    - Never propagate errors up the call chain — `fn on_pre_turn(&self) -> ()` must not return Result
  - **WASM strings → Rust strings**: use `bridge.extract_string(store, wit_str)` helper that reads memory from `guest.memory()` into a `String`. Reject non-UTF-8 with a clean error in `wrap_call`.
  - Must NOT: have adapters returning `Result` — the whole point is error isolation
  - Must NOT: have adapters panicking — all panics caught
  - Must NOT: depend on `oben-agent` — instead, put adapters in a "shared" crate or use re-exports

  Parallelization: Wave 2 | Blocked by: T2 | Blocks: T4, T5
  References: `oben-agent/src/hooks/kind.rs:241-319` (all 7 trait definitions), `oben-agent/src/hooks/runtime.rs:124-157` (emit patterns — adapters must call same signatures), `oben-agent/src/hooks/runtime.rs:133-147` (emit_stream_delta has debug logging — adapters should log too)
  Acceptance criteria: All 7 adapters compile against `oben-agent` traits; each trait impl has the `wrap_call` error isolation pattern; `cargo check` passes
  QA: `cargo check -p oben-agent --lib` + `cargo check -p oben-wasm` — happy: both crates compile. Evidence: .omo/evidence/task-3-wasm-hook-bridge.md. Also: grep for `unwrap()` or `expect()` in adapter impls — must be zero occurrences in the error paths
  Commit: Y | feat(agent): add 7 WASM hook adapter wrappers with error isolation

- [x] 4. Implement WasmHookRegistry — compile, discover hooks, produce adapters
  What to do / Must NOT do:
  - Create `oben-wasm/src/hook-registry.rs` (or extend `hook-bridge.rs` with a Registry struct)
  - `WasmHookRegistry` holds: `Arc<RwLock<Vec<Arc<dyn Hook>>>>` (same as HookEngine internal storage), `WasmRuntime` reference, `HookBuilder` reference
  - **Method: `async fn register_hooks(&mut self, component: Arc<PreparedComponent>, config: PlatformPluginConfig) -> Result<()>`**
    - Create 7 adapter instances from `component`+`config.name`
    - Push each into internal tracking vec
    - Push each into `HookBuilder` (via the builder's new method)
  - **Method: `async fn load_hooks_from_dir(&mut self, dir: &Path) -> Result<usize>`**
    - Uses existing `PluginLoader` → `discover_plugins()` to find .wasm files
    - For each discovered plugin, checks for `.plugin.json` (or extends `.platform.json` with `"hooks": true`)
    - If hooks are enabled, creates adapter instances and registers them
    - Returns count of registered hooks
  - **Method: `count(&self) -> usize`** — returns total registered WASM hooks
  - **Method: `clear(&mut self) -> Result<()>`** — clears all registered hooks (for reset/reload)
  - Must NOT: create the WASM adapters here (that's in T3's bridge module)
  - Must NOT: depend on `oben-agent` directly — accept `HookBuilder` through constructor, push through its API
  - Must NOT: call `HookEngine::new()` — only interacts with HookBuilder

  Parallelization: Wave 2 | Blocked by: T2, T3 | Blocks: T6
  References: `oben-wasm/src/loader.rs:93-122` (load_plugins() discovery flow), `oben-wasm/src/runtime.rs:85-126` (prepare_component), `oben-gateway/src/main.rs:208-260` (gateway plugin loading pattern)
  Acceptance criteria: `register_hooks()` takes component + config, creates 7 adapters, pushes them to the internal vec and returns; `load_hooks_from_dir()` discovers and registers; count returns accurate number
  QA: `cargo check -p oben-wasm` — happy. Evidence: .omo/evidence/task-4-wasm-hook-bridge.md
  Commit: Y | feat(wasm): add WasmHookRegistry with register and load methods

- [x] 5. HookEngine + HookBuilder integration — expose WASM hook injection API
  What to do / Must NOT do:
  - **In `oben-agent/src/hooks/runtime.rs:HookEngine`** — add:
    ```rust
    pub fn insert_wasm_hooks(&self, hooks: impl IntoIterator<Item = Box<dyn Hook>>) {
        for hook in hooks {
            match hook.as_ref().id() {
                id if id.starts_with("wasm-agent-loop-") => self.agent_loop_hooks.write().unwrap().push(hook),
                id if id.starts_with("wasm-turn-") => self.turn_hooks.write().unwrap().push(hook),
                id if id.starts_with("wasm-tool-") => self.tool_hooks.write().unwrap().push(hook),
                id if id.starts_with("wasm-streaming-") => self.streaming_hooks.write().unwrap().push(hook),
                id if id.starts_with("wasm-system-") => self.system_hooks.write().unwrap().push(hook),
                id if id.starts_with("wasm-session-") => self.session_hooks.write().unwrap().push(hook),
                id if id.starts_with("wasm-interrupt-") => self.interrupt_hooks.write().unwrap().push(hook),
                _ => tracing::warn!(id, "unrecognized WASM hook ID pattern"),
            }
        }
    }
    ```
    Use `starts_with` prefix matching because the pattern is `wasm-{plugin}-{kind}-...`
  - **In `oben-agent/src/hooks.rs:HookBuilder`** — add:
    ```rust
    /// Inject pre-constructed hook trait objects into the builder.
    /// Typically called by outside systems (e.g. WASM plugin system).
    pub fn with_wasm_hooks(mut self, wasm_hooks: Vec<Box<dyn Hook>>) -> Self {
        for hook in wasm_hooks {
            match hook.as_ref().id() {
                id if id.starts_with("wasm-agent-loop-") => self.agent_loop_hooks.push(hook),
                // ... etc for each kind
                _ => tracing::warn!(id, "unrecognized WASM hook ID"),
            }
        }
        self
    }
    ```
  - Must NOT: change the `build()` method signature — it accepts `self` for chaining
  - Must NOT: change existing `register_*` methods — the new method sits alongside them
  - Must NOT: change `HookEngine::new()` constructor — it takes no-args, new method is post-construction

  Parallelization: Wave 3 | Blocked by: T3 | Blocks: T6
  References: `oben-agent/src/hooks.rs:31-39` (HookBuilder fields that need new methods), `oben-agent/src/hooks.rs:115-125` (HookBuilder::build())
  Acceptance criteria: Both `insert_wasm_hooks` and `with_wasm_hooks` correctly dispatch to each queue based on `hook.id()` prefix; `cargo check -p oben-agent` passes
  QA: Unit test in `oben-agent/src/hooks.rs` — happy: create a mock hook with `id() = "wasm-test-tool-test"`, push via `with_wasm_hooks`, verify it's in tool_hooks, then push to engine and verify it's in engine.tool_hooks. Evidence: .omo/evidence/task-5-wasm-hook-bridge.md
  Commit: Y | feat(agent): add WASM hook injection to HookBuilder and HookEngine

- [x] 6. Cargo.toml wiring — cross-crate dependencies for hook bridge
  What to do / Must NOT do:
  - **Root `Cargo.toml`**: add `"oben-wasm"` to workspace members (should already be there from Phase 1)
  - **`oben-wasm/Cargo.toml`**: 
    - Add `oben-agent` as optional dev-dependency for tests (to get HookTrait references)
    - OR: extract the 7 HookKind trait definitions to a shared crate — BAD, too disruptive
    - Better: add `oben-agent = { path = "../oben-agent", optional = true }` and gate adapter code behind `#[cfg(feature = "agent-traits")]`
  - **`oben-gateway/Cargo.toml`**: ensure `oben-wasm` is an optional dependency (already there from Phase 1), add `oben-agent` as a real dependency (it already depends on it)
  - **No new crates** — everything stays within existing crate boundaries
  - Must NOT: create a `oben-hooks` or shared crate — keep minimal boundaries
  - Must NOT: add wasmtime-wit-bindgen (future — WASM guest SDK is out of scope)

  Parallelization: Wave 3 | Blocked by: T3 | Blocks: T7
  References: `oben-wasm/Cargo.toml` (current deps), `oben-gateway/Cargo.toml` (wasm-plugins feature), root `Cargo.toml` (workspace members)
  Acceptance criteria: `cargo check -p oben-wasm` + `cargo check -p oben-gateway` both pass; no new crates created
  QA: `cargo check --workspace` — happy: clean compile, 0 new warnings. Evidence: .omo/evidence/task-6-wasm-hook-bridge.md
  Commit: Y | chore(cargo): wire oben-wasm hooks bridge cross-crate dependencies

- [x] 7. Gateway integration — connect WASM hook registry to agent startup
  What to do / Must NOT do:
  - In `oben-gateway/src/main.rs`: modify the WASM plugin loading section (lines ~208-260)
  - **After** discovering WASM components but **before** constructing the agent:
    1. Create `WasmHookRegistry` with the WasmRuntime
    2. Call `registry.load_hooks_from_dir(plugin_dir)` to discover and register
    3. Pass the registry's hooks to the HookBuilder via `with_wasm_hooks()`
    4. Build HookEngine with `HookBuilder::from_config(&hooks_config).with_wasm_hooks(registry.all_hooks()).build()`
  - **In the TUI path** (`oben-tui`): agent is constructed in `oben-tui/src/lib.rs` — also needs to inject WASM hooks. Add a `hooks_wasm: Option<Arc<Mutex<Vec<Box<dyn Hook>>>>>` parameter to the TUI app constructor, populated from config if available.
  - **In the CLI path** (`oben-cli`): same pattern — inject into HookBuilder
  - Must NOT: modify the factory-based platform spawning code (that's separate from hooks)
  - Must NOT: introduce config flags to enable/disable hooks — if WASM plugins are loaded, their hooks are registered automatically
  - Must NOT: change the gateway platform handles or start_all() flow

  Parallelization: Wave 4 | Blocked by: T5, T6 | Blocks: T8
  References: `oben-gateway/src/main.rs:89` (HookBuilder::from_config), `oben-gateway/src/main.rs:208-260` (current WASM plugin loading section), `oben-tui/src/lib.rs` (TUI agent construction), `oben-cli/src/dispatch.rs` (CLI agent construction)
  Acceptance criteria: Gateway creates HookBuilder, injects WASM hooks, builds HookEngine, passes to Agent::new(); cargo check passes
  QA: `cargo check -p oben-gateway` — happy. Evidence: .omo/evidence/task-7-wasm-hook-bridge.md
  Commit: Y | feat(gateway): inject WASM hooks into agent HookBuilder during startup

- [x] 8. Integration tests + unit tests for WASM hook bridge
  What to do / Must NOT do:
  - **T8a. Unit test: WasmHookBridge creates adapters** — create a minimal mock WASM component (empty WASM bytes that compile), instantiate bridge, verify all 7 adapter types created
  - **T8b. Unit test: HookId matching** — create mock hooks with `wasm-test-tool-*`, `wasm-test-turn-*`, etc. IDs; push via HookBuilder::with_wasm_hooks(); build engine; verify each hook landed in correct queue by checking `engine.count()` per category
  - **T8c. Unit test: Error isolation** — create a bridge that simulates a wasmtime trap when calling `try_call_on_tool_start`; verify the adapter's `on_tool_start()` does NOT panic or propagate error; verify `tracing::warn!` was called (use tracing-subscriber's test layer)
  - **T8d. Integration test: Gateway flow** — mock a WASM plugin file (empty .wasm, valid `.plugin.json`) in a temp dir; call `WasmHookRegistry::load_hooks_from_dir()`; verify hooks register into HookBuilder; verify engine can be built with wasm hooks
  - **T8e. Integration test: Full emit path** — set up HookEngine with a WASM adapter + a native test adapter; emit `emit_tool_start("cat", "{}")`; assert BOTH adapters' hooks were called (use Arc<AtomicUsize> counters per adapter per method)
  - Must NOT: require a real compiled WASM module (use `Vec::new()` as empty WASM bytes for unit tests)
  - Must NOT: require network calls or real LLMs — all tests use memory stores and mocks
  - Must NOT: change existing test code — only add new test files/functions

  Parallelization: Wave 5 | Blocked by: T6 | Blocks: F1
  References: `oben-wasm/tests/e2e_plugin_load.rs` (existing test pattern), `oben-agent/src/hooks.rs:135-151` (existing test), `oben-wasm/src/runtime.rs:63-70` (test mode config with cache disabled)
  Acceptance criteria: All 5 test scenarios pass; `cargo test -p oben-agent --lib` + `cargo test -p oben-wasm` both pass with 0 failures; coverage evidence captured
  QA: `cargo test -p oben-agent --lib -p oben-wasm --lib` — happy: all tests pass. Evidence: .omo/evidence/task-8-wasm-hook-bridge.md
  Commit: Y | test(wasm): add integration tests for WASM hook bridge

## Final verification wave

Runs in parallel after ALL todos. ALL must APPROVE. Surface results and wait for the user's explicit okay before declaring complete.

- [x] F1. Plan compliance audit
  Run `cargo test -p oben-agent --lib -p oben-wasm --lib -p oben-gateway` — NO failures
  Verify no `unwrap()`, `expect()`, or `.unwrap()` in hook adapter error paths (grep for `\.unwrap\(\)` in `oben-wasm/src/hook-bridge.rs`, `oben-wasm/src/wasm-hooks.rs`, `oben-wasm/src/hook-registry.rs`, `oben-agent/src/hooks.rs`)
  Verify every adapter method has `wrap_call` or equivalent error isolation
  Verify WIT interface has exactly 5 categories (no AgentLoop, no InterruptLifecycle)

- [x] F2. Code quality review
  Verify no new `dead_code` warnings in modified/new files
  Verify no `allow(dead_code)` was added as a workaround
  Verify adapter ID format is consistent: `wasm-{plugin}-{kind}-{method}`
  Verify HookEngine/HookBuilder changes follow fluent builder pattern (no signature changes)
  Verify gateway integration is feature-gated behind `#[cfg(feature = "wasm-plugins")]` (if gateway already has it)

- [x] F3. Real manual QA
  Run gateway with a dummy plugin dir — verify logs show discovery, hook registration, and successful build
  Verify HookEngine::count() includes registered WASM hooks
  Verify that a crashing WASM hook call does NOT crash the agent (insert a bad WAT → compile → load → emit)

- [x] F4. Scope fidelity
  Verify AgentLoopHooks and InterruptLifecycleHooks are NOT in the WIT interface
  Verify no WASM-side SDK code added
  Verify no hot-reload code added
  Verify no new crates created
  Verify cross-boundary data is strings only

## Commit strategy

One atomic commit:
```
feat(wasm): add WASM hook bridge — WIT interface, adapters, engine integration, gateway wiring
```
Include updated parity docs if relevant (e.g., `docs/PRD-hooks-parity.md` if it exists).

## Success criteria

1. 8/8 todos complete with passing tests
2. `cargo test -p oben-agent --lib` passes with all tests (no regressions)
3. `cargo test -p oben-wasm --lib` passes with all new tests
4. `cargo test -p oben-gateway` still passes (no regressions in gateway)
5. WIT interface defines exactly 5 hook categories
6. HookEngine dispatches emitted events to WASM adapters the same as native adapters
7. Error isolation confirmed: no panic/probe propagation from WASM → agent
