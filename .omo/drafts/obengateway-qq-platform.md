---
slug: obengateway-qq-platform
status: drafting
intent: clear
pending-action: present approval gate
approach: Phase 1: Extract PlatformSDK + rewrite main.rs/gateway.rs dispatcher to use PlatformRegistry pattern; QQ stays inline as first registered platform, setting up factory-based extensibility for future third-party plugins.
---

# Draft: obengateway-qq-platform

## Components (topology ledger)
| id | outcome | status | evidence path |
|---|---|---|---|
| Platform SDK new crate | Shared trait + types + registry | active | .omo/... |
| QQ adapter factory | Register QQ via PlatformRegistry | active | oben-gateway/src/platform/ |
| main.rs refactor | Wire registry into entry point | active | oben-gateway/src/main.rs:136-267 |
| gateway.rs refactor | Remove hardcoded start_platforms | active | oben-gateway/src/gateway.rs:145-266 |
| ResponseRouter update | Use registry for platform names | active | oben-gateway/src/router.rs |

## Open assumptions (announced defaults)
1. **Phase 1 keeps QQ inline** — QQ stays in oben-gateway/src/platform/qq for this phase. A future Phase 2 extracts to oben-platform-qq once the plugin SDK interface is proven.
2. **listen() API unchanged** — QQ's pattern (spawn internal loop, return AdapterHandle for external abort) is preserved. No breaking changes to downstream code consuming the trait.
3. **No dynamic library loading** — true third-party plugin FFI (Phase 3) is deferred. Phase 1 builds the registry + factory abstraction so plugin crates can register themselves at static link time.
4. **max_message_length in PlatformRegistry metadata** — each registered platform declares its max message length; the registry enforces truncation during send before routing to the adapter.
5. **QqMessageHandler parameter removed** — QQ internally dispatches via `SharedState.dispatcher`; the unused `Box<dyn MessageHandler>` parameter on `listen()` is dropped.

## Findings (cited - path:lines)
1. **QQ is the only live platform adapter** (`oben-gateway/src/qq_bot.rs:1-742`) — implements `PlatformAdapter` via `async_trait`, handles WS connection + event loop + REST send
2. **Herme plugin pattern** (`hermes-agent/gateway/platform_registry.py:38-257`) — `PlatformEntry` dataclass with `adapter_factory`, `check_fn`, `validate_config`; plugins call `ctx.register_platform()` with all metadata
3. **Herme IRC plugin** (`hermes-agent/plugins/platforms/irc/adapter.py:929-971`) — single `register(ctx)` function that registers everything: factory, validation, cron delivery, auth env vars, display emoji, platform prompt hint
4. **Herme adapter creation** (`hermes-agent/gateway/run.py:5936-6029`) — checks `platform_registry.is_registered()` first (fast path for plugins), falls through to if/elif chain for built-ins
5. **Current gateway.rs start_platforms()** (`oben-gateway/src/gateway.rs:145-266`) — hardcoded if/elif for telegram, discord, slack, whatsapp (all "Not implemented yet"), and QQ Bot
6. **Current main.rs discover_platforms()** (`oben-gateway/src/main.rs:136-199`) — async spawn for each placeholder; QQ registered separately at line 253-267
7. **PlatformAdapter trait** (`oben-gateway/src/platform.rs:67-88`) — `name()`, `listen(Box<dyn MessageHandler>)`, `stop()`, `send(OutgoingMessage)`, `health_check()`
8. **ResponseRouter** (`oben-gateway/src/router.rs:13-106`) — `HashMap<String, Box<dyn PlatformAdapter>>` keyed by name; `dispatch_response()` parses platform from session_key
9. **QQ sends internally** (`oben-gateway/src/qq_bot.rs:416-436`) — `dispatch_message()` directly calls `self.dispatcher.dispatch()`, no MessageHandler
10. **QQ listen() pattern** (`oben-gateway/src/qq_bot.rs:519-527`) — spawns internal loop, gets stop_tx for shutdown, waits on rx for graceful exit; never uses the MessageHandler parameter
11. **QqMessageHandler unused** (`oben-gateway/src/gateway.rs:307-318`) — registered but does nothing, just logs and returns None; QQ uses its own dispatcher dispatch path
12. **QQ protocol types** (`oben-gateway/src/qq_protocol.rs`) — OpCode, EventType, Intents, WsIncomingMessage, SendMessageRequest etc.

## Decisions (with rationale)
1. **Registry-first, factory-based** — `PlatformRegistry` stores `AdapterFactory` closures; no if/elif chains in main.rs or gateway.rs. Factory takes `(&QQBotConfig, Arc<Dispatcher>)`, returns `Box<dyn PlatformAdapter>`.
2. **PlatformSDK as separate crate** (`oben-platform-sdk`) — all shared types move here so future platform plugins (e.g., `oben-platform-telegram`) can depend on it without importing gateway internals.
3. **AdapterHandle for lifecycle** — `listen()` returns `AdapterHandle(AbortHandle)` which the gateway stores. Gateway abort → adapter's internal loop receives signal → clean shutdown.
4. **Keep QQ inline for Phase 1** — avoiding an extra crate extraction keeps the diff focused on the registry architecture. QQ gets factory-registered in main.rs.
5. **Drop MessageHandler abstraction** — QQ never used it. Removing it from the trait simplifies the API and removes dead code.

## Scope IN
- Create `oben-platform-sdk` crate with `PlatformAdapter` trait, types, and `PlatformRegistry`
- Register QQ adapter via factory pattern instead of inline instantiation
- Refactor `main.rs` to wire `PlatformRegistry` (no hardcoded if/elif)
- Refactor `gateway.rs` to use registry for platform start/status tracking
- Refactor `ResponseRouter` to get platform names from registry
- Remove unused `MessageHandler` trait and `QqMessageHandler` struct

## Scope OUT (Must NOT have)
- Extract QQ into separate `oben-platform-qq` crate (Phase 2)
- Extract Telegram adapter (Phase 2+)
- Dynamic library loading / FFI plugin mechanism (Phase 3)
- Platform-specific configuration validation (Phase 2)
- Cron delivery integration for new platforms (Phase 2)
- Auth/allowlist integration per-platform (Phase 2)
- Platform status display UI integration (Phase 2)
- Any product code changes outside oben-gateway and platform SDK

## Open questions
- None remaining — QQ factory pattern, registry design, and lifecycle are resolved.

## Approval gate
- status: awaiting-approval
- pending-action: present plan summary to user, wait for explicit approve
