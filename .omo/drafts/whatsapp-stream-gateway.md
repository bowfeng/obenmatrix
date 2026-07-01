---
slug: whatsapp-stream-gateway
status: drafting
intent: unclear
pending-action: write .omo/plans/whatsapp-stream-gateway.md
approach: Implement WhatsApp platform adapter for oben-gateway with HTTP bridge pattern, streaming support via progressive message edit, and all supporting config/coordinator infrastructure.
---

# Draft: whatsapp-stream-gateway

## Components (topology ledger)

| Component | Outcome | Status | Evidence |
|-----------|---------|--------|----------|
| WhatsAppBridge (Node.js) | HTTP bridge for WhatsApp communication | IN | scripts/whatsapp-bridge/ |
| WhatsAppAdapter (Rust) | PlatformAdapter impl for oben-gateway | IN | oben-gateway/src/whatsapp.rs |
| GatewayConfig (YAML) | WhatsApp config schema | IN | oben-config/src/config.rs |
| GatewayCoordinator | Streaming delta callback passthrough | IN | oben-gateway/src/coordinator.rs |
| Gateway start_blocking | WhatsApp adapter wiring | IN | oben-gateway/src/gateway.rs |

## Open assumptions (announced defaults)

| Assumption | Adopted Default | Rationale | Reversible? |
|------------|----------------|-----------|-------------|
| WhatsApp connection backend | Node.js HTTP bridge (Baileys-based) | Match hermes-agent pattern — no official WhatsApp bot API for personal accounts; bridge runs as child process | Yes — can swap bridge impl without changing adapter trait |
| Streaming strategy | Progressive `edit_message` via bridge | WhatsApp has no native streaming; hermes-agent uses the same pattern (post first message, progressively edit) | Yes — could add native draft streaming later |
| Message formatting | Markdown → WhatsApp italic/bold/code conversion | hermes-agent adapter already implements this; WhatsApp uses *bold* not **bold** | Yes — formatting rules are pluggable |
| Text message batching | 5s debounce / 10s split delay | hermes-agent uses these defaults for WhatsApp; prevents rapid-fire duplicate processing | Yes — configurable per-user |
| Auth/Pairing | Manual `hermes whatsapp` pairing flow | hermes-agent requires manual pairing via QR code; bridge stores creds.json | Yes — could add auto-pairing later |
| Test strategy | Integration-style (mock bridge server) | Real WhatsApp testing requires a paired account; mock bridge HTTP server is sufficient for unit/integration tests | — |

## Findings (cited - path:lines)

### 1. Current Gateway Architecture (`oben-gateway/src/`)
- **main.rs:132-214** — Entry point creates Agent, ToolRegistry, Dispatcher, Gateway. Currently only QQBot adapter is wired. Telegram/Discord/Slack configs exist but no adapters.
- **platform.rs:28-43** — `PlatformAdapter` trait: `name()`, `listen()`, `stop()`, `send()`, `health_check()`. `IncomingMessage` / `OutgoingMessage` structs.
- **dispatcher.rs:53-98** — Session-key-based routing. Creates `mpsc` channels per session, spawns `GatewayCoordinator` tasks.
- **coordinator.rs:26-108** — `GatewayCoordinator` implements `ConversationCoordinator`. `next_turn()` receives from `mpsc` receiver, `on_turn_complete()` sends response back via `mpsc` sender. NO streaming support currently.
- **gateway.rs:15-135** — `Gateway::start_blocking()` starts all platform adapters. Only QQBot is wired in `start_platforms()`.
- **router.rs:13-48** — `ResponseRouter` — hashmap of adapter names to adapters. `send()` looks up by name and delegates.

### 2. Agent/Coordinator Architecture (`oben-agent/src/coordinator/`)
- **mod.rs:151-168** — `ConversationCoordinator` trait: `on_loop_start()`, `next_turn()`, `on_turn_complete()`, `on_loop_end()`. Returns `bool` (continue/exit).
- **mod.rs:74-128** — `execute_turn_full()` calls `TurnExecutor::execute_turn_with_config()`, returns `Result<String>`.
- **mod.rs:163-164** — Comment: "For streaming mode, the coordinator should handle output formatting."

### 3. Streaming Infrastructure
- **turn_executor.rs:361-393** — `api_call_with_retry()` calls `transport.stream_chat()` with a `StreamDeltaCallback` that invokes `hooks.emit_stream_delta(delta)`.
- **hooks/runtime.rs:133-148** — `HookEngine::emit_stream_delta()` broadcasts to all `StreamingHooks`. Currently only TUI hook uses it.
- **hooks/kind.rs:281-290** — `StreamingHooks` trait: `on_stream_delta()`, `on_thinking()`, `on_reasoning()`, `on_interim_assistant()`.
- **chat_completions.rs:776-966** — `ChatCompletionsTransport::stream_chat()` parses SSE events, accumulates final text, calls `delta_callback(text)` per chunk.

### 4. Hermes-Agent WhatsApp Implementation (`hermes-agent/gateway/platforms/whatsapp.py`)
- **whatsapp.py:218-1234** — Full WhatsApp adapter using Node.js bridge pattern.
- **Bridge**: Node.js subprocess on configurable port (default 3000), communicates via HTTP JSON.
- **Connection**: Starts bridge → polls `/health` endpoint → waits for `status: connected`. Supports pre-existing bridge.
- **Message receipt**: `_poll_messages()` long-polls `/messages` endpoint every 1s via `aiohttp`.
- **Message send**: `send()` → formats markdown → `truncate_message()` → POST `/send` with `chatId` + `message`. Chunked with 0.3s delay.
- **Streaming**: `edit_message()` → POST `/edit` with `chatId`, `messageId`, `message`. Gateway stream consumer progressively edits.
- **Media**: `_send_media_to_bridge()` → POST `/send-media`. Supports image, video, voice, document.
- **Text batching**: 5s delay / 10s split delay debounce.
- **Formatting**: `format_message()` converts **bold**→*bold*, ~~strikethrough~~→~strikethrough~, headers→bold, links→flat.
- **Policies**: `dm_policy`, `group_policy`, allowlists, mention patterns, broadcast chat filtering.
- **Config**: `PlatformConfig` with extra fields (bridge_port, session_path, dm_policy, etc.).

### 5. Config Schema (`oben-config/src/config.rs`)
- **config.rs:314-326** — `GatewayConfig` has `whatsapp: Option<PlatformConfig>`. `PlatformConfig` only has `enabled` + `token`. Too primitive for WhatsApp.
- **QQBotConfig** is a much richer example with all fields inline.

### 6. Ironclaw (`ironclaw/`)
- `StreamingMode` enum (None/SSE/WebSocket/LongPoll) in `crates/ironclaw_host_api/src/ingress.rs:532-539`. Not directly relevant — this is about ingress policies, not WhatsApp.

## Decisions (with rationale)

### D1: Add `WhatsAppConfig` to `oben-config` — NOT reuse generic `PlatformConfig`
Rationale: WhatsApp needs many fields (bridge_port, session_path, dm_policy, group_policy, etc.). Generic `PlatformConfig` only has `enabled` + `token`. Follow QQBot example.

### D2: Stream via coordinator-level delta callback, NOT hook-only
Rationale: The `HookEngine::emit_stream_delta()` path exists but is internal to the agent. The coordinator needs a direct callback to forward deltas to the platform for progressive rendering. Add `stream_callback: Option<Arc<dyn StreamCallback>>` to `GatewayCoordinator`. This mirrors the hermes-agent approach where the consumer (GatewayStreamConsumer) receives deltas and progressively renders them.

### D3: Implement WhatsApp as HTTP bridge (NOT native WebSocket)
Rationale: hermes-agent uses HTTP bridge pattern with a Node.js subprocess. It's more robust, easier to debug, and the bridge handles WhatsApp protocol complexity (Baileys). The Rust adapter talks HTTP to localhost.

### D4: Streaming = progressive edit_message (NOT native streaming)
Rationale: WhatsApp doesn't support native streaming. The bridge exposes `/edit` endpoint. Adapter posts first message, then progressively edits it. This matches hermes-agent exactly.

### D5: Build Node.js bridge from scratch (NOT reuse hermes-agent bridge)
Rationale: hermes-agent's bridge is a JS file in `scripts/whatsapp-bridge/`. We should create a similar but self-contained bridge for oben-alien that supports the same API.

### D6: Text batch debouncing in Rust adapter (like hermes-agent)
Rationale: WhatsApp often delivers burst messages. Implement same 5s/10s debounce in Rust to prevent duplicate agent invocations.

## Scope IN
1. WhatsApp `PlatformConfig` / `WhatsAppConfig` in `oben-config`  
2. Node.js WhatsApp bridge (baileys-based HTTP server)  
3. `WhatsAppAdapter` Rust impl of `PlatformAdapter`  
4. Streaming: progressive `edit_message`, typing indicator, message chunking (4096 char)  
5. Text formatting: Markdown → WhatsApp syntax  
6. Text message debouncing/batching  
7. Gateway coordinator streaming callback support  
8. Gateway `start_blocking()` wiring for WhatsApp  
9. Config.yaml documentation example  
10. Unit + integration tests (mock bridge)

## Scope OUT (Must NOT have)
- WhatsApp media sending (image/video/audio/document) — future phase
- WhatsApp group admin features (promote, demote, mute)
- Voice note / audio message support
- Automatic pairing (QR code display) — require manual pre-pairing
- Business API (WhatsApp Cloud API) — only bridge mode
- End-to-end encryption handling — delegated to Node bridge
- WhatsApp status/story processing

## Open questions
- **None** — all resolved by defaults above. User can veto any default in the TL;DR.

## Approval gate
status: drafting
pending-action: present plan, await approval
