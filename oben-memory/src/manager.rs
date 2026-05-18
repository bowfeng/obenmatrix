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

    pub fn new_with_path(storage_path: PathBuf) -> Self {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_dir() -> PathBuf {
        let dir = tempfile::tempdir().unwrap();
        dir.path().join("sessions")
    }

    #[test]
    fn test_manager_creates_session() {
        let path = make_test_dir();
        let mut mgr = MemoryManager::new_with_path(path);
        let session = mgr.new_session("test-session");
        assert_eq!(session.name, "test-session");
        assert!(!session.id.is_empty());
        assert_eq!(mgr.session_count(), 1);
    }

    #[test]
    fn test_manager_list_sessions() {
        let path = make_test_dir();
        let mut mgr = MemoryManager::new_with_path(path);
        mgr.new_session("s1");
        mgr.new_session("s2");
        assert_eq!(mgr.list_sessions().len(), 2);
    }

    #[test]
    fn test_manager_get_or_create_reuses_existing() {
        let path = make_test_dir();
        let mut mgr = MemoryManager::new_with_path(path);
        let s1 = mgr.get_or_create_session("my-session");
        s1.add_message(Message::user("first"));
        let s2 = mgr.get_or_create_session("my-session");
        assert_eq!(s2.name, "my-session");
        assert_eq!(s2.message_count(), 1); // reused, not recreated
    }

    #[test]
    fn test_save_and_load_roundtrip() {
        let path = make_test_dir();
        let mut mgr = MemoryManager::new_with_path(path.clone());
        let session = mgr.new_session("persist-test");
        session.add_message(Message::user("hello"));
        session.add_message(Message::assistant("hi there"));
        let count = mgr.session_count();

        // Save in-memory state
        mgr.save().unwrap();

        // Create a fresh manager pointing to same path
        let mut mgr2 = MemoryManager::new_with_path(path.clone());
        mgr2.load().unwrap();

        assert_eq!(mgr2.session_count(), count);
        let loaded = mgr2.list_sessions().into_iter().next().unwrap();
        assert_eq!(loaded.name, "persist-test");
        assert_eq!(loaded.message_count(), 2);
    }

    #[test]
    fn test_switch_session() {
        let path = make_test_dir();
        let mut mgr = MemoryManager::new_with_path(path);
        let s1 = mgr.new_session("s1");
        let s1_id = s1.id.clone();
        let _s2 = mgr.new_session("s2");
        assert!(mgr.active_session().unwrap().name == "s2");
        let switched = mgr.switch_session(&s1_id).unwrap();
        assert_eq!(switched.name, "s1");
    }

    #[test]
    fn test_switch_to_nonexistent_session_fails() {
        let path = make_test_dir();
        let mut mgr = MemoryManager::new_with_path(path);
        let err = mgr.switch_session("nonexistent-id").unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn test_load_empty_directory() {
        let path = make_test_dir();
        // Don't create any session files, just test loading empty dir
        let mgr = MemoryManager::new_with_path(path.clone());
        mgr.save().unwrap(); // Creates the directory
        let mut mgr2 = MemoryManager::new_with_path(path.clone());
        mgr2.load().unwrap();
        assert_eq!(mgr2.session_count(), 0);
    }
}
