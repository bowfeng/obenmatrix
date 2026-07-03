## Summary

Fix two TUI issues: nested lock deadlock causing freezes, and duplicated streaming indicators.

## Changes

### 1. Fix TUI Deadlock (conversation.rs)

**Root Cause**: `render()` held `message_entries.lock()` and called `render_bordered_blocks()`, which tried to lock `message_entries` again at Phase 2 line 828 — nested `std::sync::Mutex` (non-reentrant) = deadlock.

**Fix**: Extract titles once while holding the lock in `render()`, pass as `&[Option<Line<'static>>]` slice to `render_bordered_blocks()`. No second lock occurs.

### 2. Deduplicate Streaming Indicator

"Streaming..." was showing in 3 places — user only wants it in the input bar:

| Location | Before | After |
|----------|--------|-------|
| Input bar | "⏳ Streaming..." | Unchanged (keep) |
| Status bar | "Streaming..." | "Busy" (shows "Streaming" in mode_text) |
| Messages panel | " Streaming... " | **Removed** + `render_turn_status()` method |

## Verification

- `cargo check -p oben-tui` — passes
- `cargo build -p oben-tui` — passes
- `cargo test -p oben-tui --lib` — 96 tests pass (2 pre-existing failures in `history::tests` unchanged)
