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
| S.1 | Write concurrency (BEGIN IMMEDIATE + jittered retry + WAL checkpoint) | 🔴 | ✅ | [#30](https://github.com/bowfeng/obenagent/issues/30) | `with_conn_mut()` manages `BEGIN IMMEDIATE`; jittered retry (20-150ms); WAL checkpoint every 50 writes |
| S.2 | Schema expansion (14 new columns: billing, cache, API tracking) | 🟡 | ✅ | [#32](https://github.com/bowfeng/obenagent/issues/32) | Declarative column reconciliation — just edit `SCHEMA_SQL` |
| S.3 | Compression lineage (`end_reason`-aware walking + orphan cleanup + ghost pruning) | 🟡 | ✅ | [#33](https://github.com/bowfeng/obenagent/issues/33) | `resolve_session_tip()` checks `end_reason='compression'` |
| S.4 | Trigram FTS5 table for CJK search | 🟡 | ✅ | [#34](https://github.com/bowfeng/obenagent/issues/34) | `messages_fts_trigram` with `tokenize='trigram'` |
| S.5 | Title management (sanitization + dedup + lineage resolution) | 🟡 | ✅ | [#37](https://github.com/bowfeng/obenagent/issues/37) | `sanitize_title()`, `resolve_session_by_title()`, `get_next_title_in_lineage()` |
| S.6 | Persistence on error (save session on all exit paths) | 🟡 | ✅ | [#36](https://github.com/bowfeng/obenagent/issues/36) | Match on response before save — error path calls save() then returns |
| S.7 | Memory Provider abstraction (`MemoryProvider` trait + prefetch/recall + sync) | 🔴 | ❌ | [#31](https://github.com/bowfeng/obenagent/issues/31) | Provider trait, `prefetch_all()`, `sync_all()` |
| S.8 | Message storage (remove destructive `DELETE FROM messages`) | 🟡 | ✅ | [#35](https://github.com/bowfeng/obenagent/issues/35) | `save_messages()` now delegates to `save_new_messages()`; new `clear_messages()` for compaction |

---

## Legend

- **🔴 Critical** — blocks production use
- **🟡 High** — important for core functionality
- **Status**: ✅ Done | ❌ Not Started

**Workflow:** Open issue → branch (`#<number>-<desc>`) → implement → PR → close issue.
