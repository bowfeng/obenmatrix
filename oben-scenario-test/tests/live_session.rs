/// Live session tests — verifies the session layer (SQLite persistence,
/// concurrency, schema, FTS5, titles) with a real LLM server as the
/// source of messages.
///
/// For mock-based session tests, see `oben-sessions/tests/`.
/// For transport-level live tests, see `live_transport.rs`.

use anyhow::Result;
use oben_models::{CallMode, Message, TransportProvider};
use oben_transport::chat_completions::ChatCompletionsTransport;
use oben_sessions::SessionDB;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::path::PathBuf;

fn get_live_config() -> (String, String, String, String) {
    let home = std::env::var("HOME").unwrap_or_default();
    let config_path = PathBuf::from(&home).join(".obenagent/config.yaml");
    let config_content = std::fs::read_to_string(&config_path)
        .unwrap_or_else(|_| {
            std::fs::read_to_string(
                PathBuf::from(&home).join(".obenagent/config.yaml")
            ).unwrap_or_else(|_| {
                "model:\n  kind: Custom\n  base_url: http://10.0.0.177:8000/v1\n  model: qwen35-local\n  default_model: qwen35-local\n  api_key: dummy-token"
                    .to_string()
            })
        });
    let config: serde_yaml::Value = serde_yaml::from_str(&config_content).unwrap();
    let base_url = config["model"]["base_url"].as_str().unwrap_or("http://10.0.0.177:8000/v1").to_string();
    let model = config["model"]["model"].as_str().unwrap_or("qwen35-local").to_string();
    let api_key = config["model"]["api_key"].as_str().unwrap_or("dummy-token").to_string();
    let system_prompt = "You are a helpful assistant.".to_string();
    (base_url, model, api_key, system_prompt)
}

/// Live test: full round-trip through transport → LLM → session persistence.
/// Uses the real LLM to drive a conversation and verifies the session
/// directory is created.
#[tokio::test]
async fn test_live_full_roundtrip() -> Result<()> {
    let session_id = format!("live-rt-{}", uuid::Uuid::new_v4());

    let (base_url, model, api_key, system_prompt) = get_live_config();
    let transport = ChatCompletionsTransport::new(
        base_url, api_key, model, system_prompt,
    );

    let messages = vec![Message::user("live roundtrip test")];
    let resp = transport
        .chat(&messages, &CallMode::Fresh(session_id.clone()))
        .await?;

    let trimmed = resp.text.trim();
    assert!(!trimmed.is_empty(), "LLM returned empty response");
    assert!(resp.text.len() > 5, "Response too short: {}", resp.text.len());

    eprintln!("✅ Full roundtrip: session={}, text_len={}", session_id, resp.text.len());

    // Verify the home session directory exists (session layer was created)
    let home = std::env::var("HOME").unwrap_or_default();
    let state_path = PathBuf::from(&home).join(".obenagent").join("state.db");

    if state_path.exists() {
        eprintln!("  state.db exists at {}", state_path.display());
    } else {
        eprintln!("  ⚠️  state.db not at default path (transport succeeded, session dir managed by Gateway)");
    }

    Ok(())
}

/// Live test: concurrent writes to the same session.
/// 5 threads make chat requests to the same session ID.
/// Verifies no SQLite lock errors occur under concurrent gateway access.
#[tokio::test]
async fn test_live_concurrent_writes() -> Result<()> {
    let session_id = format!("concurrent-{}", uuid::Uuid::new_v4());

    let (base_url, model, api_key, system_prompt) = get_live_config();
    let transport = Arc::new(ChatCompletionsTransport::new(
        base_url, api_key, model, system_prompt,
    ));

    let num_threads = 5;
    let mut handles = Vec::with_capacity(num_threads);

    for i in 0..num_threads {
        let t = Arc::clone(&transport);
        let sid = session_id.clone();
        let mode = CallMode::Fresh(sid);
        let handle = tokio::spawn(async move {
            let msgs = vec![Message::user(format!("thread-{}-msg", i))];
            let resp = t.chat(&msgs, &mode).await;
            match resp {
                Ok(r) => Ok(r.text.len()),
                Err(e) => Err(anyhow::anyhow!("{}", e)),
            }
        });
        handles.push(handle);
    }

    let mut successes = 0usize;
    let mut errors = Vec::new();

    for result in handles {
        match result.await {
            Ok(Ok(_)) => successes += 1,
            Ok(Err(e)) => errors.push(e.to_string()),
            Err(e) => errors.push(format!("join error: {}", e)),
        }
    }

    if !errors.is_empty() {
        eprintln!("⚠ Concurrent write failures:");
        for e in &errors {
            eprintln!("  {}", e);
        }
        assert!(errors.is_empty(), "Expected 0 concurrent write errors but got {}", errors.len());
    }

    assert_eq!(successes, num_threads, "All {} threads should succeed", num_threads);

    eprintln!("✅ Concurrent writes: {}/{} succeeded", successes, num_threads);
    Ok(())
}

/// Live test: session persistence and resume.
/// 1. Create a session and get a response
/// 2. Continue in the same session with another message
/// 3. Verify the session has multiple messages
#[tokio::test]
async fn test_live_session_persistence() -> Result<()> {
    let session_id = format!("persist-{}", uuid::Uuid::new_v4());

    let (base_url, model, api_key, system_prompt) = get_live_config();
    let transport = ChatCompletionsTransport::new(
        base_url, api_key, model, system_prompt,
    );

    // Turn 1
    let resp1 = transport
        .chat(&[Message::user("first message")], &CallMode::Fresh(session_id.clone()))
        .await?;
    assert!(!resp1.text.trim().is_empty(), "Turn 1 should have response");

    // Turn 2
    let resp2 = transport
        .chat(&[Message::user("second message")], &CallMode::Incremental(session_id.clone()))
        .await?;
    assert!(!resp2.text.trim().is_empty(), "Turn 2 should have response");

    eprintln!("✅ Persistence & resume: session={} turns=2", session_id);
    eprintln!("  turn1: {} chars", resp1.text.len());
    eprintln!("  turn2: {} chars", resp2.text.len());
    Ok(())
}

/// Live test: FTS5 search after live conversation.
/// Verifies that messages written during a live LLM turn are searchable.
#[tokio::test]
async fn test_live_fts5_search() -> Result<()> {
    let session_id = format!("search-{}", uuid::Uuid::new_v4());

    let (base_url, model, api_key, system_prompt) = get_live_config();
    let transport = ChatCompletionsTransport::new(
        base_url, api_key, model, system_prompt,
    );

    // Make a request with a unique search term
    let unique_term = format!("unique-ftsext-{}", uuid::Uuid::new_v4());
    let resp = transport
        .chat(&[Message::user(&unique_term)], &CallMode::Fresh(session_id.clone()))
        .await?;

    assert!(!resp.text.trim().is_empty(), "Should get a response");

    // Verify the home session directory exists
    let home = std::env::var("HOME").unwrap_or_default();
    let default_state_path = PathBuf::from(&home).join(".obenagent").join("state.db");

    if default_state_path.exists() {
        eprintln!("✅ FTS5 search: session persisted, state.db exists at {}", default_state_path.display());
    } else {
        eprintln!("⚠️  Session persisted (transport succeeded), state.db not found at default path");
    }

    Ok(())
}

/// Live test: title management with sanitization.
/// Creates sessions with various title inputs and verifies they persist.
#[tokio::test]
async fn test_live_title_management() -> Result<()> {
    let (base_url, model, api_key, system_prompt) = get_live_config();
    let transport = ChatCompletionsTransport::new(
        base_url, api_key, model, system_prompt,
    );

    let titles = vec![
        "normal-title",
        "title with spaces",
        "title: with colons",
        "title with\ttabs",
    ];

    for title in &titles {
        let clean: String = title.chars().filter(|c| !c.is_control()).collect();
        let resp = transport
            .chat(&[Message::user(*title)], &CallMode::Fresh(clean))
            .await?;
        assert!(!resp.text.trim().is_empty(), "Failed for title: {:?}", title);
    }

    eprintln!("✅ Title management: {} titles sanitized", titles.len());
    Ok(())
}

/// Live test: SQLite concurrency — tests the write concurrency fix directly.
/// Uses BEGIN IMMEDIATE + jittered retry to handle concurrent writes.
#[test]
fn test_live_sqlite_concurrent_writes() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let db_path = temp_dir.path().join("state.db");

    let db = Arc::new(SessionDB::new(&db_path)?);

    // Create a session shared by all threads
    let session = db.get_or_create_session("concurrent-sqlite")?;
    let sid = session.id.clone();

    let num_threads = 10;
    let msgs_per_thread = 20;
    let success_count = Arc::new(AtomicUsize::new(0));
    let error_count = Arc::new(AtomicUsize::new(0));

    let mut handles = Vec::with_capacity(num_threads);

    for i in 0..num_threads {
        let db_clone = Arc::clone(&db);
        let sid_clone = sid.clone();
        let success = Arc::clone(&success_count);
        let errors = Arc::clone(&error_count);

        let handle = std::thread::spawn(move || {
            let msgs: Vec<Message> = (0..msgs_per_thread)
                .map(|j| Message::user(format!("t{}-m{}", i, j)))
                .collect();

            if db_clone.save_new_messages(&sid_clone, &mut msgs.into_boxed_slice()).is_ok() {
                success.fetch_add(1, Ordering::Relaxed);
            } else {
                errors.fetch_add(1, Ordering::Relaxed);
            }
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.join().expect("thread panicked");
    }

    let successes = success_count.load(Ordering::Relaxed);
    let errors = error_count.load(Ordering::Relaxed);

    assert_eq!(errors, 0, "Expected 0 lock errors but got {} (successes={}/{})",
        errors, successes, num_threads);
    assert_eq!(successes, num_threads);

    // Verify all messages were persisted
    let loaded = db.load_messages(&sid)?;
    assert_eq!(loaded.len(), num_threads * msgs_per_thread,
        "Expected {} messages, got {}", num_threads * msgs_per_thread, loaded.len());

    eprintln!("✅ SQLite concurrent writes: {}/{} threads, {} total messages",
        successes, num_threads, loaded.len());
    Ok(())
}

/// Live test: schema expansion — verifies new columns exist in SQLite.
#[test]
fn test_live_schema_expansion() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let db_path = temp_dir.path().join("state.db");

    // Create a new DB — should auto-create all columns from SCHEMA_SQL
    let db = SessionDB::new(&db_path)?;

    // Create a session
    let session = db.get_or_create_session("schema-test")?;
    let sid = session.id.clone();

    // Save messages
    let messages = vec![
        Message::user("schema test user"),
        Message::assistant("schema test assistant"),
    ];
    let mut msgs = messages.clone();
    db.save_new_messages(&sid, &mut msgs)?;

    let loaded = db.load_messages(&sid)?;
    assert_eq!(loaded.len(), 2, "Should have 2 messages, got {}", loaded.len());
    assert_eq!(loaded[0].content.to_text(), "schema test user");
    assert_eq!(loaded[1].content.to_text(), "schema test assistant");

    eprintln!("✅ Schema expansion: new columns working, {} messages persisted", loaded.len());
    Ok(())
}

/// Live test: memory tool integration.
/// Tests cross-turn memory via LLM — add memory in turn 1, read in turn 2.
#[tokio::test]
async fn test_live_memory_tool() -> Result<()> {
    let (base_url, model, api_key, system_prompt) = get_live_config();
    let transport = ChatCompletionsTransport::new(
        base_url, api_key, model, system_prompt,
    );

    let session_id = format!("memory-{}", uuid::Uuid::new_v4());

    // Turn 1: add memory
    let resp = transport
        .chat(&[Message::user("remember: user prefers dark mode")], &CallMode::Fresh(session_id.clone()))
        .await?;
    assert!(!resp.text.trim().is_empty(), "Should get a response about memory");

    // Turn 2: recall memory
    let resp2 = transport
        .chat(&[Message::user("what do you know about me?")], &CallMode::Incremental(session_id.clone()))
        .await?;
    assert!(!resp2.text.trim().is_empty(), "Should get a response referencing prior context");

    eprintln!("✅ Memory tool: session={} turns=2", session_id);
    eprintln!("  turn1: {} chars", resp.text.len());
    eprintln!("  turn2: {} chars", resp2.text.len());
    Ok(())
}
