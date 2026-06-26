# Plan: Session Lifecycle moves to ContextWindowManager

## Status: ✅ IMPLEMENTATION COMPLETE (all steps done)

## Goal

Move `active_session_id` from `SessionManager` to `ContextWindowManager` so:
- Multiple concurrent sessions (Gateway/Telegram) each get their own CWM instance
- One shared `SessionManager` backend (SQLite) stores all sessions
- CWM owns token tracking AND session lifecycle for its bound conversation

## Design Summary

### Before (current)
```
Agent
  ├── SessionManager (owns active_session_id: Option<String>)
  └── ContextWindowManager (pure stateless — only receives messages: &[])
```

### After
```
Agent
  ├── SessionManager (storage only — no active_session concept)
  └── ContextWindowManager (owns active_session_id + tracks token usage)
```

### CWM New Methods
```rust
fn session_id(&self) -> Option<String>;
fn set_active_session(&mut self, id: String);
fn should_split_session(&self, now: DateTime<Utc>) -> bool;        // time-based split decision
fn on_message_received(&mut self, now: DateTime<Utc>);            // update timestamp
fn on_session_split(&mut self, session_manager: &mut dyn SessionManager, new_session_id: String);  // reset + bind
fn should_do_time_based_split(&mut self, session_manager: &mut dyn SessionManager) -> Option<String>;  // creates child session
fn should_split_after_compaction(&self, status: CompactStatus) -> bool;  // decision after compression
```

### Two Split Paths
1. **Time-based** (`should_split_session`): TurnExecutor checks at turn start; if >1 day gap → create new session → call `on_session_split(new_id)`
2. **Lineage-based** (compression): `compact()` triggers → `split_after_compression()` creates child → call `on_session_split(child_id)`

## File Change List (9 steps)

### Step 1: `oben-agent/src/context.rs` ✅ DONE
- Added `SessionSplitConfig` struct with `max_session_duration_seconds: u64` (default: 86400)
- Added 5 new trait methods to `ContextWindowManager`
- Updated blanket impl (`Box<dyn CWM>`) with delegation
- Updated module doc: "stateless" → "stateful + session lifecycle"
- Added Session Lifecycle section to docs

### Step 2: `oben-agent/src/compact_context.rs` ✅ DONE
- Added fields to `BuiltinContextWindowManager`:
  - `session_split_config: SessionSplitConfig`
  - `active_session_id: Option<String>`
  - `last_message_timestamp: Option<DateTime<Utc>>`
- Implemented all 5 new trait methods
- Updated `reset()` to also clear `last_message_timestamp`
- Updated `with_config()` to init new fields
- Fixed `reset_current_session()` to return `Ok(())` (deprecated path)
- Fixed `set_title()` to take `(key, new_title)` args
- Added 9 new session lifecycle tests

### Step 3: `oben-sessions/src/session.rs` + `oben-models/src/session.rs` (trait) ✅ DONE
- **Removed from trait**: `active_session_id()`, `active_session_mut()`, `active_session()` (then re-added with default impls for CLI compatibility)
- **Added to trait**: `resolve_session_id(&self, key: &str) -> Option<String>`
- **Added default implementations**: `active_session()`, `active_session_mut()`, `active_session_id()` (return None by default, DB impl overrides)
- Updated `split_after_compression()` doc: "CWM tracks active independently"

### Step 4: `oben-sessions/src/mem_session.rs` ✅ DONE
- Removed `active_session_id: Option<String>` from struct
- Removed all `self.active_session_id = ...` assignments
- Removed `active_session_id()`, `active_session_mut()`, `active_session()` trait impl entries
- Added `resolve_session_id()` implementation
- Fixed `reset_current_session()` to return `Ok(())`
- Fixed `set_title()` to take `&mut self, key: &str, new_title: &str`
- Fixed `new_session()` return type
- Removed orphaned `active_sid` reference in `prune_sessions()`

### Step 5: `oben-sessions/src/manager.rs` + `session_store.rs` ✅ DONE
- Removed `active_session_id()`, `active_session_mut()`, `active_session()` from SessionManager trait impl on `SessionDB`
- Added `resolve_session_id()` to `SessionDB` struct and trait impl
- `SessionStore::set_title()`: split into `set_title(new_title)` (active session) and `set_title_by_key(key, new_title)`
- Updated `SessionStore` blanket impl for all trait methods
- `active_session()` / `active_session_mut()` kept on `SessionStore` as frontend methods (DB-only)

### Step 6: `oben-agent/src/turn_executor.rs` ✅ DONE
- Time-based split check moved AFTER `resolve_session_id()` but BEFORE acquiring `session` mutable borrow
- Time-based: if `should_split_session(now)` → `split_after_compression()` → `on_session_split(child_id)`
- Lineage split (compression): replaced `context_window_manager.reset()` call with `on_session_split(child_id)` (step 2b)
- `on_message_received(now)` called right after pushing user message

### Step 6a: Compaction split architecture (deferred → implemented during coding) ✅ DONE
- **Problem**: `handle_compaction(&mut self, ...)` in trait couldn't take `&mut SessionManager` 
  because TurnExecutor already held `session: &mut Session` (borrowed from session_manager)
- **Solution**: Split into two phases:
  - CWM provides `should_split_after_compaction(&self, status) -> bool` (decision only)
  - CWM provides `should_do_time_based_split(&mut self, SM) -> Option<String>` (time-based creates session via SM)
  - TurnExecutor bridges: drops borrows → calls SM operations → calls `on_session_split(new_id)`
- TurnExecutor compaction pattern: `drop(session)` → `drop(current_session)` → `session_manager.split_after_compression()` →
  `session_manager.save_compacted()` → `context_window_manager.on_session_split()` → re-borrow session

### Step 7: `oben-agent/src/agent.rs` ⚠️ DEFERRED — no changes needed
- Agent already holds separate `CWM` and `SessionManager` refs
- Agent calls `cwm.session_id()` pattern or `session_manager.session_mut()` directly
- No `session_manager.active_session()` calls that need migration

### Step 8: `oben-cli/src/coordinator/cli.rs` ✅ DONE (via `SessionStore::active_session()` frontend method)
- CLI uses `SessionStore` which provides `active_session()` as a frontend method (non-trait method)
- No additional changes needed — CLI still works through `SessionStore` wrapper

### Step 9: Test fixes ✅ DONE
- Fixed `test_should_split_session_returns_false_within_gap`: 10min → 30s (was exceeding 60s threshold)
- Fixed `test_on_session_split_resets_tracking`: `reset()` now clears `last_message_timestamp`
- All 173 agent lib tests pass
- All 95 plugin tests pass
- 48/49 sessions lib tests pass (1 pre-existing failure: `test_save_and_load_roundtrip`)
- Integration test failures (`reset_session`, `session_rotation`, `delegate_tests`) are pre-existing (same errors before this branch)

## Verification Criteria

1. ✅ `cargo check` passes — full workspace compiles with 0 errors
2. ✅ All agent lib tests pass (173/173)
3. ✅ All plugin tests pass (95/95)
4. ✅ 48/49 sessions lib tests pass (1 pre-existing failure, not introduced by this branch)
5. CWM `session_id()` returns bound session id ✅
6. CWM `should_split_session()` returns true when gap > threshold ✅
7. Gateway can create per-channel CWMs sharing one SessionManager ✅ (structs are no longer coupled)

## Implementation Summary — What Changed

### Architecture
- `active_session_id` ownership moved from `SessionManager` → `ContextWindowManager`
- `SessionManager.resolve_session_id(key)` replaces direct `active_session` lookups for key→ID resolution
- CWM now owns: `session_id()`, `should_split_session()`, `on_message_received()`, `on_session_split()`
- **Compaction split architecture**: CWM provides decision-only methods (`should_split_after_compaction` returns `bool`), TurnExecutor performs execution
  - Resolves Rust borrow conflict: CWM can't take `&mut SessionManager` while TurnExecutor holds `session: &mut Session`
  - Pattern: `drop(session)` → `drop(current_session)` → SM ops → `on_session_split(new_id)` → re-borrow
- Two split paths both converge on `on_session_split(child_id)`: time-based (>1 day gap) and lineage-based (compression)
- CWM provides `should_do_time_based_split()` which creates child session via SM directly (returns `Option<String>`)
- `SessionManager` keeps `active_session()` as **default trait methods** (return `None`), with `DBSessionManager` overriding

### Files Modified (8 files)
1. `oben-models/src/session.rs` — trait: added `resolve_session_id`, default impls
2. `oben-agent/src/context.rs` — CWM trait: 7 lifecycle methods + `SessionSplitConfig`
3. `oben-agent/src/compact_context.rs` — BuiltinCWM: implements new methods, removed old `handle_compaction`
4. `oben-sessions/src/session_store.rs` — Split `set_title`, added `set_title_by_key`
5. `oben-sessions/src/mem_session.rs` — MemSessionManager: removed active_session, added `resolve_session_id`
6. `oben-sessions/src/manager.rs` — SessionDB: removed active_session from trait impl, added `resolve_session_id`
7. `oben-agent/src/turn_executor.rs` — integrated all 3 lifecycle call sites, drop-reborrow compaction pattern
8. `.opencode/plans/session-lifecycle-cwm.md` — this plan
