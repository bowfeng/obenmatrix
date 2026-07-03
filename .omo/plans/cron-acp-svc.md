# cron-acp-svc - Work Plan

## TL;DR (For humans)

**What you'll get:** A cron daemon that can execute scheduled jobs in three modes: (1) simple — just runs `obenalien run -p` as before (no change), (2) daemon-agent — the daemon builds its own agent with a dedicated session and executes prompts natively using tokio, (3) gateway — the daemon POSTs prompts to a gateway HTTP endpoint for platform-based delivery.

**Why this approach:** Keeps JSON persistence and CLI compatibility intact (zero breaking changes to existing cron commands). The three-mode config gives users control over whether they want bare prompts, context-aware agent runs, or platform-connected delivery — without merging the daemon into the agent process.

**What it will NOT do:** No new database, no web UI, no WebSocket/SSE streaming, no auth on the daemon HTTP endpoint, no retry/dead-letter queues, no platform-specific routing logic.

**Effort:** Medium
**Risk:** Low — daemon-agent mode is straightforward in-process execution; gateway mode depends on adding one HTTP endpoint to the gateway.
**Decisions to sanity-check:** `delivery_mode: daemon-agent` builds a full Agent from AppConfig with transport + tools — this means the daemon has its own model config and will incur separate LLM costs during cron execution.

Your next move: approve, or run a high-accuracy review. Full execution detail follows below.

---

> TL;DR (machine): Medium effort, Low risk — HTTP submit endpoint + 3 delivery modes (simple/daemon-agent/gateway) + NudgeHook cron-job integration; 8 todos across 3 waves + 4 verification

## Scope
### Must have
1. `oben-cron` HTTP server with `/cron/submit` POST endpoint
2. `cron.delivery_mode` config field (`simple`, `daemon-agent`, `gateway`)
3. `daemon-agent` mode: build Agent from AppConfig, create session, execute via execute_turn_full
4. `gateway` mode: POST prompt to `oben-gateway` HTTP endpoint
5. `simple` mode: keep existing `obenalien run -p` subprocess path (no code change)
6. NudgeHook creates a one-shot cron job via HTTP POST to daemon `/cron/submit`
7. Config migration: add `cron.delivery_mode` to AppConfig
8. All existing JSON persistence, CLI subcommands, scheduling unchanged

### Must NOT have (guardrails, anti-slop, scope boundaries)
- No new database or schema migration
- No Web UI, no WebSocket/SSE, no auth
- No retry queue, no dead-letter queue, no message broker
- No platform-specific routing (gateway mode is passthrough)
- No changes to CronJob struct fields (backward compat only)
- No breaking changes to existing cron CLI subcommands
- No merge of cron daemon into agent process

## Verification strategy
> Zero human intervention - all verification is agent-executed.
- Test decision: tests-after (TDD discouraged for infra wiring; add integration tests after)
- Evidence: `.omo/evidence/task-{N}-cron-acp-svc.md`
- Verify: `cargo check --package oben-cron`, `cargo check --package oben-gateway`, `cargo check --workspace`

## Execution strategy
### Parallel execution waves
- Wave 1: Config migration + HTTP client (foundation) → 2 tasks (sequential)
- Wave 2: Daemon-agent mode + HTTP server → 3 tasks (can parallelize: HTTP server impl, daemon agent builder)
- Wave 3: Gateway mode + NudgeHook integration + CLI wiring → 3 tasks (sequential dependency: NudgeHook depends on server running)

### Dependency matrix
| Todo | Depends on | Blocks | Can parallelize with |
| --- | --- | --- | --- |
| T3 HTTP client | T1, T2 config | T5 daemon-agent | — |
| T4 NudgeHook cron submit | T3 client | T5 | — |
| T5 daemon-agent | T3 client | T7 | T6 HTTP server |
| T6 HTTP server | — | T7 | T5 daemon-agent |
| T7 server integration tests | T5, T6 | gate | — |
| T8 gateway mode | T1 config | F verification | — |

## Todos
> Implementation + Test = ONE todo. Never separate.
<!-- APPEND TASK BATCHES BELOW THIS LINE WITH edit/apply_patch - never rewrite the headers above. -->

- [ ] 1. Add `cron.delivery_mode` to AppConfig and above
  What to do / Must NOT do: Add `delivery_mode: DeliveryMode` (enum: Simple, DaemonAgent, Gateway) to AppConfig in `oben-config`. Ensure it de-serializes from YAML with default `DeliveryMode::Simple`. Must NOT rename existing fields.
  Parallelization: Wave 1 | Blocked by: nothing | Blocks: T2 (config must exist first)
  References: `oben-config/src/lib.rs:AppConfig`, `oben-cron/src/jobs.rs:91-119` (CronJob struct — for field add pattern)
  Acceptance criteria: `cargo check --package oben-config` passes; default DeliveryMode::Simple
  QA scenarios: happy — deserialize yaml with `delivery_mode: simple` → DeliveryMode::Simple; failure — missing field defaults to simple; Evidence `.omo/evidence/task-1-cron-acp-svc.md`
  Commit: Y | feat(config): add cron.delivery_mode to AppConfig

- [ ] 2. Add `CronSubmitRequest`/`CronSubmitResponse` to oben-cron with serde
  What to do / Must NOT do: Define HTTP request/response structs in `oben-cron/src/http.rs`. Request: `{ prompt: String, deliver_target: Option<DeliverTarget>, session_id: Option<String> }`. Response: `{ job_id: String, status: String }`. Must NOT add HTTP server logic yet.
  Parallelization: Wave 1 | Blocked by: T1 | Blocks: T3
  References: `oben-cron/src/jobs.rs:36-52` (DeliverTarget enum), `oben-cron/src/lib.rs:1-46` (existing module structure)
  Acceptance criteria: `cargo check --package oben-cron` passes; structs serialize/deserialize correctly
  QA scenarios: happy — serde roundtrip for request; failure — missing prompt field fails deserialization; Evidence `.omo/evidence/task-2-cron-acp-svc.md`
  Commit: Y | feat(cron): add HTTP request/response structs

- [ ] 3. Add reqwest HTTP client helper for submitting prompts to daemon
  What to do / Must NOT do: Add `CronClient` struct in `oben-cron/src/http.rs` with `async fn submit(&self, prompt: &str) -> Result<CronSubmitResponse>`. Uses `reqwest` (existing workspace dep). URL configurable via `OBEN_CRON_URL` env var or `oben_config`. Must NOT implement server endpoints.
  Parallelization: Wave 1 | Blocked by: T2 | Blocks: T4, T5, T6
  References: `Cargo.toml:33` (reqwest workspace dep), `oben-cron/src/http.rs` (new file), `oben-config/src/app_config.rs`
  Acceptance criteria: `cargo check --workspace` passes; HTTP client compiles with configurable base URL
  QA scenarios: happy — client builds with default URL `http://localhost:8790`; failure — invalid URL panics on client creation; Evidence `.omo/evidence/task-3-cron-acp-svc.md`
  Commit: Y | feat(cron): add HTTP client for daemon endpoint

- [ ] 4. Wire NudgeHook to create one-shot cron job via HTTP POST
  What to do / Must NOT do: In `oben-agent/src/hooks/runtime.rs`, NudgeHook must construct a `CronJob` with schedule="30m" (or use a one-shot duration like "5m"), call `CronClient::submit()` to register it on the daemon, and log success/failure. Must NOT change the existing `on_post_turn` trigger condition. Must NOT spawn subprocesses.
  Parallelization: Wave 1 | Blocked by: T3 | Blocks: nothing (no further deps)
  References: `oben-agent/src/hooks/runtime.rs:12-54` (NudgeHook), `oben-agent/src/hooks/kind.rs:32-34` (AgentInit trait), `oben-agent/src/agent.rs:671-768` (existing trigger_nudge for prompt template)
  Acceptance criteria: `cargo check --workspace` passes; NudgeHook::on_post_turn calls CronClient::submit if delivery_mode=DaemonAgent or Gateway
  QA scenarios: happy — NudgeHook hits HTTP endpoint with memory review prompt; failure — HTTP 503 logged as warning, turn completes normally; Evidence `.omo/evidence/task-4-cron-acp-svc.md`
  Commit: Y | feat(nudge): wire NudgeHook to cron daemon HTTP submit

- [ ] 5. Add delivery_mode dispatch in CronStore::advance_job
  What to do / Must NOT do: Add `delivery_mode: DeliveryMode` field to cron config. In `advance_job()`, branch on mode: Simple → existing subprocess, DaemonAgent → in-process agent execution, Gateway → HTTP POST to gateway endpoint. Must NOT change CronJob struct (delivery_mode is runtime config, not per-job). Must NOT write new output files (keep existing output_dir logic for simple mode only).
  Parallelization: Wave 2a | Blocked by: T3 | Blocks: T7
  References: `oben-cron/src/jobs.rs:500-552` (advance_job), `oben-cron/src/main.rs:67-68` (cron_exec_binary call path), `oben-agent/src/coordinator/mod.rs:23-71` (execute_turn for single turn)
  Acceptance criteria: `cargo check --package oben-cron` passes; advance_job dispatches to correct mode at compile time
  QA scenarios: happy — mode=DaemonAgent runs in-process, records output to output_dir; failure — Gateway mode with unreachable URL logs warning, marks job Error; Evidence `.omo/evidence/task-5-cron-acp-svc.md`
  Commit: Y | feat(cron): add delivery_mode dispatch in advance_job

- [ ] 6. Build in-process agent for daemon-agent delivery mode
  What to do / Must NOT do: Create `CronAgentRunner` in `oben-cron/src/agent_runner.rs`. Constructs a minimal Agent from `oben_config::AppConfig` (transport + tools + session manager), runs `execute_turn_full` with bounded max_iterations=16 (same as existing trigger_nudge), returns stdout string. Must NOT use the agent's `trigger_nudge` method (that creates sessions internally — we need bare execution). Must NOT share HTTP server state with the runner.
  Parallelization: Wave 2b | Blocked by: T3 | Blocks: T7
  References: `oben-agent/src/agent.rs:74-85` (AgentBuilder::new), `oben-agent/src/coordinator/mod.rs:23-71` (execute_turn_full), `oben-agent/src/agent.rs:734-747` (execute_turn_full call in trigger_nudge), `oben-config/src/lib.rs` (AppConfig fields needed for transport)
  Acceptance criteria: `cargo check --workspace` passes; CronAgentRunner::run(prompt) compiles and returns Result<String>
  QA scenarios: happy — daemon-agent mode executes prompt, returns LLM response; failure — LLM timeout returns Err, job marked Error; Evidence `.omo/evidence/task-6-cron-acp-svc.md`
  Commit: Y | feat(cron): add in-process agent runner for daemon-delivery mode

- [ ] 7. Implement HTTP server in oben-cron with /cron/submit endpoint
  What to do / Must NOT do: Add HTTP server to `oben-cron/src/server.rs` using `axum` (or `actix-web` — prefer axum for compatibility). Expose `POST /cron/submit` that accepts `CronSubmitRequest`, calls `CronJob::new()`, validates with `scan_cron_prompt()`, stores via `CronStore::create()`. Return `CronSubmitResponse` with job_id. Must NOT handle GET requests or other paths. Must NOT add auth middleware.
  Parallelization: Wave 2b | Blocked by: T2 | Blocks: T4 (NudgeHook depends on endpoint being available)
  References: `oben-cron/src/http.rs` (CronSubmitRequest struct), `oben-cron/src/jobs.rs:219-253` (scan_cron_prompt), `oben-cron/src/jobs.rs:363-368` (CronStore::create)
  Acceptance criteria: `cargo check --package oben-cron` passes; server compiles, starts on configurable port
  QA scenarios: happy — POST /cron/submit with valid prompt returns 201 with job_id; failure — POST with injection prompt returns 400; Evidence `.omo/evidence/task-7-cron-acp-svc.md`
  Commit: Y | feat(cron): add HTTP server with /cron/submit endpoint

- [ ] 8. Wire gateway HTTP endpoint for cron daemon → gateway mode
  What to do / Must NOT do: Add `POST /gateway/prompt` endpoint to `oben-gateway/src/router.rs` (or new `oben-gateway/src/cron_endpoint.rs`). Accepts `{ prompt: String, session_key: Option<String>, user_id: Option<String> }`. Routes through existing Dispatcher → execute_turn_full. Reuses existing session management from Gateway. Must NOT add new platform logic — this is pure passthrough. Must NOT modify Gateway::handle_message (the existing echo).
  Parallelization: Wave 3 | Blocked by: T1 | Blocks: F verification
  References: `oben-gateway/src/main.rs:175-180` (Dispatcher creation), `oben-gateway/src/dispatcher.rs` (Dispatcher), `oben-gateway/src/router.rs` (existing routing), `oben-gateway/src/gateway.rs:93-106` (handle_message — currently echo)
  Acceptance criteria: `cargo check --workspace` passes; gateway accepts POST /gateway/prompt and routes through Dispatcher
  QA scenarios: happy — gateway receives cron prompt, executes via dispatcher; failure — gateway returns 500 with error details; Evidence `.omo/evidence/task-8-cron-acp-svc.md`
  Commit: Y | feat(gateway): add HTTP prompt endpoint for cron daemon delivery

## Final verification wave
> Runs in parallel after ALL todos. ALL must APPROVE. Surface results and wait for the user's explicit okay before declaring complete.
- [ ] F1. Plan compliance audit
  What: Cross-check all 8 todos against Scope IN/Must NOT Have. Verify no scope creep.
  Acceptance: Every Scope IN item has a matching todo; zero todos touch Scope OUT items
  Evidence: `.omo/evidence/f1-cron-acp-svc.md`
  Commit: N (merge commit after all F-criteria pass)

- [ ] F2. Code quality review
  What: Check idiomatic Rust — no `Arc<Mutex<>>` where `&mut` suffices, no panic on UTF-8 strings, proper error handling with `?` operator.
  Acceptance: No Arc<Mutex<T>> in new cron daemon code; UTF-8 string slicing uses `.chars().take(N)`, not byte slices
  Evidence: `.omo/evidence/f2-cron-acp-svc.md`

- [ ] F3. Real manual QA
  What: `cargo check --workspace` passes with zero errors/warnings on new code. `cargo test --package oben-cron` passes existing tests.
  Acceptance: All existing `oben-cron` tests pass (test_create_and_list, test_roundtrip_json, test_scan_cron_prompt_*); workspace compiles
  Evidence: `.omo/evidence/f3-cron-acp-svc.md`

- [ ] F4. Scope fidelity
  What: Verify no unintended side effects — existing cron CLI subcommands (list, create, pause, resume, remove, tick, start, info) still work. No changes to agent `trigger_nudge`, HookEngine, Dispatcher, or platform adapters.
  Acceptance: No diff on files outside: `oben-config`, `oben-cron/src/`, `oben-agent/src/hooks/runtime.rs` (NudgeHook only), `oben-gateway/src/` (endpoint only)
  Evidence: `.omo/evidence/f4-cron-acp-svc.md`

## Commit strategy
- Each todo is its own atomic commit on a feature branch `#cron-acp-svc`
- PR title: `#cron-acp-svc: Add ACP-based cron delivery modes`
- PR body must list all modified files and confirm documentation is updated in `docs/PRD-cron-parity.md` if applicable

## Success criteria
1. `cargo check --workspace` passes with zero errors
2. All existing `oben-cron` tests pass unchanged
3. `delivery_mode: daemon-agent` executes a cron job in-process
4. `delivery_mode: gateway` POSTs to gateway endpoint and records output
5. `delivery_mode: simple` works exactly as before (subprocess)
6. NudgeHook creates a cron job via HTTP POST when daemon-agent mode is active
7. Zero regression on existing CLI cron subcommands
