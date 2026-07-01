# obengateway-qq-platform - Work Plan

## TL;DR (For humans)

**What you'll get:** oben-gateway stops using hardcoded if/elif chains to start platform adapters. Instead it uses a `PlatformRegistry` where each platform registers itself via a factory function. QQ becomes the first factory-registered platform, and the same pattern is ready for third-party plugin crates to follow.

**Why this approach:** The current `main.rs` and `gateway.rs` have duplicated hardcoded `if enabled { create QQBotAdapter; register; }` code. Herme-agent proves a registry + factory pattern with plugin discovery works well. Building the registry and the `oben-platform-sdk` shared crate now means any future platform (Telegram, Discord, or a community plugin) just needs a factory function + registration call — zero main.rs changes.

**What it will NOT do:** Extract QQ to a separate crate (kept inline for Phase 1), dynamic library loading / FFI for runtime plugins, Telegram / Discord adapter implementations, cron delivery integration, per-platform auth/allowlists.

**Effort:** Medium
**Risk:** Medium - introduces a new crate and rewrites gateway startup flow; regression scope limited to gateway crate build + QQ runtime behavior
**Decisions to sanity-check:** (1) QQ stays inline in Phase 1 despite being the first registered platform; (2) `listen()` returns an `AdapterHandle(AbortHandle)` instead of taking a `MessageHandler` — QQ never used the handler and this removes dead code.

Your next move: approve — or run a high-accuracy review. Full execution detail follows below.

---

> TL;DR (machine): Medium effort, medium risk — creates oben-platform-sdk crate + PlatformRegistry pattern, refactors main.rs/gateway.rs/ResponseRouter; QQ factory-registered inline; no dynamic loading

## Scope
### Must have
1. Create `oben-platform-sdk` crate as shared library: `PlatformAdapter` trait, `IncomingMessage`, `OutgoingMessage`, `PlatformRegistry`, `AdapterFactory`, `AdapterHandle`
2. Move all types currently in `oben-gateway/src/platform.rs` to `oben-platform-sdk/src/platform.rs`
3. Refactor QQ adapter: move to `oben-gateway/src/platform/qq/`, create factory function that registers QQ with `PlatformRegistry`
4. Rewrite `main.rs` startup: remove hardcoded if/elif, create PlatformRegistry, register QQ, call registry.start_platforms()
5. Refactor `gateway.rs`: remove start_platforms(), use registry for platform status tracking
6. Refactor `ResponseRouter`: derive platform names from registry, add max_message_length enforcement
7. Remove unused `MessageHandler` trait and `QqMessageHandler` struct

### Must NOT have (guardrails, anti-slop, scope boundaries)
- Do NOT extract QQ to a separate `oben-platform-qq` crate
- Do NOT implement Telegram, Discord, Slack, or WhatsApp adapters
- Do NOT add dynamic library loading / FFI plugin mechanism
- Do NOT change `IncomingMessage` or `OutgoingMessage` struct fields
- Do NOT add per-platform auth/allowlist logic
- Do NOT modify any code outside oben-gateway/ and platform SDK/

## Verification strategy
> Zero human intervention - all verification is agent-executed.
- Test decision: tests-after (build compiles + existing QQ unit tests pass + integration)
- Evidence: .omo/evidence/task-<N>-obengateway-qq-platform.md
- All existing oben-gateway unit tests must pass after changes
- `cargo build -p oben-platform-sdk -p oben-gateway` must succeed with zero warnings
- QQ runtime behavior must be unchanged (same session_key pattern, same dispatch flow)

## Execution strategy
### Parallel execution waves
> Target 5-8 todos per wave. Fewer than 3 (except the final) means you under-split.

### Dependency matrix
| Todo | Depends on | Blocks | Can parallelize with |
| --- | --- | --- | --- |
| T1 | — | T3 (types), T4 (SDK types import) | T2, T3, T4, T5 |
| T2 | — | T6 (imports from gateway) | T1, T3, T4, T5 |
| T3 | — | T4 (needs types from T3) | T1, T2, T4, T5 |
| T4 | T1, T2, T3 | T8 (registry refactor) | T5, T6, T7 |
| T5 | — | T8 (need msg handler removed) | T1, T2, T3, T4, T6, T7 |
| T6 | T4 | T9 (QQ refactor) | T7, T8 |
| T7 | T4 | T8, T9 | T5, T6 |
| T8 | T4, T5, T6, T7 | T9 | — |
| T9 | T4, T7, T8 | — | — |
| F1-F4 | all todos | — | F1-F4 parallel |

## Todos
> Implementation + Test = ONE todo. Never separate.
<!-- APPEND TASK BATCHES BELOW THIS LINE WITH edit/apply_patch - never rewrite the headers above. -->
- [ ] 1. Create `oben-platform-sdk` crate (Cargo.toml, lib.rs, platform.rs)
  What to do: Create new crate directory with Cargo.toml (publish = true, edition 2021, dependencies: serde, anyhow, async-trait, tokio). In lib.rs: export platform module as `pub use platform::*;`. In platform.rs: copy all types from existing `oben-gateway/src/platform.rs` (IncomingMessage, OutgoingMessage, PlatformStatus, PlatformInfo, PlatformAdapter trait) — but remove the unused MessageHandler trait and its derive impl. The PlatformAdapter trait gets: fn name() → &str, async fn listen() → AdapterHandle, async fn stop(), async fn send() → Result<()>, async fn health_check() → bool. AdapterHandle wraps tokio::task::AbortHandle. Add PlatformRegistry struct with register(name, factory), create(name, config) → Option<Box<dyn PlatformAdapter>>, start_platforms(dispatcher) → Vec<AdapterHandle>, status() → HashMap<String, PlatformInfo>, update_status(name, status, error). Add PlatformEntry struct with name, label, max_message_length, factory_closure, optional validate_config/is_connected closures.
  Must NOT do: Do not add any platform-specific logic (no QQ, no Telegram). Do not add dependency on oben-gateway or any gateway internals.
  Parallelization: Wave 1 | Blocked by: — | Blocks: T3, T4 (types, SDK types import)
  References: oben-gateway/src/platform.rs:9-88 (types to copy), gateway/platform_registry.py:38-257 (herme PlatformEntry reference), platform.rs:67-83 (trait methods), qq_bot.rs:519-527 (QQ listen() pattern)
  Acceptance criteria: `cargo build -p oben-platform-sdk` succeeds with zero errors/warnings. All types and traits from platform.rs compile in the new crate.
  QA: happy: cargo check -p oben-platform-sdk (exact tool + invocation). failure: cargo check on non-existent file → should fail. Evidence .omo/evidence/task-1-obengateway-qq-platform.md
  Commit: Y | chore(workspace): add oben-platform-sdk crate scaffold
- [ ] 2. Move QQ protocol types to platform SDK (qq_protocol.rs)
  What to do: Move qq_protocol.rs contents to oben-platform-sdk/src/qq_protocol.rs. This file contains OpCode, EventType, Intents (bitflags), WsIncomingMessage, HeartbeatPayload, HelloPayload, IdentifyEvent, Properties, SendMessageRequest, SendMarkdownMessageRequest, MsgType, CloseCode, FileUploadResponse, FileType — all shared types that QQ uses and could be reused by any future QQ-based plugin. Update all existing `use super::qq_protocol::` imports in platform/qq_bot.rs and platform/qq_onboard.rs to `use oben_platform_sdk::qq_protocol::`.
  Must NOT do: Do not modify the contents of qq_protocol.rs — only move the file and update imports. Do not add new types.
  Parallelization: Wave 1 | Blocked by: — | Blocks: T6 (imports from platform SDK)
  References: oben-gateway/src/qq_protocol.rs (full file), oben-gateway/src/qq_bot.rs:16-19 (imports), oben-gateway/src/qq_onboard.rs (imports)
  Acceptance criteria: `cargo build -p oben-platform-sdk` succeeds. `cargo check -p oben-gateway` shows import errors for the moved types (expected — T3 fixes the re-export).
  QA: happy: cargo check -p oben-platform-sdk (compiles). failure: cargo check -p oben-gateway before T3 fixes (expects import errors). Evidence .omo/evidence/task-2-obengateway-qq-platform.md
  Commit: Y | refactor(platform-sdk): move QQ protocol types to shared crate
- [ ] 3. Register QQ factory in PlatformRegistry (create factory function)
  What to do: In oben-gateway/src/platform/qq/builder.rs (new file), create the `fn register_qq(registry: &PlatformRegistry, dispatcher: Arc<Dispatcher>)` helper. This function creates a PlatformEntry with name="qq_bot", label="QQ Bot", max_message_length=2048, adapter_factory=QQBotAdapter::new boxed into a Box, and registers it. Also create a module-level `fn create_qq_adapter(config: QQBotConfig, dispatcher: Arc<Dispatcher>) -> QQBotAdapter` that does `Self::new(app_id, app_secret, sandbox, shard, intents, dispatcher)`. The factory closure stored in the registry takes `(&QQBotConfig, Arc<Dispatcher>)` and returns `Box<dyn PlatformAdapter>`.
  Must NOT do: Do not add any platform registration to main.rs yet. Do not modify QQBotAdapter struct.
  Parallelization: Wave 1 | Blocked by: — | Blocks: T7, T8 (factory callable)
  References: oben-gateway/src/qq_bot.rs:446-466 (QQBotAdapter::new), platform/registry.rs:18 (PlatformEntry), gateway/platform_registry.py:929-971 (herme IRC register() as reference for structure)
  Acceptance criteria: Factory function exists and compiles. Registering QQ produces a PlatformEntry with correct name and factory.
  QA: happy: create a test in builder.rs that calls register_qq() and verifies registry has "qq_bot" entry. failure: register with empty config → factory should still produce entry (validation happens at create time, not registration time). Evidence .omo/evidence/task-3-obengateway-qq-platform.md
  Commit: Y | feat(gateway): add QQ adapter factory for PlatformRegistry
- [ ] 4. Rewrite main.rs to use PlatformRegistry pipeline
  What to move: The entire `discover_platforms()` function (main.rs:136-199) — replace with `discover_platforms()` that: (a) creates new PlatformRegistry, (b) calls `registry.register_qq(config, dispatcher.clone())`, (c) returns the registry. Create adapter handles, start QQ, register with ResponseRouter. Remove the hardcoded QQBotAdapter::new() call at lines 253-267 and replace with registry.create_and_start("qq_bot", config, dispatcher). Wire ResponseRouter to get platform names from registry.list_platforms() for sending.
  Must NOT do: Do not add telegram/discord/slack/whatsapp factory calls. Do not change AppConfig or GatewayConfig structures.
  Parallelization: Wave 1 | Blocked by: T1 (PlatformRegistry), T2 (types), T3 (factory callable) | Blocks: T9 (QQ refactor)
  References: oben-gateway/src/main.rs:136-267 (current hardcoded flow), gateway/run.py:5936-6029 (herme _create_adapter + registry lookup), router.rs:41-46 (ResponseRouter.send), gateway.rs:278-293 (register_platform/update status)
  Acceptance criteria: `cargo build -p oben-gateway` succeeds. main() creates PlatformRegistry, registers QQ, starts platform, blocks on ctrl-c. No hardcoded if/elif.
  QA: happy: cargo build -p oben-gateway (compiles). failure: QQ platform disabled in config → should start with empty platform list and block on ctrl-c. Evidence .omo/evidence/task-4-obengateway-qq-platform.md
  Commit: Y | refactor(gateway): replace hardcoded platform startup with PlatformRegistry pipeline
- [ ] 5. Refactor ResponseRouter to integrate with PlatformRegistry
  What to do: ResponseRouter currently holds adapters directly. Since the registry now owns adapters, ResponseRouter should: (a) keep a clone of Arc<PlatformRegistry> + HashMap<String, Arc<dyn PlatformAdapter>> for fast lookup, (b) on `send(name, msg)`, look up adapter from registry map, (c) on `dispatch_response()`, parse platform from session_key, apply max_message_length from registry metadata, then forward to adapter.send(). Update `register_all()` to accept `(name, adapter, max_length)` tuples. The adapter must be wrapped in Arc so it can be shared between the registry (for start/stop) and router (for send).
  Must NOT do: Do not change the OutgoingMessage struct fields. Do not add max_message_length to OutgoingMessage.
  Parallelization: Wave 1 | Blocked by: T4 (registry created in main) | Blocks: T9 (QQ send flow)
  References: router.rs:1-166 (full ResponseRouter), platform.rs:19-26 (OutgoingMessage), registry.rs:72-81 (max_message_length in PlatformEntry)
  Acceptance criteria: ResponseRouter.get_platform_name() returns platform name from registry. ResponseRouter.send() applies max_message_length truncation before calling adapter.send(). Tests pass.
  QA: happy: cargo test -p oben-gateway --test router_tests. failure: send to unregistered platform → expected Result::Err. Evidence .omo/evidence/task-5-obengateway-qq-platform.md
  Commit: Y | refactor(gateway): integrate ResponseRouter with PlatformRegistry
- [ ] 6. Refactor QQ adapter: move to platform/qq/, update listen(), remove MessageHandler
  What to do: Move platform/qq_bot.rs → platform/qq/mod.rs. Create platform/qq/protocol.rs symlink to above_platform_sdk::qq_protocol. Update the ModuleAdapter impl (lines 513-611): remove the `Box<dyn MessageHandler>` parameter from `listen()`, change it to take `Arc<dyn AbortHandle>` for stop signaling. Instead of spawning internal stop channel, receive the AbortHandle from caller, store it, and use it to abort self on shutdown. Keep QQBotAdapter::new() signature as-is since QQ is inline in gateway crate.
  Must NOT do: Do not modify the QQBotAdapter struct fields or send() implementation. Do not change the event loop logic.
  Parallelization: Wave 2 | Blocked by: T4 (registry pattern established) | Blocks: T8 (gateway.rs refactor)
  References: qq_bot.rs:513-611 (PlatformAdapter impl), qq_bot.rs:467-506 (spawn_loop + internal loop), qq_bot.rs:153-165 (QQBotConfig)
  Acceptance criteria: QQ adapter compiles with new listen() signature. QQ still connects and dispatches messages same as before.
  QA: happy: create_test_qq_listen() — mock dispatcher, call listen(), verify internal loop started with tokio::spawn. failure: listen() with already-aborted handle → adapter should exit immediately. Evidence .omo/evidence/task-6-obengateway-qq-platform.md
  Commit: Y | refactor(qq): move adapter to platform/qq/ module, update listen() signature
- [ ] 7. Refactor gateway.rs: remove start_platforms() hardcoded code
  What to do: Remove start_platforms() method (gateway.rs:145-266 entirely). Replace with: call `PlatformRegistry::list_platforms()` to get enabled platform names, iterate over them, look up each in the registry, call `registry.create_and_start(name, config, dispatcher)`, register each with ResponseRouter. Keep platform_registry tracking (register/ update_status). The Gateway struct no longer needs a dedicated `platform_handles: Mutex<HashMap<String, AbortHandle>>` since the registry returns handles directly.
  Must NOT do: Do not change the Gateway struct fields except removing the dedicated platform_handles field (it becomes a Vec<AdapterHandle>). Do not change start_blocking() — it still calls ctrl_c().
  Parallelization: Wave 2 | Blocked by: T4, T5, T6 | Blocks: —
  References: gateway.rs:145-266, gateway.rs:68-77 (struct fields), main.rs:272-279 (start_blocking call)
  Acceptance criteria: cargo check -p oben-gateway succeeds. Gateway no longer contains any hardcoded platform names.
  QA: happy: cargo test -p oben-gateway (all existing gateway tests pass). failure: start_platforms called with empty config → should return no handles. Evidence .omo/evidence/task-7-obengateway-qq-platform.md
  Commit: Y | refactor(gateway): remove hardcoded start_platforms, use registry for all platforms
- [ ] 8. Remove unused MessageHandler trait and QqMessageHandler
  What to do: Delete MessageHandler trait from platform.rs. Delete QqMessageHandler struct from gateway.rs:307-318. Remove all `use` statements for MessageHandler. Update any code that created `Box<dyn MessageHandler>`. The QQ adapter's listen() signature no longer takes a MessageHandler.
  Must NOT do: Do not rename or relocate the trait — delete it entirely. Do not remove any platform-specific code.
  Parallelization: Wave 2 | Blocked by: T6 (QQ adapter removed MessageHandler reference) | Blocks: —
  References: platform.rs:85-88 (MessageHandler trait), gateway.rs:307-318 (QqMessageHandler), gateway.rs:249-253 (QQ listen() call)
  Acceptance criteria: cargo check -p oben-gateway succeeds with zero unused import warnings. No references to MessageHandler.
  QA: happy: cargo check -p oben-gateway (no warnings about unused items). Evidence .omo/evidence/task-8-obengateway-qq-platform.md
  Commit: Y | refactor(gateway): remove dead MessageHandler trait and QqMessageHandler
- [ ] 9. Update module structure and exports in lib.rs
  What to do: Update oben-gateway/src/lib.rs: change `pub mod qq_bot` → `pub mod platform` and `pub mod qq_protocol` → `pub use oben_platform_sdk::qq_protocol`. Export: `pub use platform::*; pub use platform::qq::*; pub use platform::qq::builder::*;`. Ensure `Dispatcher`, `Gateway`, `ResponseRouter`, `PlatformRegistry`, `PlatformAdapter`, `IncomingMessage`, `OutgoingMessage`, `QQBotAdapter` are all re-exported at crate level.
  Must NOT do: Do not change any public API that external crates depend on. Do not add new re-exports beyond what's needed.
  Parallelization: Wave 2 | Blocked by: T6 (module structure), T8 (clean imports) | Blocks: —
  References: lib.rs:1-21 (current exports), qq_bot.rs:21 (pub use qq_protocol::*), main.rs:63-66 (imports)
  Acceptance criteria: All existing imports in main.rs still resolve. Public API surface unchanged.
  QA: happy: cargo check -p oben-gateway (resolves all imports). failure: one missing export → cargo check shows error. Evidence .omo/evidence/task-9-obengateway-qq-platform.md
  Commit: Y | refactor(gateway): update module structure and exports for platform SDK integration

## Final verification wave
> Runs in parallel after ALL todos. ALL must APPROVE. Surface results and wait for the user's explicit okay before declaring complete.
- [ ] F1. Plan compliance audit: Verify no hardcoded if/elif platforms in any file outside tests. Verify PlatformEntry is used instead of direct factory calls. Verify MessageHandler is gone.
- [ ] F2. Code quality review: Confirm no Arc<Mutex<>> or Arc<RwLock<>> added unnecessarily. Confirm no `anyhow` used for `Result<()>` in the SDK. Confirm all platform SDK docs are clean.
- [ ] F3. Real manual QA: `cargo build -p oben-platform-sdk -p oben-gateway && cargo test -p oben-gateway --lib` — all must pass. QQ runtime test: manually enable QQ in config and verify connection.
- [ ] F4. Scope fidelity: grep -r "if.*enabled.*telegram\|if.*enabled.*discord\|if.*enabled.*slack" in oben-gateway/src/ — should return zero matches (only test files). Confirm no changes to any file outside oben-gateway/ and platform SDK/.

## Commit strategy

| Wave | Commits | Structure |
| --- | --- | --- |
| 1 | T1, T2, T3 → squash into 1 commit | `feat(gateway): extract PlatformSDK crate + QQ factory registration` |
| 2 | T4, T5, T6 → squash into 1 commit | `refactor(gateway): rewrite main.rs + ResponseRouter to use PlatformRegistry` |
| 2 | T7, T8, T9 → squash into 1 commit | `refactor(gateway): remove hardcoded pipelines, clean up dead MessageHandler` |

Final: squash all 3 into one PR: `feat(gateway): PlatformRegistry-based platform adapter architecture`

Branch: `refactor/platform-registry`
PR title: `refactor(gateway): extract PlatformRegistry-based platform adapter architecture`

## Success criteria
1. `cargo check -p oben-platform-sdk` — zero errors, zero warnings
2. `cargo check -p oben-gateway` — zero errors, zero warnings
3. `cargo test -p oben-gateway --lib` — all existing tests pass (including gateway.rs tests)
4. `cargo test -p oben-gateway --test router_tests` — ResponseRouter tests pass
5. No hardcoded platform if/elif remains in gateway source code (verified by grep)
6. QQ adapter reachable through registry with same session_key format and dispatch flow
7. QQ runtime: adapter connects, dispatches messages, responds identically to pre-refactor
8. No changes to any file outside oben-gateway/ and platform SDK/
7. QQ runtime: adapter connects, dispatches messages, responds identically to pre-refactor
8. No changes to any file outside oben-gateway/ and platform SDK/
7. QQ runtime: adapter connects, dispatches messages, responds identically to pre-refactor
8. No changes to any file outside oben-gateway/ and platform SDK/
