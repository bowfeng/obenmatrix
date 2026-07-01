# gateway-async-platforms - Work Plan

## TL;DR (For humans)

<!-- Fill this LAST, after the detailed plan below is written, so it summarizes the REAL plan. -->
<!-- Plain English for a non-engineer: NO file paths, NO todo numbers, NO wave/agent/tool names. -->

**What you'll get:** Platform adapter discovery from config + async concurrent startup so Telegram/Discord/Slack/WhatsApp can all start independently; if one fails, the rest keep running and a health state tracking system shows which platform is connecting/down/reconnecting.

**Why this approach:** Currently `main.rs` only wires QQ Bot by hand. Adding Telegram/Discord requires manual code edits for each. The fix is to discover enabled platforms from `GatewayConfig` automatically and start them concurrently. Per-platform try-catch + state tracking ensures a bad config kills one platform but not the gateway.

**What it will NOT do:** No new platform adapter implementations (just the discovery/startup skeleton so Telegram/Discord/Slack/WhatsApp are auto-discovered); no webhook-to-agent conversation routing; no dashboard or CLI commands for platform management.

**Effort:** Medium
**Risk:** Low - additive, fault-tolerant changes; existing QQ Bot path refactored but not rewritten
**Decisions I made for you:** State tracking via `Arc<tokio::sync::RwLock<PlatformRegistry>>` instead of `HashMap` + `Mutex` so reads don't block writes; platform discovery via iteration over `GatewayConfig` fields using a `build()`-style adapter constructor pattern, not `#[derive]` because each adapter's constructor args differ; health_check() already exists on `PlatformAdapter` trait so no changes needed there.

Your next move: approve → then `$start-work` for execution. Full execution detail follows below.

---

> TL;DR (machine): Medium effort, Low risk - platform discovery + async startup + health state registry for oben-gateway

## Scope
### Must have
1. **Platform state enum**: `PlatformStatus` (Idle, Connecting, Running, Failed, Error(String)), `PlatformInfo { name, status, error, started_at }`
2. **PlatformRegistry**: `Arc<RwLock<HashMap<String, PlatformInfo>>>` — thread-safe map of adapter name → state
3. **Platform discovery in main.rs**: iterate `GatewayConfig` to find all enabled platforms, construct adapters concurrently, register in `ResponseRouter`, log any failures without aborting
4. **Gateway.start_platforms refactoring**: iterate known platforms from config, spawn listeners concurrently, store `AbortHandle` per-platform
5. **Per-platform fault tolerance**: `listen()` failure logs error but keeps other platforms running; platform state moves to `Failed` with error message
6. **Health check endpoint / log**: ability to query current platform status via `Gateway::platform_status()` returning `PlatformRegistry` snapshot

### Must NOT have (guardrails, scope boundaries)
- Implementing Telegram/Discord/Slack/WhatsApp adapter adapters (just the discovery wiring)
- Webhook-to-agent conversion (already exists as `IncomingMessage` pattern)
- CLI commands to manage platform state
- WebSocket auto-reconnect logic (logging failed state is enough; reconnect is a future concern)
- Pluggable platform discovery from external files

## Verification strategy
> Zero human intervention - all verification is agent-executed.
- Test decision: TDD — unit tests for PlatformRegistry CRUD; integration test for discovery flow
- Evidence: .omo/evidence/task-1-gateway-async-platforms.md, task-2-*.md

## Execution strategy
### Parallel execution waves
> Target 5-8 todos per wave. Fewer than 3 (except the final) means you under-split.

### Dependency matrix
| Todo | Depends on | Blocks | Can parallelize with |
| --- | --- | --- | --- |
| T1: Platform state types | — | T2, T3 | — |
| T2: PlatformRegistry + Gateway struct changes | T1 | T4 | T5 |
| T3: ResponseRouter health extension | T1 | — | T2, T4 |
| T4: main.rs platform discovery | T1, T2, T3 | T5 | — |
| T5: Gateway.start_platforms refactoring | T1, T2 | — | T4 |
| T6: Tests | T3, T4, T5 | — | — |

## Todos
> Implementation + Test = ONE todo. Never separate.
<!-- APPEND TASK BATCHES BELOW THIS LINE WITH edit/apply_patch - never rewrite the headers above. -->

- [x] 1. Add `PlatformState` and `PlatformInfo` types to platform.rs
  What to do / Must NOT do: Add `PlatformStatus` enum (Idle, Connecting, Running, Failed(String)) and `PlatformInfo` struct with `name`, `status`, `started_at` fields. Must NOT modify existing `PlatformAdapter` trait or `IncomingMessage`/`OutgoingMessage`.
  Parallelization: Wave 1 | Blocked by: None | Blocks: T2, T3, T5
  References (executor has NO interview context - be exhaustive): `oben-gateway/src/platform.rs:27-44` (PlatformAdapter trait), `oben-gateway/src/gateway.rs:15-21` (Gateway struct)
  Acceptance criteria (agent-executable): `cargo test --package oben-gateway --lib` compiles and passes; PlatformStatus + PlatformInfo are public, derive Debug + Clone, status shows in format string via Display impl
  QA scenarios (name the exact tool + invocation): happy `cargo test --package oben-gateway --lib -- platform::tests` shows new tests pass; failure test verifies Failed(status) error string propagation. Evidence .omo/evidence/task-1-gateway-async-platforms.md
  Commit: Y | types(platform-state): add PlatformStatus enum and PlatformInfo struct

- [x] 2. Add `PlatformRegistry` and update `Gateway` struct
  What to do / Must NOT do: Create `PlatformRegistry` as `Arc<RwLock<HashMap<String, PlatformInfo>>>` inside `Gateway`. Add `platform_registry` field. Add `new()`, `register_platform()` (transition → Connecting → Running or Failed), `platform_status()` (returns HashMap clone). Also add `PlatformRegistry::new()` constructor. Must NOT add fields to Dispatcher or ResponseRouter.
  Parallelization: Wave 2 | Blocked by: T1 | Blocks: T4, T5
  References: `oben-gateway/src/gateway.rs:15-35` (Gateway struct + new()), `oben-gateway/src/gateway.rs:66-81` (start_blocking), `oben-gateway/src/gateway.rs:83-121` (start_platforms)
  Acceptance criteria: Gateway::new() initializes empty registry; register_platform(name, status, error_opt) transitions state; platform_status() returns a HashMap clone with all registered entries; `cargo test --package oben-gateway --lib` passes
  QA scenarios: happy - register_platform("qq_bot", Running, None) → platform_status().get("qq_bot") returns Some with Running status; failure - register_platform("telegram", Failed("bad token"), error) → verify error in status. Evidence .omo/evidence/task-2-gateway-async-platforms.md
  Commit: Y | arch(registry): add PlatformRegistry for platform state tracking in Gateway

- [x] 3. Refactor ResponseRouter to support multi-platform
  What to do / Must NOT do: ResponseRouter already supports `HashMap<String, Box<dyn PlatformAdapter>>` — verify it works for multi-platform. Add `register_all()` batch method. Add `list_registered()` method returning Vec of names. The adapter registration in main.rs currently hardcodes `"qq_bot"` string — make it use the config key instead. Must NOT change `send()` signature.
  Parallelization: Wave 2 | Blocked by: T1 | Blocks: T4
  References: `oben-gateway/src/router.rs:1-54` (ResponseRouter), `oben-config/src/config.rs:323-330` (GatewayConfig fields)
  Acceptance criteria: ResponseRouter::register_all(Iter<(&str, Box<dyn PlatformAdapter>)>) registers multiple in one call; list_registered() returns all names; existing QQ Bot path still works. `cargo test --package oben-gateway --lib` passes
  QA scenarios: happy - register_all([("qq","adapter"),("tg","adapter")]) → list_registered() == vec!["qq","tg"]; failure - register_all with empty iterator → list_registered() == []. Evidence .omo/evidence/task-3-gateway-async-platforms.md
  Commit: Y | refactor(router): add register_all and list_registered to ResponseRouter

- [x] 4. Discover and start platforms concurrently in main.rs
  What to do / Must NOT do: Replace the hardcoded QQ Bot block in `main.rs:183-209` with a `discover_platforms(config)` function that iterates all config fields. Build each adapter inside a `tokio::spawn` and await them concurrently via `tokio::join!` for 2+ platforms or `futures::future::join_all` for generic count. If an adapter construction fails, log the error, set PlatformState::Failed, and continue. Must NOT change dispatch logic or coordinator.
  Parallelization: Wave 3 | Blocked by: T1, T2, T3 | Blocks: T5 (uses built adapters)
  References: `oben-gateway/src/main.rs:183-209` (current QQ Bot init), `oben-gateway/src/main.rs:137-219` (full main function), `oben-gateway/src/main.rs:150-181` (config loading, dispatcher setup)
  Acceptance criteria: `discover_platforms()` returns Vec<(name, Box<dyn PlatformAdapter>, Result)>; all enabled platforms registered concurrently; if qq_bot config fails, telegram still registers; `cargo test --package oben-gateway` passes; compilation succeeds
  QA scenarios: happy - config has qq_bot + telegram enabled → both spawn as separate tasks, both register; failure - qq_bot has bad secret → error logged, telegram still starts, `PlatformStatus::Failed` recorded for qq_bot. Evidence .omo/evidence/task-4-gateway-async-platforms.md
  Commit: Y | feature(platform-discovery): async concurrent platform adapter startup

- [x] 5. Refactor Gateway.start_platforms to iterate config
  What to do / Must NOT do: `start_platforms()` currently has a single QQ Bot branch. Refactor to iterate over platform adapters known to the registry, spawn each listener with `tokio::spawn`, collect abort handles. Each platform listener must have its own error handling — if `listen()` returns Err, log error and keep running (other platforms unaffected). Add `platform_handles` map: `HashMap<String, AbortHandle>`. Must NOT change message routing or dispatcher.
  Parallelization: Wave 3 | Blocked by: T1, T2 | Blocks: T6
  References: `oben-gateway/src/gateway.rs:64-81` (start_blocking), `oben-gateway/src/gateway.rs:83-121` (start_platforms), `oben-gateway/src/gateway.rs:19-20` (platform_handles field)
  Acceptance criteria: start_platforms() spawns each active platform as separate tokio task; if qq_bot.listen() errors, telegram.listen() continues; all abort handles stored; `cargo test --package oben-gateway --lib` passes
  QA scenarios: happy - two platforms enabled → two JoinHandles in platform_handles; failure - listen() on one returns Err → error logged, other still connected. Evidence .omo/evidence/task-5-gateway-async-platforms.md
  Commit: Y | refactor(gateway): concurrent platform listener spawning from config

- [ ] 6. End-to-end tests
  What to do / Must NOT do: Integration test in `oben-gateway/src/gateway.rs#[cfg(test)]` that: (a) creates a Gateway with multi-platform config, (b) verifies PlatformRegistry has all platforms in Idle/Failed state, (c) starts platforms and verifies Running state. Must NOT make real network calls — use mock adapters. Must NOT test the coordinator task path.
  Parallelization: Wave 4 | Blocked by: T3, T4, T5 | Blocks: None
  References: `oben-gateway/src/platform.rs:141-165` (existing TestAdapter pattern), `oben-gateway/src/gateway.rs:148-221` (existing tests)
  Acceptance criteria: New test file/function compiles and passes; verifies state transitions: Idle → Running or Idle → Failed; no network calls
  QA scenarios: happy - two mock platforms register → both show Running; failure - mock platform fails on listen → shows Failed with error message. Evidence .omo/evidence/task-6-gateway-async-platforms.md
  Commit: Y | test(integration): add e2e platform discovery and state transition tests

## Final verification wave
> Runs in parallel after ALL todos. ALL must APPROVE. Surface results and wait for the user's explicit okay before declaring complete.
- [ ] F1. Plan compliance audit
- [ ] F2. Code quality review
- [ ] F3. Real manual QA
- [ ] F4. Scope fidelity

## Commit strategy
Single PR: `#xx-gateway-async-platforms` with title `#xx: Async platform discovery + concurrent startup + health state registry`
All 6 todos committed together in one atomic commit with updated PRD gateway parity doc.

## Success criteria
1. Gateway config can enable multiple platforms (qq_bot + telegram + discord + slack + whatsapp) via YAML only — no code changes needed
2. All enabled platforms start concurrently (measured: total startup < max single-platform startup × 1.2)
3. Platform startup failure in one does NOT prevent other platforms from starting
4. Runtime health status queryable via `Gateway::platform_status()` — returns current status of all platforms
