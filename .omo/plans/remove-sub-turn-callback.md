# Plan: Remove `sub_turn_callback` from `NudgeHook`

## Goal

Completely remove the `sub_turn_callback` field, setter, and fallback branch from `NudgeHook` — the daemon CronClient path is the only remaining code path.

## Scope

- Single file: `oben-agent/src/hooks/runtime.rs`
- No callers of `set_sub_turn_callback` in the codebase (verified via grep)

## Steps

### 1. Remove field + comment from `NudgeHook` struct

**File:** `oben-agent/src/hooks/runtime.rs:14-22`

Remove:
- Line 18: `sub_turn_callback: Option<Mutex<Box<dyn Fn(&str) -> anyhow::Result<()> + Send + Sync>>>,`
- Line 21: `/// When 'None', the hook calls the sub_turn_callback (CLI path).`

### 2. Remove `None` initializers from constructors

Three places in `from_config`, `from_config_for_daemon`, and `from_config_internal`:

- `sub_turn_callback: None,`

### 3. Remove `set_sub_turn_callback` method

Delete the entire method block:

```rust
pub fn set_sub_turn_callback<F>(&mut self, f: F)
where F: Fn(&str) -> anyhow::Result<()> + Send + Sync + 'static {
    self.sub_turn_callback = Some(Mutex::new(Box::new(f)));
}
```

### 4. Remove fallback branch in `on_post_turn`

Delete lines 118-123:

```rust
} else if let Some(ref callback) = self.sub_turn_callback {
    if let Ok(guard) = callback.lock() {
        let prompt = crate::nudge::build_nudge_prompt(true, true);
        let _ = guard(&prompt);
    }
}
```

The if-else becomes just `if let Some(ref client) = self.cron_client { ... }` with no else branch.

### 5. Verify build

```bash
cargo check -p oben-agent
```

## Risk Assessment

- **Low risk** — no callers of `set_sub_turn_callback` exist in the codebase
- The `Mutex`, `Box`, and trait object imports may become unused — remove if clippy complains
- `on_pre_turn` and `on_post_turn` are still fully functional with the cron_client path
