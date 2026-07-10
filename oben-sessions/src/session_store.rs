//! Configurable session store — choose between SQLite persistence or in-memory.

use anyhow::Result;
pub use oben_models::SessionStoreKind;

use crate::Session;
use crate::{DBSessionManager, MemSessionManager};

/// An enum holding either a `DBSessionManager` or `MemSessionManager`.
///
/// Created by `new()`, then all `SessionManager` trait methods are delegated
/// to the active variant. Also exposes backend-specific methods (e.g. `db()`,
/// `active_session()`) when the active variant matches.
pub enum SessionStore {
    Database(DBSessionManager),
    Memory(MemSessionManager),
}

impl SessionStore {
    /// Create a session store of the given kind.
    pub fn new(kind: SessionStoreKind) -> Result<Self> {
        Ok(match kind {
            SessionStoreKind::Database => Self::Database(DBSessionManager::new()?),
            SessionStoreKind::Memory => Self::Memory(MemSessionManager::new()),
        })
    }

    pub fn kind(&self) -> SessionStoreKind {
        match self {
            Self::Database(_) => SessionStoreKind::Database,
            Self::Memory(_) => SessionStoreKind::Memory,
        }
    }

    // ── Backend-specific access ──────────────────────────────────────────

    /// Access the underlying SQLite-backed session manager.
    pub fn as_db(&self) -> Option<&DBSessionManager> {
        match self {
            Self::Database(db) => Some(db),
            _ => None,
        }
    }

    /// Mutably access the underlying SQLite-backed session manager.
    pub fn as_db_mut(&mut self) -> Option<&mut DBSessionManager> {
        match self {
            Self::Database(db) => Some(db),
            _ => None,
        }
    }

    /// Access the underlying in-memory session manager.
    pub fn as_mem(&self) -> Option<&MemSessionManager> {
        match self {
            Self::Memory(mem) => Some(mem),
            _ => None,
        }
    }

    /// Mutably access the underlying in-memory session manager.
    pub fn as_mem_mut(&mut self) -> Option<&mut MemSessionManager> {
        match self {
            Self::Memory(mem) => Some(mem),
            _ => None,
        }
    }

    /// Set the title of the active session (Database) or a session by key (Memory).
    pub fn set_title(&mut self, new_title: &str) -> Result<(), anyhow::Error> {
        match self {
            Self::Database(db) => db.set_title(new_title),
            Self::Memory(mem) => mem.set_title("default", new_title),
        }
    }

    /// Set the title of a session by key.
    pub fn set_title_by_key(&mut self, key: &str, new_title: &str) -> Result<(), anyhow::Error> {
        match self {
            Self::Database(_) => Err(anyhow::anyhow!(
                "set_title_by_key is only supported on the Memory backend"
            )),
            Self::Memory(mem) => mem.set_title(key, new_title),
        }
    }
}

impl crate::SessionManager for SessionStore {
    fn init(&mut self) -> Result<(), anyhow::Error> {
        match self {
            Self::Database(db) => db.init(),
            Self::Memory(mem) => mem.init(),
        }
    }

    fn get_or_create_session(&mut self, name: &str) -> &mut Session {
        match self {
            Self::Database(db) => db.get_or_create_session(name),
            Self::Memory(mem) => mem.get_or_create_session(name),
        }
    }

    fn create_session(&mut self, name: &str) -> &mut Session {
        match self {
            Self::Database(db) => db.create_session(name),
            Self::Memory(mem) => mem.create_session(name),
        }
    }

    fn switch_session(&mut self, key: &str) -> Result<&mut Session, anyhow::Error> {
        match self {
            Self::Database(db) => db.switch_session(key),
            Self::Memory(mem) => mem.switch_session(key),
        }
    }

    fn reset_current_session(&mut self) -> Result<(), anyhow::Error> {
        match self {
            Self::Database(db) => db.reset_current_session(),
            Self::Memory(mem) => mem.reset_current_session(),
        }
    }

    fn reset_session(&mut self, key: &str) -> Result<(), anyhow::Error> {
        match self {
            Self::Database(db) => db.reset_session(key),
            Self::Memory(mem) => mem.reset_session(key),
        }
    }

    fn suspend_session(&mut self, key: &str) -> bool {
        match self {
            Self::Database(db) => db.suspend_session(key),
            Self::Memory(mem) => mem.suspend_session(key),
        }
    }

    fn mark_resume_pending(&mut self, key: &str, reason: &str) -> bool {
        match self {
            Self::Database(db) => db.mark_resume_pending(key, reason),
            Self::Memory(mem) => mem.mark_resume_pending(key, reason),
        }
    }

    fn clear_resume_pending(&mut self, key: &str) -> bool {
        match self {
            Self::Database(db) => db.clear_resume_pending(key),
            Self::Memory(mem) => mem.clear_resume_pending(key),
        }
    }

    fn list_sessions(&self, active_minutes: Option<u64>) -> Vec<crate::SessionListEntry> {
        match self {
            Self::Database(db) => db.list_sessions(active_minutes),
            Self::Memory(mem) => mem.list_sessions(active_minutes),
        }
    }

    fn delete_session(&mut self, key: &str) -> Result<(), anyhow::Error> {
        match self {
            Self::Database(db) => db.delete_session(key),
            Self::Memory(mem) => mem.delete_session(key),
        }
    }

    fn prune_sessions(&mut self, max_age_days: i64) -> usize {
        match self {
            Self::Database(db) => db.prune_sessions(max_age_days),
            Self::Memory(mem) => mem.prune_sessions(max_age_days),
        }
    }

    fn save_session(&mut self, session_id: Option<&str>) -> Result<(), anyhow::Error> {
        match self {
            Self::Database(db) => db.save_session(session_id),
            Self::Memory(mem) => mem.save_session(session_id),
        }
    }

    fn resolve_session_id(&self, key: &str) -> Option<String> {
        match self {
            Self::Database(db) => db.resolve_session_id(key),
            Self::Memory(mem) => mem.resolve_session_id(key),
        }
    }

    fn update_token_tracking(
        &mut self,
        session_id: &str,
        input_tokens: usize,
        output_tokens: usize,
        total_tokens: usize,
        estimated_cost_usd: f64,
    ) {
        match self {
            Self::Database(db) => {
                db.update_token_tracking(session_id, input_tokens, output_tokens, total_tokens, estimated_cost_usd)
            }
            Self::Memory(mem) => {
                mem.update_token_tracking(session_id, input_tokens, output_tokens, total_tokens, estimated_cost_usd)
            }
        }
    }

    fn split_after_compression(&mut self, parent_id: &str) -> Result<Session, anyhow::Error> {
        match self {
            Self::Database(db) => db.split_after_compression(parent_id),
            Self::Memory(mem) => mem.split_after_compression(parent_id),
        }
    }

    fn session_mut(&mut self, session_id: &str) -> Option<&mut Session> {
        match self {
            Self::Database(db) => db.session_mut(session_id),
            Self::Memory(mem) => mem.session_mut(session_id),
        }
    }

    fn session(&self, session_id: &str) -> Option<&Session> {
        match self {
            Self::Database(db) => db.session(session_id),
            Self::Memory(mem) => mem.session(session_id),
        }
    }

    fn save_compacted(
        &mut self,
        session_id: &str,
        messages: &[crate::Message],
    ) -> Result<(), anyhow::Error> {
        match self {
            Self::Database(db) => db.save_compacted(session_id, messages),
            Self::Memory(mem) => mem.save_compacted(session_id, messages),
        }
    }

    fn incremental_save(&mut self, session_id: Option<&str>) -> Result<(), anyhow::Error> {
        match self {
            Self::Database(db) => db.incremental_save(session_id),
            Self::Memory(mem) => mem.incremental_save(session_id),
        }
    }

    fn new_session(&mut self, name: &str) -> Result<&mut Session, anyhow::Error> {
        match self {
            Self::Database(db) => Ok(db.new_session(name)),
            Self::Memory(mem) => mem.new_session(name),
        }
    }

    fn find_key(&self, key: &str) -> Option<String> {
        match self {
            Self::Database(db) => db.find_key(key),
            Self::Memory(mem) => mem.find_key(key),
        }
    }

    fn list_sessions_full(&self) -> Vec<Session> {
        match self {
            Self::Database(db) => db.list_sessions_full(),
            Self::Memory(mem) => mem.list_sessions_full(),
        }
    }

    fn get_session_messages(&self, session_id: &str) -> Result<Vec<crate::Message>, anyhow::Error> {
        match self {
            Self::Database(db) => db.get_session_messages(session_id),
            Self::Memory(mem) => mem.get_session_messages(session_id),
        }
    }

    fn ensure_session_loaded(&mut self, session_id: &str) -> Result<(), anyhow::Error> {
        match self {
            Self::Database(db) => db.ensure_session_loaded(session_id),
            Self::Memory(mem) => mem.ensure_session_loaded(session_id),
        }
    }

    fn close(&mut self) -> Result<(), anyhow::Error> {
        match self {
            Self::Database(db) => db.close(),
            Self::Memory(mem) => mem.close(),
        }
    }

    fn set_compaction_summary(&mut self, session_id: &str, summary: String) -> Result<(), anyhow::Error> {
        match self {
            Self::Database(db) => db.set_compaction_summary(session_id, summary),
            Self::Memory(mem) => mem.set_compaction_summary(session_id, summary),
        }
    }

    fn get_compaction_summary(&self, session_id: &str) -> Option<String> {
        match self {
            Self::Database(db) => db.get_compaction_summary(session_id),
            Self::Memory(mem) => mem.get_compaction_summary(session_id),
        }
    }
}
