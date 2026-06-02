/// Integration tests for session rotation on compression (S.9).
///
/// Tests the TurnExecutor → SessionManager rotation flow using only
/// public API — this is the integration tier per AGENTS.md.

use oben_agent::compact_context::CompactContextEngine;
use oben_agent::compact::CompactCofig;
use oben_agent::context::ContextEngine;
use oben_agent::turn_executor::TurnExecutor;
use oben_models::{CallMode, Message, TransportProvider};
use oben_sessions::SessionManager;
use std::sync::Arc;
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
        })
    }

    async fn stream_chat(
        &self,
        _messages: &[Message],
        _mode: &CallMode,
        _callback: oben_models::StreamDeltaCallback,
    ) -> Result<oben_models::TransportResponse, anyhow::Error> {
        Ok(oben_models::TransportResponse {
            text: "mock response".to_string(),
            tool_calls: vec![],
            tokens_used: Some(15),
        })
    }
}

/// Create a context engine pre-loaded with a high token count so that
/// `should_compact` returns true immediately (bypassing message estimation).
fn make_compacting_engine() -> CompactContextEngine {
    let mut engine = CompactContextEngine::with_config(CompactCofig {
        context_length: 100_000,
        threshold_percent: 0.75,
        tail_token_budget: 50, // Very small tail → middle always non-empty
        tail_overhead: 1.3,
        ..Default::default()
    });
    // Set last_total_tokens above threshold so should_compact returns true
    engine.update_from_response(60_000, 40_000, 100_001);
    engine
}

// ── Helper: create a session with many messages ────────────────────────

fn create_populated_session(mgr: &mut SessionManager, msg_count: usize) -> String {
    let session = mgr.new_session("test-chat");
    for i in 0..msg_count {
        let msg = if i % 2 == 0 {
            Message::user(format!(
                "This is user message number {}. It is a long conversational turn that spans multiple sentences and contains sufficient content to exceed the default tail budget and force middle message compression during session compaction.",
                i
            ))
        } else {
            Message::assistant(format!(
                "This is assistant message number {}. It provides a thorough response with detailed explanations and analysis that exceeds the default tail budget and ensures meaningful compression results during session rotation.",
                i
            ))
        };
        session.messages.push(msg);
    }
    session.id.clone()
}

// ── Integration tests ─────────────────────────────────────────────────

/// Tests that TurnExecutor triggers session rotation when compaction fires.
///
/// given: a context engine with last_total_tokens exceeding the compaction
///        threshold, and a session with messages
/// when: execute_turn is called
/// then: parent session ends with reason "compression", child session created
///       with parent_session_id lineage
#[tokio::test]
async fn test_turn_exec_rotates_session_on_compaction() {
    let mut mgr = SessionManager::new_with_path(make_test_dir().path().join("rot-test")).unwrap();
    let session_id = create_populated_session(&mut mgr, 1500);

    let mut context_engine = make_compacting_engine();
    let transport = MockTransport;
    let tools = Arc::new(oben_tools::ToolRegistry::new());

    // Verify parent session is not ended before the turn
    let parent_session = mgr.session(&session_id).unwrap();
    assert!(
        parent_session.metadata.end_reason.is_none(),
        "parent should not be ended before turn"
    );

    // Execute a turn — should_compact returns true because last_total_tokens
    // (100_001) exceeds the default threshold (~96K).
    let result = TurnExecutor::execute_turn(
        &mut context_engine,
        &transport,
        &tools,
        &mut mgr,
        &session_id,
        Message::user("new user message"),
        &CallMode::Fresh(session_id.clone()),
        None,
    )
    .await;

    // Turn should complete successfully (mock transport returns immediately)
    assert!(result.is_ok(), "turn should succeed with mock transport");

    // Parent session should be ended with compression
    let parent_session = mgr.session(&session_id).unwrap();
    assert_eq!(
        parent_session.metadata.end_reason.as_deref(),
        Some("compression"),
        "parent should be ended with compression reason"
    );

    // Child session should exist with parent_session_id lineage
    let child_id = mgr.active_session_id().unwrap();
    assert_ne!(
        child_id, session_id,
        "active session should be the child, not the parent"
    );

    let child_session = mgr.session(&child_id).unwrap();
    assert_eq!(
        child_session.metadata.parent_session_id.as_deref(),
        Some(session_id.as_str()),
        "child should reference parent via parent_session_id"
    );
}

/// Tests that session rotation updates the active session.
///
/// given: a parent session with messages
/// when: execute_turn triggers compaction and rotation
/// then: active_session_id points to the new child with auto-numbered title
#[tokio::test]
async fn test_rotation_updates_active_session() {
    let mut mgr = SessionManager::new_with_path(make_test_dir().path().join("active-test")).unwrap();
    let parent_id = create_populated_session(&mut mgr, 1500);

    let mut context_engine = make_compacting_engine();
    let transport = MockTransport;
    let tools = Arc::new(oben_tools::ToolRegistry::new());

    // Initial active session is the parent
    assert_eq!(mgr.active_session_id().unwrap(), parent_id);

    TurnExecutor::execute_turn(
        &mut context_engine,
        &transport,
        &tools,
        &mut mgr,
        &parent_id,
        Message::user("test message"),
        &CallMode::Fresh(parent_id.clone()),
        None,
    )
    .await
    .unwrap();

    // After rotation, active session should be the child
    let child_id = mgr.active_session_id().unwrap();
    assert_ne!(child_id, parent_id);

    // Child should have the auto-numbered title "test-chat (2)"
    let child = mgr.session(&child_id).unwrap();
    assert!(
        child.metadata.title.as_deref().unwrap_or("").contains(" (2)"),
        "child title should be auto-numbered, got: {:?}",
        child.metadata.title
    );
}

/// Tests that multiple sequential rotations produce correct numbering.
///
/// given: a context engine that always triggers compaction
/// when: two consecutive turns each fire compaction
/// then: second child is titled "test-chat (3)" (incrementing from (2))
#[tokio::test]
async fn test_multiple_rotations_increment_numbering() {
    let mut mgr = SessionManager::new_with_path(make_test_dir().path().join("multi-test")).unwrap();
    let session_id = create_populated_session(&mut mgr, 1500);

    let mut context_engine = make_compacting_engine();
    let transport = MockTransport;
    let tools = Arc::new(oben_tools::ToolRegistry::new());

    // First turn — creates child "test-chat (2)"
    TurnExecutor::execute_turn(
        &mut context_engine,
        &transport,
        &tools,
        &mut mgr,
        &session_id,
        Message::user("first turn"),
        &CallMode::Fresh(session_id.clone()),
        None,
    )
    .await
    .unwrap();

    let child1_id = mgr.active_session_id().unwrap();
    let child1 = mgr.session(&child1_id).unwrap();
    assert!(
        child1.metadata.title.as_deref().unwrap_or("").contains(" (2)"),
        "first child should be titled (2), got: {:?}",
        child1.metadata.title
    );

    // Reset engine so it triggers again
    let mut context_engine2 = make_compacting_engine();

    // Second turn — should create child "test-chat (3)"
    TurnExecutor::execute_turn(
        &mut context_engine2,
        &transport,
        &tools,
        &mut mgr,
        &child1_id,
        Message::user("second turn"),
        &CallMode::Fresh(child1_id.clone()),
        None,
    )
    .await
    .unwrap();

    let child2_id = mgr.active_session_id().unwrap();
    assert_ne!(child2_id, child1_id, "child2 should have different ID from child1");
    let child2 = mgr.session(&child2_id).unwrap();
    assert!(
        child2.metadata.title.as_deref().unwrap_or("").contains(" (3)"),
        "second child should be titled (3), got: {:?}",
        child2.metadata.title
    );
}

/// Tests that rotation failure (e.g., already-ended session) does not panic.
///
/// given: a session that has already been ended
/// when: execute_turn triggers compaction
/// then: rotation failure is logged but the turn does not panic
#[tokio::test]
async fn test_rotation_failure_does_not_panic() {
    let mut mgr = SessionManager::new_with_path(make_test_dir().path().join("fail-test")).unwrap();
    let session_id = create_populated_session(&mut mgr, 300);

    let mut context_engine = make_compacting_engine();
    let transport = MockTransport;
    let tools = Arc::new(oben_tools::ToolRegistry::new());

    // First rotation succeeds
    TurnExecutor::execute_turn(
        &mut context_engine,
        &transport,
        &tools,
        &mut mgr,
        &session_id,
        Message::user("first turn"),
        &CallMode::Fresh(session_id.clone()),
        None,
    )
    .await
    .unwrap();

    // Now the parent is already ended. A second rotation on it should fail
    // gracefully (the split_after_compression tries to end_session again).
    // We don't assert the specific error — just that no panic occurs.
    let mut context_engine2 = make_compacting_engine();
    let result = TurnExecutor::execute_turn(
        &mut context_engine2,
        &transport,
        &tools,
        &mut mgr,
        &session_id,
        Message::user("second turn"),
        &CallMode::Fresh(session_id.clone()),
        None,
    )
    .await;

    // The result may be Err or Ok depending on what happens,
    // but we just verify no panic occurred.
    let _ = result;
}
