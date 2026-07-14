# Gateway & Platforms — Parity vs Hermes-Agent

**Scope:** `oben-gateway` crate  
**Reference:** `/Users/ellie/workspace/hermes-agent/gateway/`

---

## Gap Matrix

| # | Feature | Severity | Status | Issue | Notes |
|---|---------|----------|--------|-------|-------|
| G.1 | PlatformAdapter trait | ✅ | ✅ | (built-in) | Trait defined in `oben-gateway` |
| G.2 | **Telegram adapter** | 🔴 | ✅ | [#42](https://github.com/bowfeng/oben-alien/issues/42) | webhook + polling registered in main.rs |
| G.3 | **Discord adapter** | 🔴 | ✅ | [#42](https://github.com/bowfeng/oben-alien/issues/42) | bot, slash commands, registered in main.rs |
| G.4 | **Slack adapter** | 🔴 | ✅ | [#42](https://github.com/bowfeng/oben-alien/issues/42) | RTM + Socket Mode, registered in main.rs |
| G.5 | **WhatsApp adapter** | 🟡 | ✅ | [#42](https://github.com/bowfeng/oben-alien/issues/42) | WA Web API, registered in main.rs |
| G.6 | **Session context per platform** | 🟡 | ❌ | [TBD] | per-platform session isolation |
| G.7 | **Delivery routing** | 🟡 | ✅ | [TBD] | platform-aware delivery |
| G.8 | **Slash command routing** | 🟢 | ✅ | [TBD] | /pause, /resume, /status |
| G.9 | **Message routing** | 🟡 | ✅ | [TBD] | incoming message routing to adapters |
| G.10 | **Platform registry** | 🟢 | ✅ | None | Dynamic adapter discovery with deferred loading, config validation, and plugin support |
| G.11 | **Gateway initialization** | 🟢 | ✅ | [TBD] | init with all adapters |
| G.12 | **Gateway shutdown** | 🟢 | ✅ | [TBD] | graceful stop via abort_handle.abort() |
| G.13 | **Gateway state persistence** | 🟡 | ✅ | [TBD] | persist gateway state |
| G.14 | **Gateway metrics** | 🟡 | ✅ | [TBD] | performance tracking |
| G.15 | **Gateway logging** | 🟢 | ✅ | [TBD] | event logging |
| G.16 | **Gateway health check** | 🟢 | ✅ | [TBD] | health monitoring |
| G.17 | **Gateway metrics export** | 🟡 | ❌ | [TBD] | Prometheus format |
| G.18 | **Gateway diagnostics** | 🟡 | ❌ | [TBD] | diagnostic info |
| G.19 | **Gateway CLI commands** | 🟡 | ❌ | [TBD] | management commands |
| G.20 | **Gateway webhooks** | 🟡 | ❌ | [TBD] | webhook handling |
| G.21 | **Gateway events** | 🟢 | ✅ | [TBD] | event emission |
| G.22 | **Gateway callbacks** | 🟢 | ❌ | [TBD] | callback hooks |
| G.23 | **Gateway middleware** | 🟡 | ❌ | [TBD] | middleware hooks |
| G.24 | **Gateway filters** | 🟡 | ❌ | [TBD] | message filtering |
| G.25 | **Gateway transformers** | 🟡 | ❌ | [TBD] | message transformation |
| G.26 | **Gateway validators** | 🟡 | ❌ | [TBD] | message validation |
| G.27 | **Gateway rate limiters** | 🟡 | ✅ | [TBD] | rate limiting |
| G.28 | **Gateway circuit breakers** | 🟡 | ✅ | [TBD] | circuit breaker pattern |
| G.29 | **Gateway retries** | 🟡 | ✅ | [TBD] | `RetryExecutor` with exponential backoff |
| G.30 | **Gateway fallbacks** | 🟢 | ✅ | [TBD] | `FallbackManager` with strategies |
| G.31 | **Gateway monitoring** | 🟡 | ✅ | [TBD] | Enhanced `HealthChecker` with metrics |
| G.32 | **Gateway alerting** | 🟡 | ✅ | [TBD] | `AlertManager` with severity levels |
| G.33 | **Gateway metrics collection** | 🟡 | ✅ | [TBD] | `MetricsRecorder` tracks all metrics |
| G.34 | **Gateway metrics aggregation** | 🟡 | ✅ | [TBD] | Per-platform aggregation |
| G.35 | **Gateway metrics reporting** | 🟡 | ✅ | [TBD] | JSON export and summary reports |
| G.36 | **Gateway metrics export** | 🟡 | ✅ | [TBD] | JSON format export |
| G.37 | **Gateway metrics visualization** | 🟡 | ✅ | [TBD] | JSON for frontend visualization |
| G.38 | **Gateway documentation** | 🟢 | ✅ | [TBD] | Module-level doc comments |

---

## Legend

- **🔴 Critical** — blocks production use
- **🟡 High** — important for core functionality
- **🟢 Medium** — nice-to-have
- **Status**: ✅ Done | ❌ Not Started

**Workflow:** Open issue → branch (`#<number>-<desc>`) → implement → PR → close issue.

## Task 8 Status

✅ **COMPLETED** - Pairing system implemented at `oben-gateway/src/pairing.rs`
- 8-char codes from unambiguous alphabet
- Cryptographic hashing (SHA-256)
- Rate limiting (1 request per 10 minutes per user)
- Max 3 pending codes per platform
- Lockout after 5 failed approval attempts (1 hour)
- All 13 unit tests passing

**Reference:** `hermes-agent/gateway/pairing.py`

## Task 11 Status

✅ **COMPLETED** - Gateway message routing implemented at `oben-gateway/src/gateway.rs`
- `MessageRouter` struct with platform-based message routing
- `route_message()` method routes incoming messages to correct platform adapter
- `register_adapter()` method registers adapters for routing
- All 81 gateway tests passing including 6 new message routing tests:
  - `test_message_router_unknown_platform`
  - `test_message_router_known_platform`
  - `test_message_router_multi_platform`
  - `test_message_router_with_thread`
  - `test_message_router_empty_count`
  - `test_message_router_list_platforms`
- Gateway integration tests for message routing via `handle_message()`
- Message routing is now the primary path instead of echo mode

**Reference:** `hermes-agent/gateway/gateway.py`

## Task 12 Status

✅ **COMPLETED** - Platform adapter registry implemented at `oben-gateway/src/platform.rs`

## Task 14 Status

✅ **COMPLETED** - Gateway shutdown implemented at `oben-gateway/src/gateway.rs`
- `shutdown()` async method that:
  - Aborts all platform listener tasks via `abort_handle.abort()`
  - Clears the internal handles map
  - Logs shutdown progress
- Test coverage: `test_gateway_shutdown_with_empty_handles` verifies clean shutdown
- All 93 gateway tests pass

**Reference:** `hermes-agent/gateway/run.py` (stop method)

## Task 12 Status

✅ **COMPLETED** - Platform adapter registry implemented at `oben-gateway/src/platform.rs`
- `PlatformEntry` struct with config, check_fn, validate_config, adapter_factory
- `PlatformAdapterRegistry` with register(), get(), create_adapter(), is_registered()
- Deferred loader support for lazy plugin module imports
- Config validation and dependency checking via callbacks
- Plugin vs builtin entry filtering via `plugin_entries()`
- All tests pass (6 platform adapter registry tests + 27 total platform tests)

**Reference:** `hermes-agent/gateway/platform_registry.py`

**Implementation details:**
- `PlatformEntry`: Config + callback fields (check_fn, validate_config, adapter_factory)
- `PlatformAdapterRegistry`: Thread-safe RwLock-based registry with async operations
- Deferred loading: `register_deferred()` and `resolve()` methods
- Adapter creation: `create_adapter()` validates deps/config before creating adapter
- Entry enumeration: `all_entries()` and `plugin_entries()` methods
