---
slug: wasm-hook-bridge
status: awaiting-approval
intent: clear
approach: Bridge pattern — WASM-exported hook callbacks wrapped as Box<dyn HookTrait>, pushed into existing HookEngine. WIT defines guest exports. Gateway registers hooks before Agent::new().
pending-action: Write .omo/plans/wasm-hook-bridge.md
---

# Draft: wasm-hook-bridge

## Components (topology ledger)
1 | WIT hook.wit interface | active | oben-wasm/wit/hook.wit
2 | WasmHookBridge + WasmHost | active | oben-wasm/src/hook-bridge.rs (new)
3 | WasmHookRegistry | active | oben-wasm/src/wasm-hooks.rs (new)
4 | 7 hook adapters (agent/turn/streaming/tool/system/session/interrupt) | active | oben-wasm/src/wasm-hooks.rs (new)
5 | HookEngine::insert_wasm_hooks | active | oben-agent/src/hooks/runtime.rs (modify)
6 | HookBuilder::with_wasm_hooks | active | oben-agent/src/hooks.rs (modify)
7 | Gateway main.rs integration | active | oben-gateway/src/main.rs (modify)
8 | tests (6 scenarios) | active | oben-wasm/tests/integration.rs (new)

## Open assumptions (announced defaults)
| Assumption | Default | Rationale | Reversible? |
|---|---|---|---|
| Hook categories | TurnLifecycle, ToolLifecycle, Streaming, SystemEvents, SessionLifecycle — skip AgentLoop/Interrupt | AgentLoop = once-per-run, Interrupt = Ctrl+C — both runtime-unusable for plugins | Yes |
| Bridge pattern | WASM exports → Box<dyn HookTrait> wrappers → same HookEngine queue | Minimal surface area; zero changes to broadcast loop | Yes |
| Error isolation | WASM trap → warn log, agent continues | Safety requirement: never crash agent from bad plugin | No |
| Gateway wires hooks | Before Agent::new(), after WASM plugin loader | One-time registration, no hot-reload | Yes |
| Cross-crate deps | oben-wasm uses optional oben-agent dev-dep for trait references | Keep runtime isolated, test-only coupling | Yes |
| Strings only | All WASM→Rust boundary data is UTF-8 strings | Avoid binary complexity in Phase 1 | Yes |

## Findings (cited - path:lines)
- 7 hook traits: `oben-agent/src/hooks/kind.rs:241-319` — Hook (base), AgentLoop, TurnLifecycle, ToolLifecycle, Streaming, SystemEvents, SessionLifecycle, InterruptLifecycle — all default no-op, Send+Sync
- HookEngine holds 7 `Arc<RwLock<Vec<Box<dyn ...>>>>` queues, register_* pushes, emit_* iterates | `oben-agent/src/hooks/runtime.rs:90-157`
- HookBuilder::from_config builds NudgeHook (optional), then register_* fluent chain | `oben-agent/src/hooks.rs:43-125`
- Agent::new() builds HookEngine: `HookBuilder::from_config(&config.hooks).build()` | `oben-agent/src/agent.rs:89`
- Agent exposes `hooks()` → `&Arc<HookEngine>` for TUI/CLI reuse | `oben-agent/src/agent.rs:215`
- WasmRuntime manages wasmtime Engine + PreparedComponent cache | `oben-wasm/src/runtime.rs:60-136`
- PluginLoader discovers .wasm files, reads platform.json sidecars, compiles | `oben-wasm/src/loader.rs:38-172`
- WasmPlatformAdapter stub — State machine (Stopped→Starting→Running) | `oben-wasm/src/bridge.rs:21-117`
- Gateway feature-gated WASM loader in main.rs:208-260, uses platform.json sidecars | `oben-gateway/src/main.rs:208-260`
- WIT world defined in lib.rs:34-43, actual wit/.wit files not yet on disk | `oben-wasm/src/lib.rs:34-43`

## Decisions
1. **Bridge over direct callbacks**: Wrap WASM exports into `Box<dyn HookTrait>` — same dispatch path as TUI adapters. No new traits, no new dispatching.
2. **WIT: guest exports** (plugin writes `hook_on_tool_start(name, call_id)`, host calls from emit). NOT host imports guest — host needs to call WASM synchronously from emit loop.
3. **No AgentLoop/Interrupt in Phase 1**: startup/kill hooks not useful for plugins.

## Scope IN
- WIT hook.wit (5 categories, ~20 exported functions + metadata)
- WasmHookBridge (wasmtime component → exported function calls via Linker)
- 7 adapter wrappers (Arc<WasmHookBridge> → Box<dyn HookTrait>, wrap_call error isolation)
- WasmHookRegistry (register_hooks, load_hooks_from_dir, count, clear)
- HookEngine::insert_wasm_hooks (prefix dispatch by hook.id())
- HookBuilder::with_wasm_hooks (fluent batch injection)
- Gateway integration: WASM hooks into HookBuilder before Agent::new()
- Tests: 6 scenarios (adapter creation, hook ID dispatch, error isolation, gateway flow, emit path)
- Cargo.toml wiring (optional dev-deps, no new crates)

## Scope OUT (Must NOT have)
- AgentLoopHooks, InterruptLifecycleHooks (WIT + adapters)
- Hot-reload / live update
- WASM-side SDK / guest bindings (wit-bindgen)
- Real WASM instantiation in bridge.rs (stub/scaffold)
- New crates, shared crates
- Binary data across boundary (strings only)
- Hook priority override from metadata

## Open questions
None — all architecturally determined.

## Approval gate
status: awaiting-approval
pending: present plan to user for approval
