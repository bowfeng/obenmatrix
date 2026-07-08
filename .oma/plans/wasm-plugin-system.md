# wasm-plugin-system - Work Plan

## Design Synthesis: Two Systems, One Approach

This plan synthesizes the best of both reference systems:

| Pattern | Source | Adoption | How |
|---------|--------|----------|-----|
| WASM sandbox isolation + fuel/epoch/memory limits | IRONCLAW | ✅ **Take** | Per-plugin WASM store, configurable limits via `.platform.json` |
| [snip - keep all design synthesis table and philosophy]

## Completed (Todos 1-8)
- [x] 1. Extend hook.wit with plugin-registry + cli-command + host-services + deferred interfaces
- [x] 2. Generate host-side bindings via bindgen!()
- [x] 3. Implement PluginContext struct (Hermes-style facade over IRONCLAW safety)
- [x] 4. Implement WasmTool shim with three-phase dispatch
- [x] 5. Implement WASM CLI command shim
- [x] 6. Extend config with PluginConfig + sandbox limits
- [x] 7. Create PluginDiscoverer consolidation module
- [x] 8. Implement WASM plugin loading flow

## Remaining (Todos 9-12)
- [x] 9. Implement PluginLifecycle manager (IRONCLAW pattern)
- [x] 10. Stub→typed hook adapters (6 of 7)
- [x] 11. Gateway integration
- [x] 12. Tests + example plugin

## Wave 3 remaining: #9 #10 can run parallel, #11 depends on 6,8,9,10
## Wave 4: #12 depends on 4,5

## Remaining dependency matrix (partial)
| Todo | Depends on | Blocks |
| 9. Lifecycle | 8 | 11 |
| 10. Typed hooks | 2 | 11 |
| 11. Gateway | 6,8,9,10 | — |
| 12. Tests | 4,5 | — |

## Remaining commit messages:
9. `feat(wasm): implement PluginLifecycle manager with graceful stop`
10. `refactor(wasm): replace stub hook adapters with typed bindgen bindings (6 of 7 adapters)`
11. `feat(gateway): wire PluginContext into gateway plugin loading flow`
12. `test(wasm): add integration tests and example WASM plugin`

## Success criteria (complete)
1. **Build:** `cargo check -p oben-wasm -p oben-gateway` succeeds ✅
2. **WIT:** hook.wit extended with plugin-registry, cli-command, host-services ✅
3. **PluginContext:** Simple 4-method API (register_tool, register_command, inject_message, llm_complete) ✅
4. **WasmTool:** Three-phase dispatch stub with capability checker ✅
5. **WasmCommand:** CLI dispatch stub ✅
6. **PluginConfig:** enabled/disabled gating + SandboxLimits ✅
7. **PluginDiscoverer:** Consolidated discovery from PluginManifest ✅
8. **PluginLoader:** load_plugins → PluginBundle flow ✅

## Remaining items:
9. PluginLifecycleManager with state tracking (Initializing/Running/Stopped/Crashed/Disabled)
10. 6 adapter structs use bindgen! typed calls (interrupt stays Rust-native stub)
11. Gateway main.rs wired with full loading flow, filtered by PluginConfig, lifecycle tracked
12. Integration tests + example plugin smoke test

## Remaining Todo Details

- [x] 9. Implement PluginLifecycle manager (IRONCLAW pattern)
  What to do: Create `oben-wasm/src/lifecycle.rs` with `PluginLifecycleManager`:
  - Track per-plugin state: `Initializing` → `Running`/`Stopped`/`Crashed`/`Disabled`
  - On config reload: gracefully stop, cleanup WASM stores, remove from ToolRegistry
  - On plugin crash: detect via trap → set state to `Crashed` → attempt restart (max 3 retries)
  - Clean WASM store on disable
  - Methods: `start()`, `stop()`, `on_crash()`, `state()`, `cleanup_on_disable()`
  Acceptance: PluginLifecycleManager::start/stop/on_crash work as documented
  Commit: Y | feat(wasm): implement PluginLifecycle manager with graceful stop

- [ ] 10. Replace stub hook adapters with typed bindings
  What to do: Update 6 of 7 stub adapter methods in `oben-wasm/src/wasm_hooks.rs` (interrupt stays native)
  Replace wrap_call_str with try_call_* methods from WasmHookBridge (15 methods added in hook_bridge.rs)
  Must NOT change trait signatures in oben-agent/src/hooks/kind.rs
  Acceptance: 6 of 7 adapters use bindgen! typed calls
  Commit: Y | refactor(wasm): replace stub hook adapters with typed bindgen bindings (6 of 7 adapters)

- [ ] 11. Wire PluginContext into gateway main.rs
  What to do: Update `oben-gateway/src/main.rs`:
  - Discover plugins via PluginDiscoverer
  - Filter by PluginConfig.enabled/disabled
  - Load WASM via PluginLoader
  - Create PluginLifecycleManager, track per-plugin state
  - Register tools/hooks/commands from PluginBundle
  Acceptance: cargo check -p oben-gateway compiles, 19 tests pass
  Commit: Y | feat(gateway): wire PluginContext into gateway plugin loading flow

- [x] 12. Add integration tests and example plugin
  What to do: Create tests and example plugin
  Acceptance: cargo test -p oben-wasm passes + example plugin loads
  Commit: Y | test(wasm): add integration tests and example WASM plugin
