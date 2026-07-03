# Fix: Stale subagers persisting forever in TUI

## Problem
- `clear_subagers()` was defined in `SharedAgentState` but **never called anywhere**
- Subagers added by `SubagerCallback::on_start()` during `delegate_task` persist forever
- Subager panel (`[done] #0`) and agent message block get mixed together
- Every frame, `ChatPanel::draw()` sees `subagers` non-empty and renders accordion + detail

## Root cause
```
SubagerCallback::on_start() → subagers.push(SubagerInfo { status: "running" })
SubagerCallback::on_complete() → subagers[deleg_id].status = "completed"
Clear? → ❌ nothing clears it
```

## Fix
Call `clear_subagers()` when the turn completes (settled + transitioning).

### File: `oben-tui/src/panels/chat.rs`
In `update_from_turn_state()`, after the Phase 6 flush block completes (~line 344), add:
```rust
self.shared_state_ref.try_lock().map(|guard| {
    guard.clear_subagers();
});
```

**Timing:** This fires on `prev=Streaming, current=Completed` transition — the same point where we flush `streaming_text` and `completed_tools`. Subagent data has the same lifecycle, so it belongs here.

## Verification
- Run TUI, send a message that triggers `delegate_task` (e.g. "调用subager来写悬疑故事")
- After completion, `[done] #0` should disappear
- Subsequent turns should show clean message flow
- Build passes: `cargo check -p oben-tui`
