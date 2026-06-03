//! Session management — SQLite-backed storage, FTS5 search, bounded curated memory.
//!
//! Three deep modules:
//! - **`manager`** — `SessionDB` (SQLite + FTS5) and `SessionManager` (in-memory).
//!   Maps to `hermes_state.py::SessionDB` + `agent/memory_manager.py`.
//! - **`search`** — Three-shape search (discover/scroll/browse) backed by FTS5.
//!   Maps to `tools/session_search_tool.py`.
//! - **`skill_curation`** — Bounded curated memory (MEMORY.md/USER.md) with
//!   injection scanning and frozen system-prompt snapshots.
//!   Maps to `tools/memory_tool.py`.
//! - **`memory_provider`** — Pluggable `MemoryProvider` trait, `BuiltinProvider`,
//!   `MemoryManager` orchestration, and `StreamingContextScrubber`.
//!   Maps to `agent/memory_provider.py` + `agent/memory_manager.py`.

pub mod manager;
pub mod memory_provider;
pub mod search;
pub mod skill_curation;

pub use manager::*;
pub use oben_models::SessionStore;
pub use search::*;
pub use skill_curation::*;
