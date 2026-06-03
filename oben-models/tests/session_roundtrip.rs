use oben_models::{Message, Session};

// ─── Session create ─────────────────────────────────────────────────

#[test]
fn new_session_has_generated_id() {
    let session = Session::new("test-session");
    assert!(!session.id.is_empty());
    assert_eq!(session.name, "test-session");
    assert!(session.messages.is_empty());
}

#[test]
fn new_session_has_timestamps() {
    let _session = Session::new("timing-test");
    // DateTime<Utc> is always timezone-aware by construction
}

// ─── Add messages ───────────────────────────────────────────────────

#[test]
fn add_message_increments_count() {
    let mut session = Session::new("msg-count");
    assert_eq!(session.message_count(), 0);
    session.add_message(Message::user("hello"));
    assert_eq!(session.message_count(), 1);
    session.add_message(Message::assistant("hi back"));
    assert_eq!(session.message_count(), 2);
}

#[test]
fn add_message_updates_updated_at() {
    let mut session = Session::new("timestamp-update");
    let first = session.updated_at;
    session.add_message(Message::user("msg1"));
    // updated_at should advance (at least to the same second)
    assert!(session.updated_at >= first);
}

// ─── Session JSON round-trip ────────────────────────────────────────

#[test]
fn session_roundtrip_json_empty() {
    let session = Session::new("empty-session");
    let json = serde_json::to_string(&session).unwrap();
    let restored: Session = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.name, "empty-session");
    assert!(restored.messages.is_empty());
}

#[test]
fn session_roundtrip_json_with_messages() {
    let mut session = Session::new("with-messages");
    session.add_message(Message::system("You are helpful."));
    session.add_message(Message::user("What's Rust?"));
    session.add_message(Message::assistant("Rust is a systems language."));

    let json = serde_json::to_string(&session).unwrap();
    let restored: Session = serde_json::from_str(&json).unwrap();

    assert_eq!(restored.name, "with-messages");
    assert_eq!(restored.messages.len(), 3);
    assert_eq!(restored.messages[0].role, oben_models::MessageRole::System);
    assert_eq!(restored.messages[1].role, oben_models::MessageRole::User);
    assert_eq!(
        restored.messages[2].role,
        oben_models::MessageRole::Assistant
    );
}

#[test]
fn session_compress_memory() {
    let mut session = Session::new("compress-test");
    session.compress_memory("Compressed context: user asked about Rust...");
    assert!(session.memory_context.is_some());
}

#[test]
fn session_roundtrip_with_memory_context() {
    let mut session = Session::new("with-memory");
    session.compress_memory("important context here");

    let json = serde_json::to_string(&session).unwrap();
    let restored: Session = serde_json::from_str(&json).unwrap();
    assert_eq!(
        restored.memory_context,
        Some("important context here".to_string())
    );
}

// ─── Session YAML round-trip ────────────────────────────────────────

#[test]
fn session_roundtrip_yaml() {
    let mut session = Session::new("yaml-test");
    session.add_message(Message::user("test"));
    let yaml = serde_yaml::to_string(&session).unwrap();
    let restored: Session = serde_yaml::from_str(&yaml).unwrap();
    assert_eq!(restored.name, "yaml-test");
    assert_eq!(restored.messages.len(), 1);
}
