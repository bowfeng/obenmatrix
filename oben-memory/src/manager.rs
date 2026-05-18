/// Session storage and memory management.
/// Maps to `agent/memory_manager.py`.

use anyhow::Result;
use oben_models::{Message, Session};
use std::collections::HashMap;
use std::path::PathBuf;
use tracing::info;

/// Persistent session store.
pub struct MemoryManager {
    sessions: HashMap<String, Session>,
    storage_path: PathBuf,
    active_session_id: Option<String>,
}

impl MemoryManager {
    pub fn new() -> Self {
        let storage_path = dirs::data_dir()
            .map(|d| d.join("oben").join("sessions"))
            .unwrap_or_else(|| PathBuf::from("~/.local/share/oben/sessions"));

        Self {
            sessions: HashMap::new(),
            storage_path,
            active_session_id: None,
        }
    }

    /// Get or create a session.
    pub fn get_or_create_session(&mut self, name: &str) -> &mut Session {
        let key = self.find_session_key_by_name(name);
        match key {
            Some(key) => self.sessions.get_mut(&key).unwrap(),
            None => {
                let session = Session::new(name);
                let id = session.id.clone();
                self.sessions.insert(id.clone(), session);
                self.active_session_id = Some(id.clone());
                self.sessions.get_mut(&id).unwrap()
            }
        }
    }

    fn find_session_key_by_name(&self, name: &str) -> Option<String> {
        self.sessions.iter().find(|(_, s)| s.name == name).map(|(k, _)| k.clone())
    }

    /// Start a new session.
    pub fn new_session(&mut self, name: &str) -> &mut Session {
        let session = Session::new(name);
        let id = session.id.clone();
        self.sessions.insert(id.clone(), session);
        self.active_session_id = Some(id);
        self.sessions.get_mut(self.active_session_id.as_ref().unwrap()).unwrap()
    }

    /// Switch to an existing session.
    pub fn switch_session(&mut self, session_id: &str) -> Result<&mut Session> {
        self.active_session_id = Some(session_id.to_string());
        self.sessions.get_mut(session_id).ok_or_else(|| anyhow::anyhow!("Session not found: {}", session_id))
    }

    /// Get current active session.
    pub fn active_session(&self) -> Option<&Session> {
        self.active_session_id.as_ref().and_then(|id| self.sessions.get(id))
    }

    /// Get current active session (mutable).
    pub fn active_session_mut(&mut self) -> Option<&mut Session> {
        self.active_session_id.as_ref().and_then(|id| self.sessions.get_mut(id))
    }

    /// List all sessions.
    pub fn list_sessions(&self) -> Vec<&Session> {
        self.sessions.values().collect()
    }

    /// Save all sessions to disk.
    pub fn save(&self) -> Result<()> {
        std::fs::create_dir_all(&self.storage_path)?;
        for (id, session) in &self.sessions {
            let path = self.storage_path.join(format!("{}.json", id));
            let content = serde_json::to_string_pretty(session)?;
            std::fs::write(path, content)?;
        }
        info!("Saved {} sessions to {}", self.sessions.len(), self.storage_path.display());
        Ok(())
    }

    /// Load sessions from disk.
    pub fn load(&mut self) -> Result<()> {
        if !self.storage_path.exists() {
            return Ok(());
        }
        let entries = std::fs::read_dir(&self.storage_path)?;
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let content = std::fs::read_to_string(&path)?;
            let session: Session = serde_json::from_str(&content)?;
            self.sessions.insert(session.id.clone(), session);
        }
        if let Some(last) = self.sessions.values().max_by_key(|s| s.updated_at) {
            self.active_session_id = Some(last.id.clone());
        }
        info!("Loaded {} sessions from {}", self.sessions.len(), self.storage_path.display());
        Ok(())
    }

    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }
}

impl Default for MemoryManager {
    fn default() -> Self {
        Self::new()
    }
}
