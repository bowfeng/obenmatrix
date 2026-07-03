# Task 4: Wire NudgeHook to Cron Daemon HTTP Submit

## Evidence

### cargo check -p oben-agent

```
$ cargo check -p oben-agent
warning: field `finish_reason` is never read
   --> oben-transport/src/chat_completions.rs:339:9
    |
334 | struct StreamChoice {
    |        ------------ field in this struct
...
339 |     pub finish_reason: Option<String>,
    |         ^^^^^^^^^^^^^
    |
    = note: `#[warn(dead_code)]` (part of `#[unused]`) on default

warning: `oben-transport` (lib) generated 1 warning
    Checking oben-agent v0.1.0 (/Users/ellie/workspace/oben-alien/oben-agent)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 20.82s
```

**Result**: ✅ `oben-agent` compiles with zero errors. The only warning is pre-existing in `oben-transport`.

### Modified files

1. `oben-agent/Cargo.toml` — Added `oben-cron = { path = "../oben-cron" }` dependency
2. `oben-agent/src/hooks/runtime.rs` — Wired CronClient into NudgeHook
3. `oben-cron/src/http.rs` — Added `#[derive(Clone)]` to CronClient

### New NudgeHook implementation

```rust
// Lines 1-8: imports — added CronClient + CronSubmitRequest
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use anyhow;
use super::kind::*;
use crate::ContextWindowManager;
use crate::nudge::NudgeConfig;
use oben_cron::http::{CronClient, CronSubmitRequest};

// Lines 14-23: NudgeHook struct with new cron_client field
pub struct NudgeHook {
    config: NudgeConfig,
    turn_count: AtomicUsize,
    has_memory_tools: bool,
    sub_turn_callback: Option<Mutex<Box<dyn Fn(&str) -> anyhow::Result<()> + Send + Sync>>>,
    cron_client: Option<CronClient>,  // ← NEW
}

// Lines 37-46: from_config_for_daemon constructor
pub fn from_config_for_daemon(config: &NudgeConfig, daemon_url: Option<String>) -> Self {
    Self {
        config: config.clone(),
        turn_count: AtomicUsize::new(0),
        has_memory_tools: false,
        sub_turn_callback: None,
        cron_client: Some(CronClient::new(daemon_url)),
    }
}

// Lines 87-123: on_post_turn with daemon submission path
fn on_post_turn(&self, _response: &str, _success: bool, _turn_count: u32) {
    if !self.config.enabled() || !self.has_memory_tools { return; }
    let turns = self.turn_count.fetch_add(1, Ordering::SeqCst);
    let threshold = self.config.memory_nudge_interval;
    if turns < threshold { return; }
    self.turn_count.store(0, Ordering::SeqCst);

    if let Some(ref client) = self.cron_client {
        let client = client.clone();
        let prompt = crate::nudge::build_nudge_prompt(true, true);
        tokio::spawn(async move {
            let request = CronSubmitRequest {
                prompt,
                deliver_target: None, // use daemon default
                session_id: None,
            };
            match client.submit(&request).await {
                Ok(resp) => {
                    tracing::info!(
                        job_id = resp.job_id,
                        status = resp.status,
                        "Nudge cron job submitted to daemon agent"
                    );
                }
                Err(e) => {
                    tracing::warn!(error = e.to_string(), "Nudge cron submit failed");
                }
            }
        });
    } else if let Some(ref callback) = self.sub_turn_callback {
        // Fallback: CLI path calls callback directly
        if let Ok(guard) = callback.lock() {
            let prompt = crate::nudge::build_nudge_prompt(true, true);
            let _ = guard(&prompt);
        }
    }
}
```

### Design decisions

- **Non-blocking**: `tokio::spawn(async move { ... })` fires HTTP POST without blocking the turn loop
- **Fire-and-forget**: Failures are `tracing::warn!`-ed, not propagated
- **Dual-path**: When `cron_client` is `Some`, uses HTTP; when `None`, falls back to existing `sub_turn_callback` (CLI subprocess path)
- **No subprocesses**: This is the hook-level HTTP integration only; existing `Agent::trigger_nudge()` handles the CLI subprocess path

### Known pre-existing issues

- `oben-wasm` has pre-existing compilation errors (trait `on_post_turn` signature mismatch, `AgentInit` not implemented for WASM adapters). These existed before this change and are unrelated.
