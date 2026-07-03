---
slug: cron-acp-svc
status: updating
approach: Cron daemon uses HTTP endpoints (keep as designed). ACP was overkill.
---

# Draft: cron-acp-svc

## Components (topology ledger)
| id | outcome | status | evidence |
|---|---|---|---|
| oben-cron | Standalone JSON-persisted daemon, 60s poll loop, `obenalien run -p` subprocess | active | `oben-cron/src/main.rs:52-74`, `oben-cron/src/jobs.rs:500-552` |
| NudgeHook | Fires `on_post_turn`, currently logs debug only | active | `oben-agent/src/hooks/runtime.rs:40-54` |
| HookEngine | `run_hook_turn()` and `init_hooks()` methods exist | active | `oben-agent/src/hooks/runtime.rs:230-301` |
| Gateway | Platform adapters + Dispatcher, no HTTP server | active | `oben-gateway/src/main.rs`, `oben-gateway/src/gateway.rs:93-106` |
| CliDispatch | All cron subcommands: list/create/pause/resume/remove/tick/start/info | active | `oben-cli/src/cli.rs:67-128`, `oben-cli/src/dispatch.rs:855-1110` |
| Agent::trigger_nudge | Full sub-turn with bounded max_iterations=16 | active | `oben-agent/src/agent.rs:676-769` |

## Open assumptions (announced defaults)
| assumption | adopted default | rationale | reversible? |
|---|---|---|---|
| Execution mode | User-selected via config: `simple: obenalien run -p`, `daemon: new agent with own conversation`, `gateway: via platform channel` | User explicitly requested user choice | Yes, but requires config migration |
| Cron daemon stays standalone | Remains separate binary, not merged into agent | PID management, cron scheduling is orthogonal to agent loop | Moderate |
| JSON persistence stays | Keep `~/.config/obenalien/cron/jobs.json` | Existing CLI cron commands depend on it; no schema change needed | Yes |
| HTTP for daemon-agent communication | HTTP POST to agent listener, JSON-RPC payload | reqwest already in workspace deps; tokio runtime available | Yes |
| Poll loop stays on 60s | Same interval as current daemon | No reason to change frequency | Trivial to change |
| Nudge triggers = cron jobs | NudgeHook creates a one-shot cron job for the memory-review prompt | Same code path as scheduled jobs; no special case | Yes |
| Safety scanning stays | `scan_cron_prompt()` keeps running on job creation | Prevents injection; no change needed | N/A |

## Findings (cited - path:lines)
- CronStore::default_path(): `oben-cron/src/jobs.rs:265-269` → `~/.config/obenalien/cron/`
- CronStore::advance_job(): `oben-cron/src/jobs.rs:500-552` → spawns `obenalien run -p <prompt>`, writes stdout to `{output_dir}/{id}/{date}_{time}.md`
- CronStore::get_due_jobs(): `oben-cron/src/jobs.rs:487-498` → filter enabled + (scheduled|error) + next_run_at <= now
- CronDaemon::spawn: `oben-cron/src/jobs.rs:594-606` → tokio::spawn run_loop
- NudgeHook struct: `oben-agent/src/hooks/runtime.rs:12-24` → nudge_config, has_memory_tools:AtomicBool
- NudgeHook.on_post_turn: `oben-agent/src/hooks/runtime.rs:40-54` → checks enabled && has_memory_tools && threshold, logs debug
- HookEngine::run_hook_turn: `oben-agent/src/hooks/runtime.rs:257-301` → full Agent turn via TurnExecutor
- HookEngine::init_hooks: `oben-agent/src/hooks/runtime.rs:230-252` → broadcasts to all hook kinds
- Agent::trigger_nudge: `oben-agent/src/agent.rs:676-769` → creates new session, runs execute_turn_full with max_iterations=16
- Gateway handle_message: `oben-gateway/src/gateway.rs:93-107` → TODO: "Route through conversation loop" — currently just echo
- Cli cron commands: `oben-cli/src/cli.rs:67-128` → CronCommand enum with list/create/pause/resume/remove/tick/start/info
- Cli dispatch: `oben-cli/src/dispatch.rs:855-1110` → all cron ops use CronStore directly; cron_start() spawns oben-cron binary
- Gateway main: `oben-gateway/src/main.rs` → builds Agent from config, creates Dispatcher, loads platform adapters

## Decisions (with rationale)
1. **Add HTTP listener to cron daemon** — exposes `/cron/trigger` and `/cron/status` POST endpoints.
2. **Three execution modes in config** — `mode: "simple"` (obenalien run -p), `mode: "daemon-agent"` (daemon creates its own agent conversation), `mode: "gateway"` (daemon POSTs to gateway endpoint).
3. **Daemon agent model** — when mode=daemon-agent, daemon builds an Agent from AppConfig, creates a dedicated session ("cron-job-{job_id}"), and executes the prompt via execute_turn_full.
4. **Gateway integration** — when mode=gateway, daemon POSTs to existing gateway HTTP endpoint (new endpoint `POST /gateway/prompt`), gateway routes through dispatcher → execute_turn_full with platform metadata.
5. **No schema changes to CronJob** — keep all existing fields; add `delivery_mode` field with default "local" (backward compat).

## Scope IN
- Add HTTP server (actix-web or axum) to `oben-cron` with `/cron/trigger` and `/cron/status` endpoints
- Add `delivery_mode` config to choose execution backend (simple/daemon-agent/gateway)
- In `simple` mode: keep existing `obenalien run -p` subprocess path (no change)
- In `daemon-agent` mode: build Agent, execute turn, record output → same as current advance_job but without subprocess
- In `gateway` mode: POST to gateway HTTP endpoint, record output
- NudgeHook creates a one-shot cron job via HTTP POST to daemon
- Config schema change: add `cron.delivery_mode` field
- Keep all existing JSON persistence, scheduling, CLI subcommands unchanged

## Scope OUT (Must NOT have)
- New database (keep JSON)
- Web UI for cron management
- Real-time job status streaming (WebSocket/SSE)
- Authentication on daemon HTTP endpoint (assume local-only)
- Message queue / retry queue with dead-letter (deferred)
- Platform-specific routing logic (just passthrough to gateway)
- ACP protocol (cron daemon uses HTTP endpoints as designed)

## Open questions
- None remaining — user answered all fork questions via approval gate questions

## Approval gate
status: awaiting-update-of-plan
pending-action: update plan to remove ACP references
approach: Keep cron daemon using HTTP endpoints, no ACP protocol layer
<!-- When exploration is exhausted and unknowns are answered, set status: awaiting-approval. -->
<!-- That durable record is the loop guard: on a later turn read it and resume at the gate instead of re-running exploration. -->
