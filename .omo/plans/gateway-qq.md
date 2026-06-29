# gateway-qq - Work Plan

## TL;DR (For humans)

**What you'll get:** The QQ Bot gateway adapter wired into a full multi-platform message routing system. Every incoming message from any platform flows through a central Dispatcher, which routes it to a per-user conversation session backed by the existing agent agent loop. Responses flow back through a ResponseRouter to the correct platform adapter.

**Why this approach:** We use the existing `ConversationCoordinator` pattern (`CliCoordinator` / `TuiCoordinator`) and implement `GatewayCoordinator` identically. Agent owns all business logic (tool dispatch, retries, context compression). Gateway only provides the I/O layer. This is the same topology as Hermes-Agent's `_handle_message` / `_run_agent` flow.

**What it will NOT do:** No Telegram/Discord/Slack adapter implementations (only the gateway infrastructure), no streaming responses, no image/file handling.

**Effort:** Large
**Risk:** Medium — changes to dispatcher and gateway are core infrastructure
**Decisions to sanity-check:** Per-user `Agent::new(config, prompt, tools)` in dispatcher vs pre-created transport; `GatewayCoordinator` uses `Receiver<IncomingMessage>` with `next_turn()` blocking on receive

## Scope
### Must have
1. `Dispatcher` — routes inbound messages to per-user coordinator tasks
2. `ResponseRouter` — sends agent responses back to correct platform adapter
3. `GatewayCoordinator: ConversationCoordinator` — drives `Agent::run()` per-user
4. QQ adapter → dispatcher wiring and response back-channel
5. Platform trait: `MessageHandler` returns `Ok(None)` (response goes through channel, not return value)

### Must NOT have (guardrails, anti-slop, scope boundaries)
- Telegram/Discord/Slack adapter implementations (only dispatcher infrastructure)
- Streaming responses
- Image/file handling
- Other platform implementations

## Verification strategy
> Zero human intervention — all verification is agent-executed.
- Test decision: **tests-after** + `cargo test --package oben-gateway --lib`
- Evidence: `lsp_diagnostics` on project directory, `cargo test --package oben-gateway`
1. Unit test: `Dispatcher::dispatch_message` routes to existing user queue
2. Unit test: `GatewayCoordinator::handle` creates response channel and calls `Agent::run`
3. Integration test: Full pipeline `PlatformAdapter::send` → `Dispatcher::dispatch` → `GatewayCoordinator::handle` → response back to platform

## Execution strategy
### Wave 1: Dispatcher (foundation — blocks all)
**Solo task.** Dispatcher is the central routing service. Everything depends on its interface.
- `dispatcher.rs` — NEW: `Dispatcher` struct, `dispatch_message()`, `get_or_spawn_user()` spawns `GatewayCoordinator` + `Agent` task

### Wave 2: Coordinator + Router (depends on dispatcher)
**Parallel.** Coordinator needs dispatcher's interface; router needs platform trait.
- `coordinator.rs` — NEW: `GatewayCoordinator: ConversationCoordinator` impl
- `router.rs` — NEW: `ResponseRouter` (sends to platform adapter by platform name)

### Wave 3: QQ adapter + Gateway wiring (depends on coordinator + router)
**Single task.** Must wire dispatcher → QQ adapter → coordinator → router together.
- `qq_bot.rs` — MODIFY: `SharedState` adds dispatcher ref, `dispatch_message` routes to dispatcher instead of `MessageHandler`
- `gateway.rs` — MODIFY: creates dispatcher in constructor, passes to QQ adapter
- `lib.rs` — MODIFY: export new modules
- `Cargo.toml` — MODIFY: add `oben_agent` dev-dependency

### Wave 4: Tests (depends on Wave 3)
**Single task.** Unit and integration tests for coordinator and gateway wiring.

## Todos
> Implementation + Test = ONE todo. Never separate.

<!-- APPEND TASK BATCHES BELOW THIS LINE WITH edit/apply_patch - never rewrite the headers above. -->

- [x] 1. Dispatcher — routes inbound messages to per-user coordinator tasks
  What to do:
  - Create `dispatcher.rs` with `Dispatcher` struct.
  - `Dispatcher::new(app_config, platform_adapter_registry)` takes config and registry. `PlatformAdapterRegistry` stores adapters for response routing by platform name.
  - `Dispatcher::dispatch_message(msg: IncomingMessage)`:
    1. Extract `session_key = format!("{}:{}/{}", msg.platform, msg.user_id, msg.thread_id.as_deref().unwrap_or("global"))`
    2. If session_key not in `HashMap`, create task:
       a. Create `Agent::new(app_config, system_prompt, tools)`
       b. Create `GatewayCoordinator` with `mpsc::channel(64)` for messages + responses
       c. Spawn `Agent::run(Arc<Mutex<agent>>, coordinator)` task
       d. Store `Sender` in hashmap
    3. Call `user_sender.send(msg).await` (or drop if disconnected)
  - Must import `oben_agent` for `Agent` and `ConversationCoordinator` trait.
  - Must be `Send + Sync`.

  Parallelization: Wave 1 — solo, blocks everything else
  References:
  - `agent.rs:61-72` — Agent struct fields: transport, tools, context_window_manager, session_manager, config, interrupt_state, system_prompt
  - `agent.rs:74-111` — `Agent::new(config, system_prompt, tools)` constructor — creates transport (from config.model), session_manager (SessionStore), hooks, context_manager (BuiltinContextWindowManager)
  - `agent.rs:150-203` — `Agent::run(agent, mut coordinator)`: calls `coordinator.on_loop_start()`, loops: `coordinator.next_turn()` → `me.turn()` → `coordinator.on_turn_complete()` → check max_iterations
  - `coordinator/mod.rs:151-169` — `ConversationCoordinator` trait: `on_loop_start`, `next_turn() → Option<String>`, `on_turn_complete(response, count, success) → bool`, `on_loop_end(outcome)`
  - `coordinator/mod.rs:74-128` — `execute_turn_full` (called by `Agent::turn`) requires: `ContextWindowManager`, `TransportProvider`, `ToolRegistry`, `SessionManager`, session_id, Message, CallMode, ConversationConfig

  Acceptance criteria:
  - `cargo check` passes in `oben-gateway` crate
  - `Dispatcher` is constructible with `Dispatcher::new()`
  - `Dispatcher::dispatch_message(msg)` compiles with `IncomingMessage` input
  - `Cargo.toml` includes `oben_agent` dependency in dependencies section

  QA scenarios:
  - Happy: `cargo check --package oben-gateway` exits 0
  - Failure: `Dispatcher` struct missing fields causes `missing_field` error — fix
  - Evidence: `.omo/evidence/task-1-gateway-qq.md`

  Commit: Y | feat(gateway): add Dispatcher for message routing to per-user coordinator tasks

- [x] 2. GatewayCoordinator — ConversationCoordinator impl that drives Agent::run per-user
  What to do:
  - Create `coordinator.rs` with `GatewayCoordinator` struct.
  - Implement `ConversationCoordinator` trait for `GatewayCoordinator`:
    - `on_loop_start()`: no-op (can set up hooks if needed later)
    - `async fn next_turn() -> Option<String>`: self.msg_rx.recv().await or None if disconnected
    - `fn on_turn_complete(&mut self, response: &str, _msg_count: usize, success: bool) -> bool`:
      1. Build ResponseMessage: platform from IncomingMessage, user_id from IncomingMessage, thread_id from IncomingMessage, content = response
      2. self.response_tx.send(response_msg).await
      3. Return success (true = continue loop, false = exit loop/kill task)
    - `fn on_loop_end(&mut self, outcome: &ConversationResult)`: log outcome
  - `struct GatewayCoordinator { msg_rx: Receiver<IncomingMessage>, response_tx: Sender<ResponseMessage> }`
  - ResponseMessage = { platform: String, user_id: String, thread_id: Option<String>, content: String }

  Parallelization: Wave 2 — parallel with router.rs (no dependency between them)
  Blocked by: dispatcher.rs (must match Dispatcher's coordinator creation pattern)
  References:
  - `coordinator/mod.rs:151-169` — `ConversationCoordinator` trait: `on_loop_start`, `next_turn() → Option<String>`, `on_turn_complete(response, count, success) → bool`, `on_loop_end(&mut self, outcome: &ConversationResult)`
  - `coordinator/mod.rs:132-143` — `ConversationResult` enum: Exit, BudgetExhausted, Interrupted, GoalDone, Error(String)
  - `oben-cli/src/coordinator/cli.rs:81-149` — CliCoordinator impl: `next_turn` from stdin, `on_turn_complete` prints response
  - `oben-tui/src/coordinator/mod.rs:55-113` — TuiCoordinator impl: `next_turn` from channel, `on_turn_complete` sends to TUI panel

  Acceptance criteria:
  - `GatewayCoordinator` implements `ConversationCoordinator` trait
  - `next_turn()` blocks on `msg_rx.recv()`, returns `None` on channel close
  - `on_turn_complete()` forwards response to `response_tx`
  - Compiles with `above-gateway`: `cargo check --package oben-gateway`

  QA scenarios:
  - Happy: `cargo check --package oben-gateway` exits 0
  - Failure: missing `#[async_trait::async_trait]` on `ConversationCoordinator` impl — fix with attribute
  - Evidence: `.omo/evidence/task-2-gateway-qq.md`

  Commit: Y | feat(gateway): add GatewayCoordinator implementing ConversationCoordinator

- [x] 3. ResponseRouter — sends agent replies back to correct platform adapter
  What to do:
  - Create `router.rs` with `ResponseRouter` struct.
  - `ResponseRouter::new()` creates empty HashMap
  - `fn register(platform: String, adapter: Box<dyn PlatformAdapter>)` add to HashMap
  - `async fn send(msg: ResponseMessage)` → look up adapter by platform name, call `adapter.send(PlatformMessage { platform, user_id, thread_id, content })`. Return error if platform not found.
  - `ResponseMessage` = same struct as coordinator response (platform, user_id, thread_id, content)

  Parallelization: Wave 2 — parallel with coordinator.rs (no dependency between them)
  References:
  - `platform.rs:27-43` — `PlatformAdapter` trait: `send(&self, msg: OutgoingMessage) -> Result<()>`
  - `platform.rs:20-25` — `OutgoingMessage` struct

  Acceptance criteria:
  - `ResponseRouter::register("platform_name", adapter)` adds adapter
  - `ResponseRouter::send()` correctly routes to platform named adapter
  - Returns error if platform not found
  - `cargo check --package oben-gateway` exits 0

  QA scenarios:
  - Happy: register two adapters, send to each — both get delivered
  - Failure: send to unknown platform → `Err(anyhow!("no adapter for platform X"))` — verify error path
  - Evidence: `.omo/evidence/task-3-gateway-qq.md`

  Commit: Y | feat(gateway): add ResponseRouter for agent reply delivery

- [x] 4. QQ adapter + Gateway wiring — dispatcher into QQ adapter and gateway startup
  What to do:
  - `qq_bot.rs` — MODIFY:
    1. `SharedState` adds field `dispatcher: Arc<Dispatcher>` 
    2. `SharedState::new(config)` — add dispatcher parameter
    3. `dispatch_message(event_type, data)`:
       a. Convert to `IncomingMessage` via `event_to_incoming()` (line 389-436)
       b. Strip QQ mention prefix: remove `<at appid=XX/>` from content
       c. Call `self.dispatcher.dispatch_message(msg).await` (drop response)
    4. Remove `handler` and `send_tx` from SharedState (no longer needed with dispatcher pattern)

  - `GatewayCoordinator` (already in coordinator.rs) — update constructor:
    ```rust
    impl Dispatcher {
        fn spawn_coordinator_task(&self, session_key: String, platform: String, msg_rx: Receiver<IncomingMessage>, resp_tx: Sender<ResponseMessage>) {
            let adapter = self.registries.get(&platform)... // get adapter for response routing
            let coordinator = GatewayCoordinator::new(msg_rx, resp_tx);
            spawn(async move {
                let agent = Agent::new(app_config.clone(), system_prompt, tool_registry.clone()).await.unwrap();
                let agent = Arc::new(Mutex::new(agent));
                Agent::run(agent, coordinator).await
            });
        }
    }
    ```

  - `gateway.rs` — MODIFY:
    1. `Gateway` struct adds `dispatcher: Arc<Dispatcher>`
    2. `Gateway::new` takes `dispatcher: Arc<Dispatcher>` instead of just session_manager + tools
    3. In `start_platforms()`, pass dispatcher to QQ adapter via `QQBotAdapter::new(..., dispatcher.clone())`
    4. Pass app_config to dispatcher for spawning agents

  - `lib.rs` — MODIFY: add `pub mod dispatcher; pub mod router;` and `pub use...`
  - `Cargo.toml` — MODIFY: add `oben-agent = { path = "../oben-agent" }` in dependencies

  Parallelization: Wave 3 — single task (all files interdependent)
  Blocked by: dispatcher.rs, coordinator.rs, router.rs all compiled

  Acceptance criteria:
  - `cargo check --package oben-gateway` exits 0
  - `dispatch_message` routes to dispatcher instead of handler
  - Mention stripping in QQ adapter
  - QQ adapter receives dispatcher and uses it for routing

  QA scenarios:
  - Happy: event_to_incoming → Dispatcher::dispatch → GatewayCoordinator spawns Agent::run task
  - Failure: SharedState struct field mismatch — fix compile errors incrementally
  - Evidence: `.omo/evidence/task-4-gateway-qq.md`

  Commit: Y | feat(gateway): wire dispatcher into QQ adapter and gateway startup

Now let me write the plan file using apply_patch. I need to fill in the actual todos in the plan file.

<tool_call>
<function=edit>