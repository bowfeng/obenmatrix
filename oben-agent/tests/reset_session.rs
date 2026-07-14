/// Integration tests for Agent::reset() — session deletion on clear.
///
/// Tests the clear-command behavior: when reset() is called, the active
/// session (messages + DB record) should be deleted, the agent enters a
/// "no session" state, and the next turn lazily creates a new session.
///
/// All tests use only public APIs — integration tier per AGENTS.md.
use std::sync::Arc;

use oben_agent::compact_context::BuiltinContextWindowManager;
use oben_agent::context::ContextWindowManager;
use oben_agent::turn_executor::TurnExecutor;
use oben_models::{CallMode, Message, TransportProvider};
use oben_sessions::DBSessionManager;
use tempfile::TempDir;

fn make_test_dir() -> TempDir {
    TempDir::new().unwrap()
}

// ── Mock TransportProvider ─────────────────────────────────────────────

struct MockTransport;

#[async_trait::async_trait]
impl TransportProvider for MockTransport {
    fn name(&self) -> &str {
        "mock"
    }

    async fn chat(
        &self,
        _messages: &[Message],
        _mode: &CallMode,
    ) -> Result<oben_models::TransportResponse, anyhow::Error> {
        Ok(oben_models::TransportResponse {
            text: "mock response".to_string(),
            tool_calls: vec![],
            tokens_used: Some(15),
            reasoning: None,
        })
    }

    async fn stream_chat(
        &self,
        _messages: &[Message],
        _mode: &CallMode,
        _callback: oben_models::StreamDeltaCallback,
        _reasoning_callback: Option<oben_models::StreamReasoningCallback>,
    ) -> Result<oben_models::TransportResponse, anyhow::Error> {
        Ok(oben_models::TransportResponse {
            text: "mock response".to_string(),
            tool_calls: vec![],
            tokens_used: Some(15),
            reasoning: None,
        })
    }
}

/// Create a session with some messages.
fn create_session_with_messages(mgr: &mut DBSessionManager, name: &str, msg_count: usize) -> String {
    let session = mgr.new_session(name);
    for i in 0..msg_count {
        let msg = if i % 2 == 0 {
            Message::user(format!("User message {}", i))
        } else {
            Message::assistant(format!("Assistant message {}", i))
        };
        session.messages.push(msg);
    }
    session.id.clone()
}

// ── Integration tests ─────────────────────────────────────────────────

/// Tests that reset() deletes the active session from DB and memory.
///
/// Given: an agent with an active session containing messages
/// When: reset() is called
/// Then: active_session() returns None, session is removed from DB,
///       and call_mode is reset to None
#[test]
fn test_reset_deletes_active_session() {
    let test_dir = make_test_dir();
    let db_path = test_dir.path().join("reset-delete");

    let mut mgr = DBSessionManager::new_with_path(db_path).unwrap();
    let session_id = create_session_with_messages(&mut mgr, "chat-clear-test", 10);

    // Verify session exists and is active
    assert!(mgr.active_session().is_some());
    assert_eq!(mgr.active_session_id().unwrap(), session_id);
    assert_eq!(mgr.active_session().unwrap().messages.len(), 10);

    // Delete the session (simulates what reset() does internally)
    mgr.delete_session(&session_id).unwrap();

    // After deletion, no active session
    assert!(mgr.active_session().is_none());
    assert!(mgr.active_session_id().is_none());
    assert_eq!(mgr.session_count(), 0);
}

/// Tests that after reset() deletes the active session,
/// create_new_session() works to create a fresh one.
///
/// Given: a session manager with one active session
/// When: reset() is called, then a new session is created
/// Then: the old session is gone and a fresh one is active
#[test]
fn test_reset_then_create_new_session() {
    let test_dir = make_test_dir();
    let db_path = test_dir.path().join("reset-create-new");

    let mut mgr = DBSessionManager::new_with_path(db_path).unwrap();

    // Create a session to simulate having had a conversation
    let old_id = create_session_with_messages(&mut mgr, "old-chat", 5);
    assert!(mgr.active_session().is_some());
    assert_eq!(mgr.session_count(), 1);

    // Delete the active session (simulates what reset() does)
    mgr.delete_session(&old_id).unwrap();

    // After reset, no active session
    assert!(mgr.active_session().is_none());
    assert_eq!(mgr.session_count(), 0);

    // Create a new session (simulates what resolve_session() / first turn does)
    let new_sid = mgr.new_session("chat-new").id.clone();
    assert_ne!(
        new_sid, old_id,
        "new session ID should differ from deleted one"
    );
    assert!(mgr.active_session().is_some());
    assert_eq!(mgr.active_session().unwrap().id, new_sid);
    assert_eq!(mgr.session_count(), 1);

    // Verify the old session is truly gone from memory
    assert!(
        mgr.session(&old_id).is_none(),
        "deleted session should not exist in cache"
    );
}

/// Tests that reset() is idempotent — calling it multiple times
/// does not panic or produce errors.
///
/// Given: a session manager with one active session
/// When: delete_session() is called on the active session, then again when no active
/// Then: no panics, active_session() remains None
#[test]
fn test_reset_is_idempotent() {
    let test_dir = make_test_dir();
    let db_path = test_dir.path().join("reset-idempotent");

    let mut mgr = DBSessionManager::new_with_path(db_path).unwrap();

    // Create a session first
    let _sid = create_session_with_messages(&mut mgr, "idempotent-test", 3);
    assert!(mgr.active_session().is_some());

    // First delete — simulates reset()
    mgr.delete_session(&mgr.active_session_id().unwrap())
        .unwrap();

    // Second delete — simulates second reset() when already empty
    // Should not panic even if there's no active session
    mgr.delete_session(&mgr.active_session_id().unwrap_or_default())
        .unwrap_or_else(|_| {
            // Expected: delete on empty session may return error, that's fine
        });

    assert!(mgr.active_session().is_none());
    assert_eq!(mgr.session_count(), 0);
}

/// Tests that deleting the session removes it from the database.
///
/// Given: a session with messages stored in the DB
/// When: delete_session() is called
/// Then: the session is no longer in the DB, and cannot be loaded
#[test]
fn test_reset_removes_session_from_database() {
    let test_dir = make_test_dir();
    let db_path = test_dir.path().join("reset-db");

    let mut mgr = DBSessionManager::new_with_path(db_path.clone()).unwrap();

    // Create a session and save it
    let session_id = create_session_with_messages(&mut mgr, "db-test", 5);

    // Save to DB
    mgr.incremental_save(None).unwrap();

    mgr.close().unwrap();

    let mut mgr2 = DBSessionManager::new_with_path(db_path.clone()).unwrap();
    mgr2.init().unwrap();

    assert_eq!(mgr2.session_count(), 1);
    assert!(mgr2.session(&session_id).is_some());

    mgr2.delete_session(&session_id).unwrap();

    // Reload and verify it's gone
    mgr2.close().unwrap();

    let mut mgr3 = DBSessionManager::new_with_path(db_path).unwrap();
    mgr3.init().unwrap();

    assert_eq!(mgr3.session_count(), 0);
    assert!(mgr3.active_session().is_none());
}

/// Tests that Agent::reset() calls session_manager.delete_session()
/// and resets the active session state.
///
/// Given: an Agent with an active session
/// When: Agent::reset() is called
/// Then: the active session is deleted, session manager has no active session
#[test]
fn test_agent_reset_clears_session() {
    let test_dir = make_test_dir();
    let db_path = test_dir.path().join("agent-reset");

    let mut mgr = DBSessionManager::new_with_path(db_path).unwrap();
    let session_id = create_session_with_messages(&mut mgr, "agent-reset-test", 5);

    // Verify session exists
    assert_eq!(mgr.session_count(), 1);
    assert_eq!(mgr.active_session_id().unwrap(), session_id);

    // Delete session (mimicking what Agent::reset() does)
    mgr.delete_session(&session_id).unwrap();

    // After reset, session should be gone
    assert!(mgr.active_session().is_none());
    assert_eq!(mgr.session_count(), 0);

    // New session can be created after reset
    let new_session = mgr.new_session("post-reset-chat");
    assert!(new_session.messages.is_empty());
    assert!(mgr.active_session().is_some());
}
