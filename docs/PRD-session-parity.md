# Session Layer — Parity vs Hermes-Agent

**Scope:** `oben-sessions` + session integration in `oben-agent`  
**Reference:** `/Users/ellie/workspace/hermes-agent/agent/conversation_loop.py`, `hermes_state.py`

---

## Summary

Hermes-agent has a mature, battle-tested session layer. Our Rust port (`oben-sessions`) covers the basic CRUD skeleton but is missing critical infrastructure for production use.

---

## Gap Matrix

| # | Feature | Severity | Status | Issue | Notes |
|---|---------|----------|--------|-------|-------|
| S.1 | Write concurrency (BEGIN IMMEDIATE + jittered retry + WAL checkpoint) | 🔴 | ✅ | [#30](https://github.com/bowfeng/oben-alien/issues/30) | `with_conn_mut()` manages `BEGIN IMMEDIATE`; jittered retry (20-150ms); WAL checkpoint every 50 writes |
| S.2 | Schema expansion (14 new columns: billing, cache, API tracking) | 🟡 | ✅ | [#32](https://github.com/bowfeng/oben-alien/issues/32) | Declarative column reconciliation — just edit `SCHEMA_SQL` |
| S.3 | Compression lineage (`end_reason`-aware walking + orphan cleanup + ghost pruning) | 🟡 | ✅ | [#33](https://github.com/bowfeng/oben-alien/issues/33) | `resolve_session_tip()` checks `end_reason='compression'` |
| S.4 | Trigram FTS5 table for CJK search | 🟡 | ✅ | [#34](https://github.com/bowfeng/oben-alien/issues/34) | `messages_fts_trigram` with `tokenize='trigram'` |
| S.5 | Title management (sanitization + dedup + lineage resolution) | 🟡 | ✅ | [#37](https://github.com/bowfeng/oben-alien/issues/37) | `sanitize_title()`, `resolve_session_by_title()`, `get_next_title_in_lineage()` |
| S.6 | Persistence on error (save session on all exit paths) | 🟡 | ✅ | [#36](https://github.com/bowfeng/oben-alien/issues/36) | Match on response before save — error path calls save() then returns |
| S.7 | Memory Provider abstraction (`MemoryProvider` trait + prefetch/recall + sync) | 🔴 | ✅ | [#31](https://github.com/bowfeng/oben-alien/issues/31), [#43](https://github.com/bowfeng/oben-alien/pull/43) | `MemoryProvider` trait (14 methods), `BuiltinProvider`, `MemoryManager` (tool routing, fan-out), `StreamingContextScrubber` |
| S.8 | Message storage (remove destructive `DELETE FROM messages`) | 🟡 | ✅ | [#35](https://github.com/bowfeng/oben-alien/issues/35) | `save_messages()` now delegates to `save_new_messages()`; new `clear_messages()` for compaction |
| S.9 | **Session rotation on compression** (end old + create new with lineage) | 🔴 | ✅ | [#41](https://github.com/bowfeng/oben-alien/pull/42) | `end_session("compression")` → new session ID → `create_session(parent_session_id=old)` → title auto-numbering in lineage → `on_session_start(boundary_reason="compression")` → memory manager `on_session_switch()` |

---

## Legend

- **🔴 Critical** — blocks production use
- **🟡 High** — important for core functionality
- **Status**: ✅ Done | ❌ Not Started

**Note on S.9:** The schema already has `parent_session_id` and `end_reason` columns (S.3 infrastructure), and `end_session()` exists in `SessionDB`. However, the compaction codepath in `oben-agent` **never calls `end_session()` or creates a new session** — it mutates the message buffer in-place on the same session row. The columns exist but are never populated during compaction. This means:
- No compaction history tracked (can't see how many times a session was compressed)
- No lineage chain for session search (parent→child relationships invisible)
- No boundary for ContextWindowManager / memory provider reset
- Title auto-numbering never triggered (e.g. "My Task (2)", "My Task (3)")

Hermes rotates sessions on compression to create clean boundaries, track lineage, and let external systems (ContextWindowManagers, memory providers) reset state per-phase. Oben's approach is simpler but loses all compaction metadata.

**Workflow:** Open issue → branch (`#<number>-<desc>`) → implement → PR → close issue.
