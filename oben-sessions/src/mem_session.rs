//! In-memory session manager — pure HashMap-backed, no persistence.
//!
//! Implements the `SessionManager` trait to provide the same interface as
//! `DBSessionManager` for testing and scenarios where SQLite isn't needed.

use std::collections::HashMap;

use anyhow::{anyhow, Result};
use chrono::Utc;
use oben_models::{
    Message, Session, SessionListEntry, SessionManager, SessionMetadata, SessionSource,
};

/// Pure in-memory session manager.
///
/// All operations happen in memory. No database, no persistence.
/// Useful for testing, one-shot commands, or embedded scenarios.
pub struct MemSessionManager {
    sessions: HashMap<String, Session>,
}

impl MemSessionManager {
    /// Create an empty in-memory session manager.
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
        }
    }

    /// Create an in-memory session manager (path ignored, kept for API parity
    /// with `DBSessionManager::new_with_path`).
    pub fn new_with_path(_path: std::path::PathBuf) -> Self {
        Self::new()
    }
}

impl Default for MemSessionManager {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(clippy::result_large_err)]
impl SessionManager for MemSessionManager {
    fn init(&mut self) -> Result<(), anyhow::Error> {
        Ok(())
    }

    fn get_or_create_session(&mut self, name: &str) -> &mut Session {
        let now = Utc::now();
        let key = if let Some(session) = self.sessions.values_mut().find(|s| s.name == name) {
            session.id.clone()
        } else {
            let id = uuid::Uuid::new_v4().to_string();
            let session = Session {
                id: id.clone(),
                name: name.to_string(),
                created_at: now,
                updated_at: now,
                messages: Vec::new(),
                memory_context: None,
                summary_chunks: Vec::new(),
                persisted_message_count: 0,
                metadata: SessionMetadata {
                    id: id.clone(),
                    name: name.to_string(),
                    source: SessionSource::Cli,
                    title: Some(name.to_string()),
                    started_at: now,
                    ended_at: None,
                    end_reason: None,
                    message_count: 0,
                    ..Default::default()
                },
            };
            self.sessions.insert(id.clone(), session);
            id
        };
        self.sessions.get_mut(&key).unwrap()
    }

    fn create_session(&mut self, name: &str) -> &mut Session {
        let now = Utc::now();
        let id = uuid::Uuid::new_v4().to_string();
        let session = Session {
            id: id.clone(),
            name: name.to_string(),
            created_at: now,
            updated_at: now,
            messages: Vec::new(),
            memory_context: None,
            summary_chunks: Vec::new(),
            persisted_message_count: 0,
            metadata: SessionMetadata {
                id: id.clone(),
                name: name.to_string(),
                source: SessionSource::Cli,
                title: Some(name.to_string()),
                started_at: now,
                ended_at: None,
                end_reason: None,
                message_count: 0,
                ..Default::default()
            },
        };
        let id = id.clone();
        self.sessions.insert(id.clone(), session);
        
        self.sessions.get_mut(&id).unwrap()
    }

    fn switch_session(&mut self, key: &str) -> Result<&mut Session, anyhow::Error> {
        if !self.sessions.contains_key(key)
            && !self.sessions.values().any(|s| s.name == key)
        {
            return Err(anyhow!("Session not found: {}", key));
        }
        let target = self
            .sessions
            .get_key_value(key)
            .map(|(k, _)| k.clone())
            .or_else(|| {
                for (k, s) in self.sessions.iter() {
                    if s.name == key {
                        return Some(k.clone());
                    }
                }
                None
            })
            .ok_or_else(|| anyhow!("Session not found: {}", key))?;

        Ok(self.sessions.get_mut(&target).unwrap())
    }

    fn reset_current_session(&mut self) -> Result<(), anyhow::Error> {
        // Deprecated: the notion of "current" session lives in ContextWindowManager now.
        // Callers should use reset_session(key) with a specific key instead.
        Ok(())
    }

    fn reset_session(&mut self, key: &str) -> Result<(), anyhow::Error> {
        let sid = self
            .sessions
            .get_key_value(key)
            .map(|(k, _)| k.clone())
            .or_else(|| {
                self.sessions
                    .iter()
                    .find(|(_, s)| s.name == key)
                    .map(|(k, _)| k.clone())
            })
            .ok_or_else(|| anyhow!("Session not found: {}", key))?;

        if let Some(session) = self.sessions.get_mut(&sid) {
            session.metadata.end_reason = Some("reset".to_string());
            session.metadata.ended_at = Some(Utc::now());
            session.messages.clear();
            session.persisted_message_count = 0;
            let new_id = uuid::Uuid::new_v4().to_string();
            session.metadata.id = new_id.clone();
            session.id = new_id;
            
        }
        Ok(())
    }

    fn suspend_session(&mut self, key: &str) -> bool {
        let sid = self
            .sessions
            .get_key_value(key)
            .map(|(k, _)| k.clone())
            .or_else(|| {
                self.sessions
                    .iter()
                    .find(|(_, s)| s.name == key)
                    .map(|(k, _)| k.clone())
            })
            .unwrap_or_default();

        if let Some(session) = self.sessions.get_mut(&sid) {
            session.metadata.suspended = true;
            session.metadata.end_reason = Some("suspended".to_string());
            session.metadata.ended_at = Some(Utc::now());
            true
        } else {
            false
        }
    }

    fn mark_resume_pending(&mut self, key: &str, reason: &str) -> bool {
        let sid = self
            .sessions
            .get_key_value(key)
            .map(|(k, _)| k.clone())
            .or_else(|| {
                self.sessions
                    .iter()
                    .find(|(_, s)| s.name == key)
                    .map(|(k, _)| k.clone())
            })
            .unwrap_or_default();

        if let Some(session) = self.sessions.get_mut(&sid) {
            if !session.metadata.suspended {
                session.metadata.resume_pending = true;
                session.metadata.resume_reason = Some(reason.to_string());
                session.metadata.last_resume_marked_at = Some(Utc::now());
                true
            } else {
                false
            }
        } else {
            false
        }
    }

    fn clear_resume_pending(&mut self, key: &str) -> bool {
        let sid = self
            .sessions
            .get_key_value(key)
            .map(|(k, _)| k.clone())
            .or_else(|| {
                self.sessions
                    .iter()
                    .find(|(_, s)| s.name == key)
                    .map(|(k, _)| k.clone())
            })
            .unwrap_or_default();

        if let Some(session) = self.sessions.get_mut(&sid) {
            if session.metadata.resume_pending {
                session.metadata.resume_pending = false;
                session.metadata.resume_reason = None;
                session.metadata.last_resume_marked_at = None;
                true
            } else {
                false
            }
        } else {
            false
        }
    }

    fn list_sessions(&self, active_minutes: Option<u64>) -> Vec<SessionListEntry> {
        let now = Utc::now();
        let cutoff = active_minutes.map(|m| now - chrono::Duration::minutes(m as i64));
        self.sessions
            .values()
            .filter(|s| cutoff.map(|c| s.updated_at >= c).unwrap_or(true))
            .map(|s| SessionListEntry {
                id: s.id.clone(),
                name: s.name.clone(),
                title: s.metadata.title.clone(),
                source: s.metadata.source.clone(),
                model: s.metadata.model.clone(),
                message_count: s.metadata.message_count,
                tool_call_count: s.metadata.tool_call_count,
                input_tokens: s.metadata.input_tokens,
                output_tokens: s.metadata.output_tokens,
                started_at: s.metadata.started_at,
                last_active: s.updated_at,
                ended_at: s.metadata.ended_at,
                end_reason: s.metadata.end_reason.clone(),
                preview: s.metadata.preview.clone(),
                parent_session_id: s.metadata.parent_session_id.clone(),
                suspended: s.metadata.suspended,
            })
            .collect()
    }

    fn delete_session(&mut self, key: &str) -> Result<(), anyhow::Error> {
        let sid = self
            .sessions
            .get_key_value(key)
            .map(|(k, _)|k.clone())
            .or_else(|| {
                self.sessions
                    .iter()
                    .find(|(_, s)| s.name == key)
                    .map(|(k, _)| k.clone())
            })
            .ok_or_else(|| anyhow!("Session not found: {}", key))?;

        self.sessions.remove(&sid);
        Ok(())
    }

    fn prune_sessions(&mut self, max_age_days: i64) -> usize {
        if max_age_days <= 0 {
            return 0;
        }
        let cutoff = Utc::now() - chrono::Duration::days(max_age_days);
        let to_remove: Vec<String> = self
            .sessions
            .iter()
            .filter(|(_, s)| {
                !s.metadata.suspended
                    && s.updated_at < cutoff
            })
            .map(|(id, _)| id.clone())
            .collect();
        for sid in &to_remove {
            self.sessions.remove(sid);
        }
        to_remove.len()
    }

    fn save_session(&mut self, _session_id: Option<&str>) -> Result<(), anyhow::Error> {
        Ok(())
    }

    fn resolve_session_id(&self, key: &str) -> Option<String> {
        self.sessions.get_key_value(key)
            .map(|(k, _)| k.clone())
            .or_else(|| {
                self.sessions.iter()
                    .find(|(_, s)| s.name == key || s.id == key)
                    .map(|(k, _)| k.clone())
            })
    }

    fn update_token_tracking(
        &mut self,
        session_id: &str,
        input_tokens: usize,
        output_tokens: usize,
        total_tokens: usize,
        estimated_cost_usd: f64,
    ) {
        if let Some(s) = self.sessions.get_mut(session_id) {
            s.metadata.input_tokens += input_tokens;
            s.metadata.output_tokens += output_tokens;
            s.metadata.total_tokens += total_tokens;
            s.metadata.estimated_cost_usd += estimated_cost_usd;
            s.metadata.cost_status = "tracked".to_string();
            s.metadata.last_prompt_tokens = input_tokens;
        }
    }

    fn split_after_compression(&mut self, parent_id: &str) -> Result<Session, anyhow::Error> {
        // Find parent
        let parent_name = self
            .sessions
            .get(parent_id)
            .map(|p| p.name.clone())
            .ok_or_else(|| anyhow!("Parent session not found: {}", parent_id))?;

        // Find next child number
        let next_num = {
            let mut max_num = 1usize;
            for s in self.sessions.values() {
                if s.metadata.parent_session_id.as_deref() == Some(parent_id) {
                    if let Some(title) = &s.metadata.title {
                        if let Some(suffix) = title.strip_prefix(&parent_name) {
                            if let Some(num_str) = suffix.strip_prefix(" (").and_then(|s| s.strip_suffix(')')) {
                                if let Ok(n) = num_str.parse::<usize>() {
                                    if n > max_num { max_num = n; }
                                }
                            }
                        }
                    }
                }
            }
            max_num + 1
        };

        let now = Utc::now();
        let child_id = uuid::Uuid::new_v4().to_string();
        let child_title = format!("{} ({})", parent_name, next_num);
        let child_name = parent_name;

        // End parent
        if let Some(parent) = self.sessions.get_mut(parent_id) {
            parent.metadata.end_reason = Some("compression".to_string());
            parent.metadata.ended_at = Some(now);
        }

        // Create child
        let child_session = Session {
            id: child_id.clone(),
            name: child_name.clone(),
            created_at: now,
            updated_at: now,
            messages: Vec::new(),
            memory_context: None,
            summary_chunks: Vec::new(),
            persisted_message_count: 0,
            metadata: SessionMetadata {
                id: child_id.clone(),
                name: child_name,
                source: SessionSource::Cli,
                title: Some(child_title),
                started_at: now,
                ended_at: None,
                end_reason: None,
                parent_session_id: Some(parent_id.to_string()),
                message_count: 0,
                ..Default::default()
            },
        };

        self.sessions.insert(child_id.clone(), child_session);
        

        Ok(self.sessions.get(&child_id).unwrap().clone())
    }

    fn session_mut(&mut self, session_id: &str) -> Option<&mut Session> {
        self.sessions.get_mut(session_id)
    }

    fn session(&self, session_id: &str) -> Option<&Session> {
        self.sessions.get(session_id)
    }

    fn save_compacted(
        &mut self,
        session_id: &str,
        messages: &[Message],
    ) -> Result<(), anyhow::Error> {
        if let Some(s) = self.sessions.get_mut(session_id) {
            s.messages = messages.to_vec();
            s.persisted_message_count = messages.len();
        }
        Ok(())
    }

    fn incremental_save(&mut self, _session_id: Option<&str>) -> Result<(), anyhow::Error> {
        Ok(())
    }

    fn new_session(&mut self, name: &str) -> Result<&mut Session, anyhow::Error> {
        let now = Utc::now();
        let sid = uuid::Uuid::new_v4().to_string();
        let session = Session {
            id: sid.clone(),
            name: name.to_string(),
            created_at: now,
            updated_at: now,
            messages: Vec::new(),
            memory_context: None,
            summary_chunks: Vec::new(),
            persisted_message_count: 0,
            metadata: SessionMetadata {
                id: sid.clone(),
                name: name.to_string(),
                source: SessionSource::Cli,
                title: Some(name.to_string()),
                started_at: now,
                ..Default::default()
            },
        };
        self.sessions.insert(sid.clone(), session);
        self.sessions.get_mut(&sid).ok_or_else(|| anyhow!("Session insert failed: {}", sid))
    }

    fn find_key(&self, key: &str) -> Option<String> {
        self.sessions.get_key_value(key).map(|(k, _)| k.clone())
    }

    fn list_sessions_full(&self) -> Vec<Session> {
        self.sessions.values().cloned().collect()
    }

    fn get_session_messages(&self, session_id: &str) -> Result<Vec<Message>, anyhow::Error> {
        self.sessions
            .get(session_id)
            .map(|s| s.messages.clone())
            .ok_or_else(|| anyhow!("Session not found: {}", session_id))
    }

    fn ensure_session_loaded(&mut self, session_id: &str) -> Result<(), anyhow::Error> {
        if !self.sessions.contains_key(session_id) {
            return Err(anyhow!("Session not found: {}", session_id));
        }
        Ok(())
    }

    fn close(&mut self) -> Result<(), anyhow::Error> {
        self.sessions.clear();
        Ok(())
    }
}

impl MemSessionManager {
    /// Set the title of a session by key or ID.
    pub fn set_title(&mut self, key: &str, new_title: &str) -> Result<(), anyhow::Error> {
        let sid = self.resolve_session_id(key).ok_or_else(|| anyhow!("Session not found: {}", key))?;
        if let Some(session) = self.sessions.get_mut(&sid) {
            session.metadata.title = Some(new_title.to_string());
        }
        Ok(())
    }
}
