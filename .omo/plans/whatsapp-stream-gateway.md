# whatsapp-stream-gateway - Work Plan

## TL;DR (For humans)
<!-- Fill this LAST, after the detailed plan below is written, so it summarizes the REAL plan. -->
<!-- Plain English for a non-engineer: NO file paths, NO todo numbers, NO wave/agent/tool names. -->

**What you'll get:** A working WhatsApp bot integration that lets users chat with your AI agent through WhatsApp — they send a message, the agent thinks and responds in real time with streaming text (first word appears instantly, then the message progressively updates until complete). Supports DMs and group chats with configurable access controls.

**Why this approach:** WhatsApp has no official bot API for personal accounts, so we use a Node.js bridge process (same proven pattern as Hermes-Agent) that connects to WhatsApp via Baileys and exposes a simple HTTP API. The Rust gateway talks to this bridge over HTTP. Streaming is done by sending the first message, then progressively editing it — WhatsApp doesn't have native streaming, but this gives the same UX.

**What it will NOT do:** This plan does NOT include media sending (photos, videos, voice notes, documents), automatic QR-code pairing (pre-pairing is required), or WhatsApp Business API support. Those are explicitly deferred.

**Effort:** Medium
**Risk:** Medium - depends on Node.js bridge stability and WhatsApp's unpredictable message delivery patterns
**Decisions I made for you:** (1) Node.js HTTP bridge using Baileys — same proven approach as Hermes-Agent, (2) progressive message editing for streaming — not native streaming since WhatsApp doesn't support it, (3) 5-second text debounce to prevent duplicate processing of burst messages, (4) Markdown-to-WhatsApp formatting conversion, (5) full mock-based integration tests without requiring real WhatsApp credentials. You can veto any of these.

Your next move: approve to proceed, or request a high-accuracy plan review. Full execution detail follows below.

---

> TL;DR (machine): Medium effort, Medium risk, 8 todos + 4 verification — WhatsApp adapter + Node bridge + streaming + tests

## Scope
### Must have
1. WhatsAppConfig with all fields (bridge_port, session_path, policies, formatting options)
2. Node.js WhatsApp Bridge — HTTP server on configurable port, handles WhatsApp connection via Baileys, exposes /health, /messages, /send, /edit, /typing endpoints
3. WhatsAppAdapter — Rust `PlatformAdapter` impl, HTTP client to bridge, message polling, send/receive flow
4. Streaming — progressive `edit_message` to show response as it arrives, typing indicator
5. Text formatting — Markdown → WhatsApp syntax (*bold*, _italic_, ~strikethrough~, ```code```, links)  
6. Message chunking — 4096 char limit, never split inside code blocks
7. Text debouncing — 5s batch delay, 10s split delay
8. Gateway lifecycle wiring — adapter registered, starts with gateway, stops cleanly
9. Unit and integration tests with mock bridge server

### Must NOT have (guardrails, anti-slop, scope boundaries)
- No media sending (images, video, voice, documents)
- No WhatsApp Business API (Cloud API)
- No auto-pairing / QR code handling
- No group admin operations (promote, demote, mute)
- No voice note support
- No status/story processing
- No end-to-end encryption handling (delegated to bridge)
- No changes to existing QQBot adapter or other platform adapters
- No changes to ConversationCoordinator trait
- No breaking changes to execute_turn_full API

## Verification strategy
> Zero human intervention - all verification is agent-executed.
- Test decision: TDD for core adapter logic, tests-after for integration (bridge mock is complex)
- Evidence: .omo/evidence/task-<N>-whatsapp-stream-gateway.log for build/test output

## Execution strategy
### Parallel execution waves
Wave 1 (config + bridge): #1 WhatsAppConfig, #2 Node.js Bridge (independent)
Wave 2 (Rust core): #3 WhatsAppAdapter, #4 StreamCallback, #5 Gateway wiring (can parallelize)  
Wave 3 (streaming + tests): #6 Streaming/editing/formatting, #7 Config docs, #8 Integration tests

### Dependency matrix
| Todo | Depends on | Blocks | Can parallelize with |
| --- | --- | --- | --- |
| #1 Config | — | #2 bridge needs field names | #2 bridge |
| #2 Bridge | #1 field names (for config docs) | #3 adapter needs API contract | #1 |
| #3 Adapter | #2 bridge API contract | #6 streaming | #4, #5 |
| #4 StreamCallback | — | #6 streaming | #3, #5 |
| #5 Gateway wiring | #3 adapter | #6 streaming | #4 |
| #6 Streaming | #3, #4, #5 | — | #7 |
| #7 Config docs | — | — | #6 |
| #8 Integration tests | #2, #3, #6 | — | standalone |

## Todos
> Implementation + Test = ONE todo. Never separate.
<!-- APPEND TASK BATCHES BELOW THIS LINE WITH edit/apply_patch - never rewrite the headers above. -->

### Wave 1: Config + Node.js Bridge Foundation
- [ ] 1. Add WhatsAppConfig to oben-config
  What to do / Must NOT do: Create `WhatsAppConfig` struct in `oben-config/src/config.rs` with fields: enabled, bridge_port(3000), bridge_script, session_path, dm_policy("open"), group_policy("open"), allow_from, group_allow_from, reply_prefix, mention_patterns, text_batch_delay_seconds(5.0), text_batch_split_delay_seconds(10.0). Add `whatsapp: Option<WhatsAppConfig>` to `GatewayConfig`. Replace placeholder `whatsapp: Option<PlatformConfig>` with the new field. Must NOT remove any existing gateway config fields.
  Parallelization: Wave 1
  References: oben-config/src/config.rs:314-326, hermes-agent/gateway/platforms/whatsapp.py:251-298
  Acceptance criteria: `cargo test -p oben-config --lib` passes. `GatewayConfig` deserializes YAML with whatsapp section.
  QA scenarios: happy - parse config with all whatsapp fields; failure - parse missing required fields gets defaults
  Commit: Y | feat(config): add WhatsAppConfig struct to gateway config
  Blocks: Wave 2 (GatewayAdapter wiring)

- [ ] 2. Build Node.js WhatsApp Bridge (HTTP server)
  What to do / Must NOT do: Create `scripts/whatsapp-bridge/` with `package.json` (baileys + express), `bridge.ts`, and `tsconfig.json`. Bridge must expose: GET /health (returns {status: "connecting"|"connected"}), GET /messages (returns unread messages as JSON array), POST /send (chatId + message → messageId), POST /edit (chatId + messageId + message → success), POST /typing (chatId → shows typing). Bridge uses baileys to connect to WhatsApp, runs as HTTP server on configurable port. Must NOT implement media sending in this wave. Must NOT require manual changes — auto-installs npm dependencies if node_modules missing.
  Parallelization: Wave 1
  References: hermes-agent/gateway/platforms/whatsapp.py:531-760 (bridge lifecycle), hermes-agent/gateway/platforms/whatsapp.py:112-203 (send/receive pattern)
  Acceptance criteria: `npm install` succeeds. Bridge starts on configurable port. Health endpoint responds. Message send/receive endpoints return 200 with mock data.
  QA scenarios: happy - start bridge, poll health, send message, edit message; failure - bridge on occupied port, malformed request body
  Commit: Y | feat(bridge): add Node.js WhatsApp HTTP bridge with baileys
  Blocks: Wave 2 (Rust adapter depends on bridge API contract)

### Wave 2: Rust Adapter + Coordinator Streaming
- [ ] 3. Implement WhatsAppAdapter (PlatformAdapter impl)
  What to do / Must NOT do: Create `oben-gateway/src/whatsapp.rs` with `WhatsAppAdapter` implementing `PlatformAdapter`. Use `reqwest`/`tokio` for HTTP calls to bridge (localhost bridge_port). Methods: `name()` → "whatsapp", `listen()` → spawn `_poll_loop()` + `_check_bridge_exit()` background tasks, `stop()` → abort background tasks, `send()` → POST to bridge `/send` with formatted message, `health_check()` → GET bridge `/health`. Must handle bridge lifecycle: process started/killed, health polling, stale process cleanup. Must NOT depend on baileys directly — only HTTP to localhost bridge. Must NOT implement media sending. Must NOT support typing indicator yet.
  Parallelization: Wave 2
  References: oben-gateway/src/platform.rs:28-48 (PlatformAdapter trait), oben-gateway/src/qq_bot.rs:406-553 (existing adapter pattern), hermes-agent/gateway/platforms/whatsapp.py:811-970 (send formatting and chunking)
  Acceptance criteria: `cargo test -p oben-gateway --lib` passes. Adapter compiles. Mock tests verify send/receive flow.
  QA scenarios: happy - adapter connects, receives message, sends response; failure - bridge unreachable, send fails with HTTP error
  Commit: Y | feat(gateway): implement WhatsAppAdapter PlatformAdapter

- [ ] 4. Add streaming callback support to GatewayCoordinator
  What to do / Must NOT do: Modify `GatewayCoordinator` in `oben-gateway/src/coordinator.rs`. Add optional `stream_tx: Option<tokio::sync::mpsc::UnboundedSender<String>>` field. Modify constructor to accept optional stream channel. In coordinator's run loop (currently in `spawn_coordinator_task` in dispatcher.rs), wrap the agent turn to also subscribe to stream deltas: inject a `StreamingHook` into the `HookEngine` that forwards deltas to the stream channel. In `run()` (oben-agent/src/agent.rs:150-204), after turn starts, recv on stream_rx and forward each delta to the platform adapter's `stream_send()` method. Must NOT change the `ConversationCoordinator` trait. Must NOT affect non-gateway coordinators (CLI/TUI).
  Parallelization: Wave 2
  References: oben-gateway/src/coordinator.rs:26-108 (current coordinator), oben-agent/src/coordinator/mod.rs:74-128 (execute_turn_full), oben-agent/src/hooks/kind.rs:281-290 (StreamingHooks), oben-agent/src/hooks/runtime.rs:133-148 (emit_stream_delta), oben-gateway/src/dispatcher.rs:100-156 (spawn_coordinator_task)
  Acceptance criteria: `cargo build -p oben-gateway` succeeds. Streaming path compiles and forwards deltas when HookEngine streaming hooks are registered. Non-streaming paths unchanged.
  QA scenarios: happy - streaming on, deltas reach platform; failure - no streaming hook registered, deltas silently skipped; failure - stream channel closed doesn't crash agent
  Commit: Y | feat(gateway): add streaming callback passthrough in GatewayCoordinator
  Blocks: Flow: Wave 3 (end-to-end streaming)

- [ ] 5. Wire WhatsApp into Gateway start_blocking + ResponseRouter
  What to do / Must NOT do: Modify `oben-gateway/src/gateway.rs` `start_platforms()` to check for `gateway_config.whatsapp` and create `WhatsAppAdapter`. Add adapter registration to `ResponseRouter`. Modify `oben-gateway/src/main.rs` to pass WhatsApp config. Must NOT remove existing QQBot wiring. Must NOT change Gateway struct fields.
  Parallelization: Wave 2
  References: oben-gateway/src/gateway.rs:86-124 (start_platforms), oben-gateway/src/main.rs:179-203 (QQBot registration pattern)
  Acceptance criteria: Gateway compiles. WhatsApp adapter registers if enabled in config. ResponseRouter can send through WhatsApp.
  QA scenarios: happy - enabled WhatsApp config starts adapter; failure - disabled config skips adapter; failure - missing bridge port gets error
  Commit: Y | feat(gateway): wire WhatsApp adapter into Gateway lifecycle

### Wave 3: Streaming, Formatting, Debouncing, Tests
- [ ] 6. Implement progressive streaming (edit_message) + formatting + debouncing
  What to do / Must NOT do: In `WhatsAppAdapter::send()`, first send initial message via bridge `/send`, store `message_id`. In the streaming callback (from coordinator), progressively call bridge `/edit` endpoint with accumulated text. Implement `format_message()` — convert **bold**→*bold*, ~~strike~~→~strike~, `inline code` → keep, ```fenced code``` → keep (protect with placeholder during conversion), headers → bold, links: [text](url) → text (url). Implement message chunking (4096 char limit, never split inside ``` or `). Implement text debouncing: batch incoming messages within 5s window, concatenate, flush to dispatcher. Must NOT change the bridge API contract. Must NOT send media.
  Parallelization: Wave 3 | Blocked by: Wave 2 (adapter + coordinator streaming)
  References: hermes-agent/gateway/platforms/whatsapp.py:854-909 (format_message), hermes-agent/gateway/platforms/whatsapp.py:117-203 (truncation/chunking), hermes-agent/gateway/platforms/whatsapp.py:1116-1137 (typing), hermes-agent/gateway/platforms/whatsapp.py:1201-1234 (text batching)
  Acceptance criteria: Long messages stream progressively (first token appears immediately, final text arrives via edit). Formatting conversion is correct for common markdown. Debouncing prevents duplicate processing within 5s window.
  QA scenarios: happy - stream "Hello\n\nWorld" progressively, format markdown links, debounce 3 messages in 3s; failure - message over 4096 chars splits at code boundary; failure - rapid-fire messages merge correctly
  Commit: Y | feat(gateway): streaming edit_message, markdown→WhatsApp formatting, text debouncing

- [ ] 7. Add config.yaml documentation + example
  What to do / Must NOT do: Add WhatsApp example config block to config documentation (README.md or a dedicated config example file). Show complete gateway config with whatsapp section including all fields with comments. Must NOT modify existing config defaults.
  Parallelization: Wave 3 | Independent
  References: oben-config/src/config.rs:314-326 (final field names), hermes-agent examples
  Acceptance criteria: Example config is valid YAML, all fields documented with defaults.
  QA scenarios: happy - example YAML parses without errors
  Commit: N | docs: add WhatsApp gateway configuration example (no code change)

- [ ] 8. Integration tests — mock bridge + full flow
  What to do / Must NOT do: Create `oben-gateway/tests/whatsapp_integration.rs`. Start a mock bridge server (in-process HTTP server mimicking the Node.js bridge). Test: (a) full message flow: incoming message → dispatcher → coordinator → agent → platform response, (b) streaming: mock bridge sends incremental text, adapter progressively edits message, (c) error handling: bridge unavailable, send fails, (d) text batching: send 3 messages within 5s, verify only 1 agent invocation, (e) message formatting: verify markdown→WhatsApp conversion. Must NOT require real WhatsApp credentials. Must NOT start a real Node.js process.
  Parallelization: Wave 3 | Independent of #6 (test both alongside implementation)
  References: oben-gateway/src/platform.rs:50-178 (existing test adapter pattern), hermes-agent/tests/gateway/test_whatsapp_text_batching.py, hermes-agent/tests/gateway/test_whatsapp_reply_prefix.py
  Acceptance criteria: All integration tests pass. `cargo test -p oben-gateway --test whatsapp_integration` succeeds. Coverage for happy + failure paths across all features.
  QA scenarios: happy - full flow with mock bridge, streaming works; failure - bridge returns 500, adapter handles gracefully; failure - malformed bridge response doesn't panic
  Commit: Y | test(gateway): integration tests for WhatsApp adapter with mock bridge

## Final verification wave
> Runs in parallel after ALL todos. ALL must APPROVE. Surface results and wait for the user's explicit okay before declaring complete.
- [ ] F1. Plan compliance audit
- [ ] F2. Code quality review
- [ ] F3. Real manual QA
- [ ] F4. Scope fidelity

## Commit strategy
One atomic commit per todo item. PR title: `feat(gateway): whatsapp streaming adapter with bridge and progressive editing`

## Success criteria
1. `cargo test -p oben-gateway` passes (all existing + new tests)
2. `cargo build -p oben-gateway` succeeds
3. WhatsApp config section recognized in config.yaml
4. Bridge starts, adapter connects, sends/receives messages
5. Streaming: first token visible immediately, text progressively updates via edit_message
6. Message formatting correctly converts markdown to WhatsApp syntax
7. Text debouncing works (messages within 5s batched)
8. No regressions in existing QQBot or other gateway functionality
