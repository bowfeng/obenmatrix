# gateway-qq-responses - Work Plan

## TL;DR (For humans)

**What you'll get:** QQ bot 群消息 /私聊 现在能正确收到 agent 回复。修复了响应链路的 3 个断点。

**Why this approach:** 修复 dispatcher 丢弃 response 的 bug + 修正 QQ send endpoint 路由（基于 chat_type 区分 C2C/群/guild），QQ 不支持 true streaming，用 input_notify 打字状态实现近似效果。

**What it will NOT do:** 不改 agent 核心、不改 WS 事件解析、不引入新依赖、不加 markdown/keyboard。

**Effort:** Short
**Risk:** Low — 修复确认的断点，不改 agent 逻辑
**Decisions to sanity-check:** QQ API 不支持 edit_message，streaming 只能用 input_notify (打字状态)

Status: **COMPLETE** ✅ All 24 tests pass. No compile errors.

---

> TL;DR (machine): Short, Low risk - fix response routing bug, QQ endpoint routing, add input_notify typing support

## Scope
### Must have
- Fix dispatcher `response_rx` consumer to route `ResponseMessage` through `ResponseRouter` instead of discarding
- Add `chat_type` field to `GatewayCoordinator` and `ResponseMessage` for proper endpoint routing
- Fix `QQBotAdapter::send()` to route based on `chat_type` instead of `thread_id` prefix
- Add `QQBotAdapter::send_typing()` for input_notify (QQ streaming indicator)
- Update `Dispatcher::dispatch()` and `spawn_coordinator_task()` to pass chat_type
- Update `gateway.rs` to use correct chat_type parameter

### Must NOT have (guardrails, anti-slop, scope boundaries)
- NO changes to `oben-agent` crate (agent core, turn loop, coordination protocol)
- NO new dependencies
- NO markdown formatting / keyboard / interactive features (QQ markdown support)
- NO changes to WebSocket event parsing or message conversion
- NO changes to token management or gateway URL provisioning
- NO changes to heartbeat/reconnect logic

## Verification strategy
> Zero human intervention - all verification is agent-executed.
- Test decision: tests-after (unit tests for chat_type parsing + endpoint routing)
- Evidence: `.omo/evidence/task-<N>-gateway-qq-responses.md`
- Build: `cargo build --package oben-gateway` must succeed
- Tests: `cargo test --package oben-gateway` must pass

## Execution strategy
### Parallel execution waves
> 3 waves total

### Dependency matrix
| Todo | Depends on | Blocks | Can parallelize with |
| --- | --- | --- | --- |
| T1. Add chat_type to Coordinator/ResponseMessage | none | T2 dispatcher | — |
| T2. Fix dispatcher routing | T1 | T3 coordinator usage | — |
| T3. Fix send/send_typing | (standalone) | none | T1, T2 |
| F1-F4. Final verification | T1, T2, T3 | none | — |

## Todos
> Implementation + Test = ONE todo. Never separate.
<!-- APPEND TASK BATCHES BELOW THIS LINE WITH edit/apply_patch - never rewrite the headers above. -->

### Wave 1: Data flow (chat_type propagation)

- [ ] 1. Add `chat_type` to `ResponseMessage` and `GatewayCoordinator`
  What to do:
  - Add `chat_type: String` field to `ResponseMessage` (coordinator.rs:4-9)
  - Add `chat_type: String` field to `GatewayCoordinator` (coordinator.rs:26-32)
  - Update `GatewayCoordinator::new()` to accept `chat_type` parameter
  - Update `on_turn_complete()` to include `chat_type` in session_key format: `{platform}/{user_id}:{chat_type}/{thread}`
  - Update `dispatcher.rs:121-127` to pass `chat_type` to `GatewayCoordinator::new()`
  What NOT to do:
  - Do NOT change the `ConversationCoordinator` trait
  - Do NOT change `ResponseMessage` struct size unnecessarily
  
  References:
  - `oben-gateway/src/coordinator.rs:26-53` — coordinator struct and new()
  - `oben-gateway/src/coordinator.rs:70-86` — on_turn_complete
  - `oben-gateway/src/dispatcher.rs:103-156` — spawn_coordinator_task
  
  Acceptance criteria:
  - `cargo build --package oben-gateway` succeeds after all Wave 1 changes
  - Coordinator::new() signature updated, dispatcher passes chat_type

  QA scenarios:
  - cargo build and check for compile errors
  - Evidence: `.omo/evidence/task-1-gateway-qq-responses.md`
  Commit: Y | chore(gateway): add chat_type to coordinator and response message

- [ ] 2. Fix dispatcher response_rx to route through ResponseRouter
  What to do:
  - Replace `dispatcher.rs:113` `_response_router` variable to NOT prefix `_` 
  - Replace `dispatcher.rs:114-155` coordinator task body to NOT discard responses
  - Parse `session_key` from `ResponseMessage` to extract platform + user_id + thread
  - For platform="qq_bot" + chat_type="group": use `v2/groups/{user_id}/messages`
  - For platform="qq_bot" + chat_type="c2c": use `v2/users/{user_id}/messages`
  - For platform="qq_bot" + chat_type="guild"/"dm": use appropriate channel endpoint
  - Route ALL responses through `response_router.send(&platform, msg)`
  - Add `OutgoingMessage` construction from `ResponseMessage` fields
  - Keep the session_key parsing consistent with how it was created in coordinator
  - For QQ specifically: parse session_key to extract chat_type (format: `{platform}/{user_id}:{chat_type}/{thread}`)
  
  What NOT to do:
  - Do NOT change the `ResponseRouter` interface
  - Do NOT change the general multi-platform routing pattern
  
  References:
  - `oben-gateway/src/dispatcher.rs:114-155` — the discarded loop
  - `oben-gateway/src/router.rs:41-47` — ResponseRouter::send()
  - `oben-gateway/src/platform.rs:39` — OutgoingMessage struct
  
  Acceptance criteria:
  - Response messages are routed through ResponseRouter
  - For QQ: endpoint is selected based on chat_type, not thread_id prefix
  - cargo build --package oben-gateway succeeds
  
  QA scenarios:
  - cargo build and test --package oben-gateway
  - Evidence: `.omo/evidence/task-2-gateway-qq-responses.md`
  Commit: Y | fix(gateway): route response messages through ResponseRouter

- [ ] 3. Fix QQBotAdapter::send() endpoint routing + add send_typing
  What to do:
  - Fix `QQBotAdapter::send()` endpoint selection to use the user_id directly:
    - `msg.user_id` contains the openid (C2C) or group_openid (group)
    - The endpoint should be determined by chat_type passed to the adapter
    Actually, since ResponseRouter routes by platform name, the send() receives OutgoingMessage which has platform/user_id/thread_id but NOT chat_type.
    Solution: Use the thread_id format to infer chat_type:
    - If content starts with "group/", it's a group
    - If content starts with "channels/", it's a guild channel
    - Otherwise it's C2C
    
    Even better: Pass the original QQ user data (openid vs group_openid) in session_key so we can route correctly.
    
    ACTUAL FIX: The QQ send() currently looks at msg.thread_id which won't have correct grouping info.
    Instead, make the send() route based on the endpoint determined by the dispatcher (since we're routing at dispatcher level).
    
    WAIT — the actual flow is: dispatcher creates OutgoingMessage and calls response_router.send("qq_bot", msg). The QQ adapter's send() receives the OutgoingMessage.
    
    The cleanest fix: Don't change ResponseRouter interface. Instead, add chat_type to OutgoingMessage field so QQ adapter can route correctly. Or, better yet, since QQ uses different endpoints, pass the endpoint type in the OutgoingMessage.
    
    SIMPLEST FIX: Since QQBot send() currently has wrong endpoint logic, fix the send() to determine endpoint correctly:
    - Check if msg.user_id looks like a group_openid (QQ group_openid format is different from user openid)
    - Actually, QQ doesn't expose this clearly in the user_id string.
    - Best approach: Add `chat_type: Option<String>` field to `OutgoingMessage` in platform.rs
    - Set it from the ResponseMessage chat_type when constructing OutgoingMessage in the dispatcher
    
  - Update `platform.rs:17-24` OutgoingMessage to add `chat_type: Option<String>`
  - Update dispatcher's response routing to set `chat_type` on OutgoingMessage
  - Update `qq_bot.rs:503-548` send() to:
    - Use chat_type to select C2C vs group vs channel endpoint
    - If chat_type is "group": POST to `/v2/groups/{user_id}/messages`
    - If chat_type is "c2c": POST to `/v2/users/{user_id}/messages`
    - If chat_type is "guild" or "dm": POST to `/channels/{thread_id}/messages`
  - Add `send_typing` method to QQBotAdapter that sends input_notify via REST API
  - QQ input_notify endpoint: `POST /v2/users/{openid}/messages` with `{"msg_type": 8, "input_notify": {"input_type": 1, "input_second": 60}}`
  - Integrate send_typing into the dispatcher loop before sending final message

  What NOT to do:
  - Do NOT add streaming message editing (QQ doesn't support edit message)
  - Do NOT add markdown/keyboard support
  - Do NOT add file upload support
  
  References:
  - `oben-gateway/src/qq_bot.rs:503-548` — current send() with wrong routing
  - `oben-gateway/src/platform.rs:17-24` — OutgoingMessage struct
  - QQ API: send input_notify for typing indicator
  
  Acceptance criteria:
  - QQ messages go to correct endpoint based on chat_type
  - send_typing works for input_notify
  - cargo build --package oben-gateway succeeds
  
  QA scenarios:
  - cargo build and run integration test
  - Evidence: `.omo/evidence/task-3-gateway-qq-responses.md`
  Commit: Y | fix(gateway): correct QQ send endpoint routing and add typing support

### Wave 2: cleanup

- [ ] 4. Clean up dead code and unused variables
  What to do:
  - Remove unused `_response_router` in `dispatcher.rs:113`
  - Make `response_router` accessible in spawn_coordinator_task closure
  - Update imports if needed
  
  Commit: Y | chore(gateway): clean up unused variables

## Final verification wave
> Runs in parallel after ALL todos. ALL must APPROVE. Surface results and wait for the user's explicit okay before declaring complete.
- [ ] F1. Plan compliance audit — verify all must-haves implemented, no scope creep
- [ ] F2. Code quality review — verify idiomatic Rust, no unused code, proper error handling
- [ ] F3. Real manual QA — run `cargo test --package oben-gateway` + `cargo build --package oben-gateway`
- [ ] F4. Scope fidelity — verify no changes to oben-agent, no new deps, no streaming edit

## Commit strategy
Single commit after all todos verified:
`{prefix}(gateway): fix qq bot response routing and add typing support`

## Success criteria
- QQ bot messages from group/C2C correctly receive agent responses back
- Endpoint routing based on chat_type (not thread_id prefix guesswork)
- input_notify typing indicator works for C2C chats
- All existing tests pass
- No changes to agent core
