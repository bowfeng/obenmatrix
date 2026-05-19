/// Session storage and memory management.
/// Maps to `agent/memory_manager.py`.
///
/// Layout: `{storage_path}/{session_id}/meta.json` — session metadata
/// `{storage_path}/{session_id}/raw.jsonl` — one message per line
/// `{storage_path}/{session_id}/summary.jsonl` — summary chunks with from/to indices

use anyhow::Result;
use oben_models::{Message, Session, SummaryChunk};
use std::collections::HashMap;
use std::io::Write;
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
                .map(|d| d.join(".oben").join("memory"))
                .unwrap_or_else(|| PathBuf::from("~/.oben"));
        Self::new_with_path(storage_path)
    }

    pub fn new_with_path(storage_path: PathBuf) -> Self {
        let mut mgr = Self {
            sessions: HashMap::new(),
            storage_path,
            active_session_id: None,
        };
        // Discover sessions from disk
        let _ = mgr.init();
        mgr
    }

    /// Discover all sessions from disk by reading only `meta.json`.
    ///
    /// Scans the storage path for directories containing `meta.json`,
    /// deserializes the metadata, and populates `sessions` with session
    /// info (id, name, timestamps, etc.). Messages are NOT loaded,
    /// so `persisted_message_count` is set to 0 — the raw JSONL and
    /// index files will be read later by a full `load()` call.
    ///
    /// If the storage path does not exist yet, does nothing.
    pub fn init(&mut self) -> Result<()> {
        if !self.storage_path.exists() {
            return Ok(());
        }
        let entries = std::fs::read_dir(&self.storage_path)?;
        let mut found = 0;
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let session_id = path
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(|| anyhow::anyhow!("Invalid session directory"))?;

            let meta_path = path.join("meta.json");
            if !meta_path.exists() {
                continue;
            }
            let content = std::fs::read_to_string(&meta_path)?;
            let mut metadata: serde_json::Value = serde_json::from_str(&content)?;

            // Build a lightweight session from metadata only.
            // Clear message-related fields so they don't carry stale data.
            if let Some(obj) = metadata.as_object_mut() {
                obj.remove("messages");
                obj.remove("summary_chunks");
                obj.remove("persisted_message_count");
            }
            let mut session: Session = serde_json::from_value(metadata)?;
            session.persisted_message_count = 0;

            self.sessions.insert(session_id.to_string(), session);
            found += 1;
        }
        if let Some(last) = self.sessions.values().max_by_key(|s| s.updated_at) {
            self.active_session_id = Some(last.id.clone());
        }
        info!("Initialized {} sessions from {}", found, self.storage_path.display());
        Ok(())
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
    /// Saves the current active session's messages to disk first, then loads
    /// the target session's messages from raw.jsonl.
    pub fn switch_session(&mut self, session_id: &str) -> Result<&mut Session> {
        // Save current active session if different from target
        let current_active = self.active_session_id.clone();
        if let Some(ref active_id) = current_active {
            if active_id != session_id {
                self.save(Some(active_id))?;
            }
        }
        // Load target session's messages from disk
        self.load(Some(session_id))?;
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

    /// Save a session's messages to disk.
    ///
    /// If `session_id` is `None`, saves the active session. If `Some(id)`,
    /// saves that specific session regardless of what is active.
    pub fn save(&mut self, session_id: Option<&str>) -> Result<()> {
        let sid = match session_id {
            Some(id) => id.to_string(),
            None => match &self.active_session_id {
                Some(id) => id.clone(),
                None => {
                    info!("No active session to save");
                    return Ok(());
                }
            },
        };
        let session = self.sessions.get(&sid).ok_or_else(|| anyhow::anyhow!("Session not found: {}", sid))?;
        let session = session.clone();
        let new_count = session.messages.len();
        self.save_session_to_file(&session)?;
        if let Some(s) = self.sessions.get_mut(&sid) {
            s.persisted_message_count = new_count;
        }
        info!("Saved session '{}'", sid);
        Ok(())
    }

    /// Load a session's messages from disk.
    ///
    /// If `session_id` is `None`, loads messages for all known sessions.
    /// If `Some(id)`, loads only that session's messages from raw.jsonl.
    pub fn load(&mut self, session_id: Option<&str>) -> Result<()> {
        let targets: Vec<String> = match session_id {
            Some(id) => vec![id.to_string()],
            None => self.sessions.keys().cloned().collect(),
        };
        for sid in targets {
            self.load_session_messages(&sid)?;
        }
        if let Some(last) = self.sessions.values().max_by_key(|s| s.updated_at) {
            self.active_session_id = Some(last.id.clone());
        }
        Ok(())
    }

    /// Load messages from raw.jsonl + summary.jsonl for one session.
    fn load_session_messages(&mut self, session_id: &str) -> Result<()> {
        let session = match self.sessions.get_mut(session_id) {
            Some(s) => s,
            None => return Ok(()),
        };

        let session_dir = self.storage_path.join(session_id);
        let raw_path = session_dir.join("raw.jsonl");
        if !raw_path.exists() {
            return Ok(());
        }

        let raw_messages: Vec<Message> = {
            let lines = std::fs::read_to_string(&raw_path)?;
            lines
                .lines()
                .filter(|l| !l.is_empty())
                .filter_map(|line| serde_json::from_str::<Message>(line).ok())
                .collect()
        };

        // Load summary chunks from disk and rebuild the message list.
        // Each summary chunk replaces the raw messages it covers,
        // so the final list = head messages + summaries + tail messages.
        let summary_path = session_dir.join("summary.jsonl");
        let mut chunks: Vec<SummaryChunk> = Vec::new();
        if summary_path.exists() {
            let summary_lines = std::fs::read_to_string(&summary_path)?;
            for line in summary_lines.lines().filter(|l| !l.trim().is_empty()) {
                if let Ok(chunk) = serde_json::from_str::<SummaryChunk>(line) {
                    chunks.push(chunk);
                }
            }
        }

        // Rebuild: insert summaries in place of skipped ranges.
        // `from` and `to` are 1-indexed, convert to 0-indexed for array access.
        let mut messages: Vec<Message> = Vec::new();
        let mut msg_idx: usize = 0;
        for chunk in &chunks {
            // Push head messages before this summary's range
            let from_0 = chunk.from.saturating_sub(1);
            while msg_idx < from_0 {
                messages.push(raw_messages[msg_idx].clone());
                msg_idx += 1;
            }
            // Insert the summary (replaces the skipped range)
            messages.push(Message::system(&chunk.summary));
            // Skip to end of this chunk (1-indexed, so to is exclusive in 0-indexed)
            msg_idx = chunk.to;
        }
        // Push remaining tail messages
        while msg_idx < raw_messages.len() {
            messages.push(raw_messages[msg_idx].clone());
            msg_idx += 1;
        }

        let chunk_count = chunks.len();
        // Restore summary_chunks on the session for future saves
        session.summary_chunks = chunks;

        let msg_count = messages.len();
        session.messages = messages;
        session.persisted_message_count = raw_messages.len();
        info!("Loaded {} messages ({} kept, {} summary) for session '{}'", 
            raw_messages.len(), msg_count, chunk_count, session_id);
        Ok(())
    }

    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Find session key by ID or name.
    pub fn find_key(&self, key: &str) -> Option<String> {
        // Exact ID match
        if self.sessions.contains_key(key) {
            return Some(key.to_string());
        }
        // Name match
        self.find_session_key_by_name(key)
    }

    /// Get all sessions as an iterator.
    pub fn sessions(&self) -> impl Iterator<Item = &Session> {
        self.sessions.values()
    }

    /// Get mutable reference to a session by key.
    pub fn session_mut_by_key(&mut self, key: &str) -> Option<&mut Session> {
        let id = self.find_key(key)?;
        self.sessions.get_mut(&id)
    }

    /// Get all sessions as immutable references (convenience for CLI).
    pub fn list_sessions_ref(&self) -> impl Iterator<Item = &Session> {
        self.sessions.values()
    }

    /// Get the storage path for this manager.
    pub fn storage_path(&self) -> &PathBuf {
        &self.storage_path
    }

    /// Remove a session by ID and delete its files on disk.
    pub fn remove_session(&mut self, session_id: &str) -> Result<()> {
        let session_dir = self.storage_path.join(session_id);
        if session_dir.exists() {
            std::fs::remove_dir_all(&session_dir)?;
        }
        self.sessions.remove(session_id);
        if self.active_session_id.as_deref() == Some(session_id) {
            self.active_session_id = None;
        }
        Ok(())
    }

    /// Remove the first session whose ID or name matches the given key.
    pub fn remove_session_by_key(&mut self, key: &str) -> Result<()> {
        let target = self.find_key(key)
            .ok_or_else(|| anyhow::anyhow!("Session not found: {}", key))?;
        self.remove_session(&target)
    }

    /// Save a single session to disk.
    pub fn save_session_to_file(&self, session: &Session) -> Result<()> {
        let session_dir = self.storage_path.join(&session.id);
        std::fs::create_dir_all(&session_dir)?;
        
        // Save meta.json (without messages, summary_chunks, and persisted_message_count)
        let metadata: serde_json::Value = {
            let mut m = serde_json::to_value(session)?;
            if let Some(obj) = m.as_object_mut() {
                obj.remove("messages");
                obj.remove("summary_chunks");
                obj.remove("persisted_message_count");
            }
            m
        };
        let meta_path = session_dir.join("meta.json");
        let content = serde_json::to_string_pretty(&metadata)?;
        std::fs::write(&meta_path, content)?;
        
        // Append new messages to raw.jsonl (append-only)
        if session.messages.len() > session.persisted_message_count {
            let raw_path = session_dir.join("raw.jsonl");
            
            // Collect new messages before opening files (avoid borrow conflicts)
            let new_messages: Vec<String> = session.messages[session.persisted_message_count..]
                .iter()
                .map(|msg| serde_json::to_string(msg).unwrap())
                .collect();
            
            // Append new messages to raw.jsonl
            {
                let mut file = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&raw_path)?;
                
                for line in &new_messages {
                    writeln!(file, "{}", line)?;
                }
            }
            
            info!("Appended {} new messages to session '{}' ({} total)", 
                session.messages.len() - session.persisted_message_count, session.id, session.messages.len());
        } else if !session_dir.join("raw.jsonl").exists() {
            // Create raw.jsonl if it doesn't exist yet
            let raw_path = session_dir.join("raw.jsonl");
            std::fs::File::create(&raw_path)?;
        }
        
        // Save summary.jsonl
        if !session.summary_chunks.is_empty() {
            let summary_path = session_dir.join("summary.jsonl");
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&summary_path)?;
            
            for chunk in &session.summary_chunks {
                let line = serde_json::to_string(chunk)?;
                use std::io::Write;
                writeln!(file, "{}", line)?;
            }
        }
        
        info!("Saved session '{}' to {}", session.id, session_dir.display());
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Session lifecycle (create, switch, delete)
// Maps to `hermes_cli/sessions.py`
// ---------------------------------------------------------------------------

/// Result of a session switch operation.
#[derive(Debug)]
pub struct SwitchResult {
    pub session_id: String,
    pub name: String,
    pub message_count: usize,
    pub was_new: bool,
}

/// Manage session lifecycle operations (create, switch, delete).
pub struct SessionManager {
    memory: MemoryManager,
}

impl SessionManager {
    /// Create a new session manager and load existing sessions.
    pub fn new() -> Result<Self> {
        Self::new_with_path(dirs::data_dir()
            .map(|d: PathBuf| d.join(".oben").join("memory"))
            .unwrap_or_else(|| PathBuf::from("~/.oben/memory")))
    }

    /// Create a new session manager with a custom storage path.
    pub fn new_with_path(storage_path: PathBuf) -> Result<Self> {
        let memory = MemoryManager::new_with_path(storage_path);
        Ok(Self { memory })
    }

    /// Load messages for a specific session, or all sessions if `session_id` is `None`.
    pub fn load(&mut self, session_id: Option<&str>) -> Result<()> {
        self.memory.load(session_id)
    }

    pub fn create(&mut self, name: &str) -> Result<&mut Session> {
        let session = self.memory.new_session(name);
        info!("Created session '{}' ({} messages)", session.name, session.message_count());
        Ok(session)
    }

    pub fn switch(&mut self, key: &str) -> Result<SwitchResult> {
        let session_id = self.memory.find_key(key)
            .ok_or_else(|| anyhow::anyhow!("Session not found: {}", key))?;
        let session = self.memory.switch_session(&session_id)?;
        Ok(SwitchResult {
            session_id,
            name: session.name.clone(),
            message_count: session.message_count(),
            was_new: false,
        })
    }

    /// Load messages for a specific session from disk.
    pub fn load_session(&mut self, session_id: &str) -> Result<()> {
        self.memory.load(Some(session_id))
    }

    pub fn switch_or_create(&mut self, key: &str) -> Result<SwitchResult> {
        if let Ok(sr) = self.switch(key) {
            return Ok(sr);
        }
        let session = self.create(key)?;
        Ok(SwitchResult {
            session_id: session.id.clone(),
            name: session.name.clone(),
            message_count: session.message_count(),
            was_new: true,
        })
    }

    pub fn list(&self) -> Vec<&Session> {
        self.memory.list_sessions()
    }

    pub fn delete(&mut self, key: &str) -> Result<String> {
        let session_id = self.memory.find_key(key)
            .ok_or_else(|| anyhow::anyhow!("Session not found: {}", key))?;
        self.memory.remove_session(&session_id)?;
        info!("Deleted session '{}'", session_id);
        Ok(session_id)
    }

    /// Save a session's messages to disk.
    ///
    /// If `session_id` is `None`, saves the active session. Otherwise saves that one.
    pub fn save(&mut self, session_id: Option<&str>) -> Result<()> {
        self.memory.save(session_id)
    }

    /// Save a specific session by ID or name.
    pub fn save_session(&self, key: &str) -> Result<()> {
        let id = self.memory.find_key(key)
            .ok_or_else(|| anyhow::anyhow!("Session not found: {}", key))?;
        if let Some(session) = self.memory.sessions.get(&id) {
            self.memory.save_session_to_file(session)
        } else {
            Err(anyhow::anyhow!("Session not found: {}", key))
        }
    }

    pub fn active(&self) -> Option<&Session> {
        self.memory.active_session()
    }

    pub fn session_mut(&mut self, key: &str) -> Option<&mut Session> {
        let id = self.memory.find_key(key)?;
        self.memory.session_mut_by_key(&id)
    }

    pub fn clone_session(&self, key: &str) -> Option<Session> {
        let id = self.memory.find_key(key)?;
        self.memory.sessions().find(|s| s.id == id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_dir() -> PathBuf {
        tempfile::tempdir().unwrap().path().join("sessions")
    }

    // --- MemoryManager tests ---

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
        mgr.save(None).unwrap();

        // Create a fresh manager pointing to same path
        let mut mgr2 = MemoryManager::new_with_path(path.clone());
        mgr2.load(None).unwrap();

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
        s1.add_message(Message::user("msg in s1"));
        let s1_id = s1.id.clone();
        let s2 = mgr.new_session("s2");
        s2.add_message(Message::user("msg in s2"));
        assert!(mgr.active_session().unwrap().name == "s2");
        let switched = mgr.switch_session(&s1_id).unwrap();
        assert_eq!(switched.name, "s1");
        // s1's messages were loaded from disk
        assert_eq!(switched.message_count(), 1);
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
        let mut mgr = MemoryManager::new_with_path(path.clone());
        mgr.save(None).unwrap();
        let mut mgr2 = MemoryManager::new_with_path(path.clone());
        mgr2.load(None).unwrap();
        assert_eq!(mgr2.session_count(), 0);
    }

    #[test]
    fn test_summary_chunk_skips_messages() {
        let path = make_test_dir();
        let mut mgr = MemoryManager::new_with_path(path.clone());
        let session = mgr.new_session("summary-test");
        session.add_message(Message::user("msg1"));
        session.add_message(Message::assistant("msg2"));
        session.add_message(Message::user("msg3"));
        session.add_message(Message::assistant("msg4"));
        session.summary_chunks.push(SummaryChunk {
            from: 1,
            to: 4,
            summary: "compressed all messages".to_string(),
        });
        
        mgr.save(None).unwrap();
        
        // Load with summary chunks
        let mut mgr2 = MemoryManager::new_with_path(path.clone());
        mgr2.load(None).unwrap();
        
        let loaded = mgr2.list_sessions().into_iter().next().unwrap();
        assert_eq!(loaded.message_count(), 1); // 1 summary replaces all 4
    }

    #[test]
    fn test_summary_chunk_partial_skip() {
        let path = make_test_dir();
        let mut mgr = MemoryManager::new_with_path(path.clone());
        let session = mgr.new_session("partial-test");
        session.add_message(Message::user("msg1"));
        session.add_message(Message::assistant("msg2"));
        session.add_message(Message::user("msg3"));
        session.add_message(Message::assistant("msg4"));
        session.summary_chunks.push(SummaryChunk {
            from: 1,
            to: 3,
            summary: "compressed first 3 messages".to_string(),
        });
        
        mgr.save(None).unwrap();
        
        // Load with summary chunks
        let mut mgr2 = MemoryManager::new_with_path(path.clone());
        mgr2.load(None).unwrap();
        
        let loaded = mgr2.list_sessions().into_iter().next().unwrap();
        assert_eq!(loaded.message_count(), 2); // 1 summary + msg4
    }

    // --- SessionManager tests ---

    #[test]
    fn test_session_manager_create_and_list() {
        let mut mgr = SessionManager::new_with_path(make_test_dir()).unwrap();
        mgr.create("test-session").unwrap();
        assert_eq!(mgr.list().len(), 1);
        assert_eq!(mgr.list()[0].name, "test-session");
    }

    #[test]
    fn test_session_manager_switch_existing() {
        let mut mgr = SessionManager::new_with_path(make_test_dir()).unwrap();
        mgr.create("existing").unwrap();
        let result = mgr.switch("existing").unwrap();
        assert_eq!(result.name, "existing");
        assert!(!result.was_new);
    }

    #[test]
    fn test_session_manager_switch_or_create() {
        let mut mgr = SessionManager::new_with_path(make_test_dir()).unwrap();
        let result = mgr.switch_or_create("new-session").unwrap();
        assert_eq!(result.name, "new-session");
        assert!(result.was_new);
    }

    #[test]
    fn test_session_manager_delete() {
        let mut mgr = SessionManager::new_with_path(make_test_dir()).unwrap();
        mgr.create("to-delete").unwrap();
        assert_eq!(mgr.list().len(), 1);
        mgr.delete("to-delete").unwrap();
        assert_eq!(mgr.list().len(), 0);
    }

    #[test]
    fn test_session_manager_switch_nonexistent_fails() {
        let mut mgr = SessionManager::new_with_path(make_test_dir()).unwrap();
        let err = mgr.switch("nonexistent").unwrap_err();
        assert!(err.to_string().contains("not found"));
    }
}
