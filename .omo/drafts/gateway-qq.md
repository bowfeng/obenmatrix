---
slug: gateway-qq
status: awaiting-approval
intent: clear
pending-action: write .omo/plans/gateway-qq.md
approach: Implement full multi-platform gateway with Dispatcher per-user routing, GatewayCoordinator + Agent::run(), and ResponseRouter. One task at a time — start with Dispatcher.
---

# Draft: gateway-qq

## Components (topology ledger)
| id | outcome | status | evidence |
|----|---------|--------|----------|
| C1: Dispatcher | Routes incoming_msg → per-user TaskScheduler. SessionKey = platform:user_id:thread_id. Spawns coordinator+agent task on first message from a session. | active | dispatcher.rs (NEW) |
| C2: ResponseRouter | Agent reply → platform adapter (by platform name). HashMap<name, adapter>. | active | router.rs (NEW) |
| C3: GatewayCoordinator: ConversationCoordinator | next_turn() = msg_rx.recv(), on_turn_complete() = forward_response_to_router. | active | coordinator.rs (NEW) |
| C4: QQ adapter → dispatcher | dispatch_message() → dispatcher.dispatch(). Response back to coordinator via channel. | active | qq_bot.rs (MOD) |
| C5: Gateway passes AppConfig to Dispatcher | Allows Dispatcher to create Agents via Agent::new(config, prompt, tools). | active | gateway.rs (MOD) |

## Open assumptions (announced defaults)
| assumption | adopted default | rationale | reversible? |
|---|---|---|---|
| GatewayCoordinator | Implements ConversationCoordinator. next_turn() reads from Receiver, sends None on disconnect | Pattern matches CliCoordinator/stdin and TuiCoordinator/channel | Yes — if we need async handling of turn completion later |
| Dispatcher session key | format!("{}:{}/{}", platform, user_id, thread_id.unwrap_or("global")) | Stable per channel | Can vary if we need user + session separation |
| ResponseRouter | Stores adapters by platform name, looks up adapter by platform name | Simple HashMap lookup O(1) | Add LRU cache if needed |
| Per-user agent | One Agent per session (QQ user/channel). Agent::new(config, prompt, tools) | Reuses existing Agent constructor | Configurable if needed |
| Mention stripping | Strip QQ <at appid=.../> mention prefix in event_to_incoming | QQ API includes mention in raw content; agent shouldn't see @BotName | Configurable |

## Finding Summary
1. Current Gateway: DBSessionManager + ToolRegistry only. Missing: AppConfig, TransportProvider, ContextWindowManager.
2. Current GatewayCoordinator stub: QqMessageHandler::handle() returns Ok(None), no agent processing.
3. QQ adapter already has WS lifecycle, heartbeat, READY handling, heartbeat acks, reconnect loop.
4. QQ adapter send() uses REST API routes: group/channel/DM. Already tested.
5. SharedState: All fields use Arc<Mutex<Option<T>>>. Clone is cheap (Arc increment).
6. Existing dispatch_message: calls self.handler.lock() → handler(incoming_msg) → take_pending_response() (no-op).
7. Event types handled: C2cMessageCreate, GroupAtMessageCreate, AtMessageCreate, DirectMessageCreate.

## Scope IN
1. Dispatcher — routes incoming_message to per-user coordinator via async channel
2. ResponseRouter — agent reply → platform adapter send()
3. GatewayCoordinator: ConversationCoordinator — drives Agent::run() per-user
4. QQ adapter wiring: QqEventDispatcher → dispatcher
5. Platform trait: MessageHandler returns Ok(None) (response via Channel)
6. Mention stripping: Strip QQ mention prefix in event_to_incoming

## Scope OUT (Must NOT have)
1. Telegram/Discord/Slack adapter implementations
2. Streaming responses
3. Image/attachment handling
4. Per-session agent routing (single per user)
5. CLI binary wiring

## Plan Status
status: approved
Approvals: User approved the architecture in 2 turns.
Next: Execute dispatcher task.
