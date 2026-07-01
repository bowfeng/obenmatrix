---
slug: wasm-platform-plugins
status: awaiting-approval
intent: clear
pending-action: approve plan and say "开始" to start execution
approach: Add `oben-wasm` crate with wasmtime WASM runtime, define WIT plugin interface, implement loader that scans ~/.obenalien/plugins/ for .wasm + .plugin.json pairs, wraps each loaded plugin as a PlatformAdapter, and integrates into gateway main.rs. Built-in adapters stay unchanged (no migration).
---

# Draft: wasm-platform-plugins

## Components (topology ledger)
| id | outcome | status | evidence path |
|----|---------|--------|---------------|
| C1 | WIT interface compiles (`wit` types) | active | `oben-wasm/tests/wit_build.rs` |
| C2 | WASM runtime + loader compiles clean | active | `cargo check -p oben-wasm` |
| C3 | Gateway loads plugins, starts, compiles | active | `cargo check -p oben-gateway` |
| C4 | End-to-end: .wasm loads → PlatformAdapter usable | active | `oben-wasm/tests/e2e_plugin_load.rs` |

## Open assumptions (announced defaults)
| assumption | default | rationale | reversible? |
|------------|---------|-----------|---------------|
| WASM runtime | wasmtime 44.0.3 + component-model | Ironclaw already uses this; proven in production; best WASI Preview 2 support | Yes — swap to wasmer later |
| Plugin directory | ~/.obenalien/plugins/ (with `plugins:` section in config.yaml) | Matches existing config pattern; user-configurable | Yes |
| Plugin format | .wasm + .plugin.json sidecar (not Capabilities.json) | Simpler than Ironclaw's full capabilities.json; sufficient for platform adapters | Yes |
| WIT version | 0.1.0 | First release; major version bumps = ABI break; gate on host version | Yes |
| Security | Time + memory limits, no FS/network by default | Least-privilege; sandbox the plugin | Yes — extend later |
| Built-in adapter migration | Not in scope | Built-in adapters stay as-is; can be converted to WASM later as separate work | Yes |
| Plugin hot-reload | Not in scope (defer to Phase 2) | Restart-to-load is the explicit requirement | Yes |
| Cross-language SDK | Rust-only for now | First release needs one solid SDK; Go/TS SDKs later | Yes |

## Findings (cited - path:lines)
- **Current main.rs**: `oben-gateway/src/main.rs:100-206` — 100+ line if/elif chain with `#[cfg(feature)]` for built-in platforms
- **PlatformAdapter trait**: `oben-platform-sdk/src/platform.rs:68-83` — 5 methods: `name()`, `listen()`, `stop()`, `send()`, `health_check()`
- **Ironclaw WASM runtime**: `wasmtime 44.0.3` + `component-model` + `wasmtime-wasi`
- **Ironclaw loader**: `load_from_dir()` finds `.wasm` + `.capabilities.json`, validates WIT version, compiles in `spawn_blocking`, caches PreparedModule
- **Ironclaw version check**: `check_wit_version_compat()` — semver major + minor matching for 0.x
- **Gateway Config**: `oben-config/src/config.rs:492` — `GatewayConfig` struct with typed platform fields
- **Cargo.toml workspace**: `oben-gateway` has features: whatsapp, telegram, discord, slack

## Decisions (with rationale)
| decision | choice | rationale |
|----------|--------|-----------|
| Runtime | wasmtime 44.0.3 + component-model | Ironclaw proven; best WASI Preview 2; component model for interface isolation |
| New crate | `oben-wasm` (sibling of `oben-gateway`) | Keeps WASM code modular; gateway only depends on it for loading |
| Adapter wrapper | trait object `Box<dyn PlatformAdapter>` wrapping WASM instance | Existing gateway code uses `PlatformFactory` trait — wrap plugin behind it |
| WIT polling model | Plugin calls host functions for `send_message()`, receives events via polling `next_event()` | Simpler than callback model; WASM can't spawn threads so no background listeners needed |
| Config location | Add `platforms: Option<Vec<PlatformConfig>>` to `GatewayConfig` | Extensible; user can add plugin entries by name + WASM path |

## Scope IN
- Add `wasmtime` + features to workspace Cargo.toml
- Create `oben-wasm/` crate with:
  - `wit/` — WIT interface definition for platform plugins
  - `runtime.rs` — Wasmtime engine init + component preparation
  - `loader.rs` — Scan `~/.obenalien/plugins/`, load .wasm + .plugin.json, verify WIT version
  - `bridge.rs` — WASM instance → `PlatformAdapter` trait object wrapper
- Update `GatewayConfig` in `oben-config` to accept plugin platform entries
- Update `oben-gateway/src/main.rs` to call loader instead of built-in chain (hybrid: plugins + built-ins)
- Add unit tests for loader, version check, error handling
- Add integration test: fake .wasm loads, is usable as PlatformAdapter

## Scope OUT (Must NOT have)
- ~~Async `listen()` inside WASM — plugin polls events on host (blocking loop)~~
- ~~Plugin hot-reload (defer to Phase 2)~~
- ~~Remote plugin marketplace / OTA updates~~
- ~~WASM plugin debugging (REPL, profiler)~~
- ~~Cross-language plugin SDKs (Go, TS, Zig)~~
- ~~Migration of existing built-in adapters to WASM~~ (done in separate work)
- ~~Plugin sandboxing beyond wasmtime time/safety limits~~ (extend later)

## Open questions
*(None — user's WASM architecture is clear and matches Ironclaw reference)*

## Approval gate
- **status**: awaiting-approval
- **pending action**: User says "approve" or "开始" (start)
- **summary**: Add `oben-wasm` crate with wasmtime WASM runtime. Define WIT interface for platform plugins (~80 lines). Build loader that scans `~/.obenalien/plugins/`, checks `.wasm` + `.plugin.json` pairs, validates WIT version, compiles to sandboxed runtime. Wrap each loaded plugin as `Box<dyn PlatformAdapter>`. Update gateway main.rs to call loader alongside built-in factory chain. 7 tasks, ~1200 LOC across 6 new files, 2 modified. ~150 LOC tests. TDD verification with 3 test modules.
