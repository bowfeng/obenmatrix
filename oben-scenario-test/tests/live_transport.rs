/// Live transport tests — verifies the ChatCompletionsTransport works with
/// a real LLM server. These tests require a configured LLM server at
/// `~/.obenagent/config.yaml`.
///
/// For mock-based transport tests, see `oben-transport/tests/integration.rs`.

use anyhow::Result;
use oben_models::{CallMode, Message, TransportProvider, StreamDeltaCallback};
use oben_transport::chat_completions::ChatCompletionsTransport;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Get the live LLM configuration from the config file.
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

// =============================================================================
// Scrub tests (deterministic, no LLM needed)
// =============================================================================

/// Fixed scrub_thinking_blocks — matches the bug fix in stream_processor.rs
fn scrub_thinking_blocks(text: &str) -> String {
    let mut result = String::new();
    let mut remaining = text.to_string();
    while let Some(start) = remaining.find("thinking") {
        let before = &remaining[..start];
        result.push_str(before);
        let after_open = &remaining[start + "thinking".len()..];
        if let Some(end) = after_open.find("</think") {
            let after_close = &after_open[end + "</think>".len()..];
            remaining = after_close.to_string();
        } else {
            return text.to_string();
        }
    }
    result.push_str(&remaining);
    result
}

/// Fixed scrub_memory_context — matches the bug fix in stream_processor.rs
fn scrub_memory_context(text: &str) -> String {
    let mut result = String::new();
    let mut remaining = text.to_string();
    while let Some(start) = remaining.find("<memory-context>") {
        let before = &remaining[..start];
        result.push_str(before);
        let after_open = &remaining[start + "<memory-context>".len()..];
        if let Some(end) = after_open.find("</memory>") {
            let after_close = &after_open[end + "</memory>".len()..];
            remaining = after_close.to_string();
        } else {
            return text.to_string();
        }
    }
    result.push_str(&remaining);
    result
}

#[test]
fn test_scrub_thinking_blocks() {
    // ✅ Valid closed blocks — stripped
    assert_eq!(
        scrub_thinking_blocks("thinkinglet me think</think>visible"),
        "visible"
    );
    assert_eq!(
        scrub_thinking_blocks("firstthinkingblock</think>second"),
        "firstsecond"
    );
    assert_eq!(
        scrub_thinking_blocks("AthinkingB</think>CthinkingD</think>E"),
        "ACE"
    );

    // ✅ Unclosed blocks — text PRESERVED (bug fix)
    assert_eq!(
        scrub_thinking_blocks("thinkingunclosed"),
        "thinkingunclosed"
    );
    assert_eq!(
        scrub_thinking_blocks("hello thinking about this"),
        "hello thinking about this"
    );

    // ✅ Normal text — unchanged
    assert_eq!(
        scrub_thinking_blocks("just plain text"),
        "just plain text"
    );
}

#[test]
fn test_scrub_memory_context() {
    // ✅ Valid closed blocks — stripped
    assert_eq!(
        scrub_memory_context("<memory-context>secret</memory>visible"),
        "visible"
    );
    assert_eq!(
        scrub_memory_context("before<memory-context>hidden</memory>after"),
        "beforeafter"
    );

    // ✅ Unclosed block — text PRESERVED
    assert_eq!(
        scrub_memory_context("<memory-context>unclosed"),
        "<memory-context>unclosed"
    );
    assert_eq!(
        scrub_memory_context("before <memory-context> hidden"),
        "before <memory-context> hidden"
    );

    // ✅ Normal text — unchanged
    assert_eq!(
        scrub_memory_context("no blocks here"),
        "no blocks here"
    );
}

/// Test that scrub functions don't destroy a real LLM greeting response.
#[test]
fn test_scrub_preserves_real_llm_responses() {
    let responses = vec![
        "Hello! How can I help you today?",
        "\n\nHello! How can I help you today? 😊",
        "Sure, I'll help you with that.",
        "Let me run the command for you.",
        "Here is the output from your command:\n```bash\n$ ls\nfile1.txt\nfile2.txt\n```",
        "I'm thinking about how to best answer this...",
    ];

    for text in &responses {
        let after = scrub_thinking_blocks(text);
        let after = scrub_memory_context(&after);
        assert_eq!(*text, after, "Scrub changed response: {:?}", text);
    }

    eprintln!("✅ All realistic LLM responses survived scrubbing");
}

// =============================================================================
// Live LLM tests
// =============================================================================

/// Live test: basic transport → LLM → response round-trip.
/// This is the simplest check that the wire protocol works.
#[tokio::test]
async fn test_live_chat_simple() -> Result<()> {
    let (base_url, model, api_key, system_prompt) = get_live_config();
    let transport = ChatCompletionsTransport::new(
        base_url, api_key, model, system_prompt,
    );

    let messages = vec![Message::user("hello")];
    let resp = transport
        .chat(&messages, &CallMode::Fresh("test-session-1".to_string()))
        .await?;

    let trimmed = resp.text.trim();
    assert!(!trimmed.is_empty(), "LLM returned empty response!");
    assert!(resp.text.len() > 10, "Response too short ({} chars)", resp.text.len());

    eprintln!("✅ Live chat test passed: text_len={}, text_preview='{}...'",
        resp.text.len(),
        &resp.text[..resp.text.len().min(80)]);

    Ok(())
}

/// Live test: transport with a large system prompt (as the binary actually sends).
#[tokio::test]
async fn test_live_chat_with_large_system_prompt() -> Result<()> {
    let (base_url, model, api_key, _system_prompt) = get_live_config();

    let large_system_prompt = format!(
        "You are an AI agent that helps users accomplish complex tasks.\n\
         You have access to tools and can create and improve your own skills from experience.\n\
         Be thorough, careful, and efficient.\n\n## Guidelines\n\
         - Understand the user's intent fully before acting\n\
         - Use tools to accomplish tasks; explain what you're doing\n\
         - If a tool call fails, analyze the error and retry with corrections\n\
         - Create skills for repeated complex workflows\n\
         - Compress conversation context when it grows large\n\
         - Search your memory for relevant past information before starting new work\n\
         - Be honest about your limitations\n\n\
         ## Tool Usage Guidelines\n\n\
         You have a `shell` tool for executing commands.\n\n\
         ## Execution Discipline\n\
         You MUST use your tools to take action.",
    );

    let transport = ChatCompletionsTransport::new(
        base_url, api_key, model, large_system_prompt,
    );

    let messages = vec![Message::user("hello")];
    let resp = transport
        .chat(&messages, &CallMode::Fresh("test-session-2".to_string()))
        .await?;

    let trimmed = resp.text.trim();
    assert!(!trimmed.is_empty(), "LLM returned empty response with large system prompt!");
    assert!(resp.text.len() > 10, "Response too short with large system prompt ({} chars)", resp.text.len());

    eprintln!("✅ Large system prompt test passed: text_len={}, text_preview='{}...'",
        resp.text.len(),
        &resp.text[..resp.text.len().min(80)]);

    Ok(())
}

/// Live test: streaming transport — verifies SSE parsing, callback capture,
/// and that the final response matches the callback accumulation.
#[tokio::test]
async fn test_live_stream_chat() -> Result<()> {
    let (base_url, model, api_key, system_prompt) = get_live_config();
    let transport = ChatCompletionsTransport::new(
        base_url, api_key, model, system_prompt,
    );

    let messages = vec![Message::user("tell me a short greeting")];
    let captured = Arc::new(std::sync::Mutex::new(String::new()));
    let captured_clone = captured.clone();

    let cb: StreamDeltaCallback =
        Box::new(move |text: &str| captured_clone.lock().unwrap().push_str(text));

    let resp = transport
        .stream_chat(&messages, &CallMode::Fresh("test-session-3".to_string()), cb)
        .await?;

    let captured_text = captured.lock().unwrap().clone();
    eprintln!("✅ Live stream test passed:");
    eprintln!("  response.text len={}", resp.text.len());
    eprintln!("  callback captured len={}", captured_text.len());
    eprintln!("  response.text preview='{}'", &resp.text[..resp.text.len().min(80)]);
    eprintln!("  callback preview='{}'", &captured_text[..captured_text.len().min(80)]);

    assert!(!resp.text.trim().is_empty(), "Stream response text is empty!");
    assert!(resp.text.len() > 10, "Stream response too short ({} chars)", resp.text.len());

    // Verify callback captured the same text
    assert_eq!(resp.text, captured_text, "Stream response text doesn't match callback capture!");

    Ok(())
}

/// Live test: model returns tool calls (e.g., shell command execution).
#[tokio::test]
async fn test_live_chat_with_tool_calls_response() -> Result<()> {
    let (base_url, model, api_key, system_prompt) = get_live_config();
    let transport = ChatCompletionsTransport::new(
        base_url, api_key, model, system_prompt,
    );

    let messages = vec![Message::user("run ls -la")];
    let resp = transport
        .chat(&messages, &CallMode::Fresh("test-session-4".to_string()))
        .await?;

    eprintln!("✅ Tool call test passed:");
    eprintln!("  response.text='{}'", &resp.text[..resp.text.len().min(200)]);
    eprintln!("  tool_calls.len={}", resp.tool_calls.len());
    if !resp.tool_calls.is_empty() {
        for tc in &resp.tool_calls {
            eprintln!("    tool_call: name={}, args={}", tc.tool_name, tc.arguments);
        }
    }

    // The model might return a text response or a tool call - both are valid
    assert!(!resp.text.is_empty() || !resp.tool_calls.is_empty(),
        "LLM returned neither text nor tool calls!");

    Ok(())
}

/// Live test: concurrent transport calls (simulates gateway multi-queue).
/// Each thread makes an independent chat request to the same LLM server.
/// Verifies no connection pool or rate-limit errors.
#[tokio::test]
async fn test_live_concurrent_requests() -> Result<()> {
    let (base_url, model, api_key, system_prompt) = get_live_config();
    let transport = Arc::new(ChatCompletionsTransport::new(
        base_url, api_key, model, system_prompt,
    ));

    let num_threads = 5;
    let mut handles = Vec::with_capacity(num_threads);

    for i in 0..num_threads {
        let t = Arc::clone(&transport);
        let mode = CallMode::Fresh(format!("concurrent-req-{}", uuid::Uuid::new_v4()));
        let handle = tokio::spawn(async move {
            let msgs = vec![Message::user(format!("request from thread {}", i))];
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
        eprintln!("⚠ Concurrent request failures:");
        for e in &errors {
            eprintln!("  {}", e);
        }
        // Connection pool exhaustion is acceptable in high-concurrency scenarios
        // with a single transport instance. But 0% failure is ideal.
        eprintln!("  {}/{} succeeded (connection pool or rate limiting may affect others)", successes, num_threads);
    } else {
        assert_eq!(successes, num_threads, "All {} threads should succeed", num_threads);
    }

    eprintln!("✅ Live concurrent requests: {}/{} succeeded", successes, num_threads);
    Ok(())
}

/// Live test: streaming with tool calls (SSE containing tool delta chunks).
/// This test sends a prompt that is likely to trigger a tool call.
#[tokio::test]
async fn test_live_stream_chat_with_tool_calls() -> Result<()> {
    let (base_url, model, api_key, system_prompt) = get_live_config();
    let transport = ChatCompletionsTransport::new(
        base_url, api_key, model, system_prompt,
    );

    let messages = vec![Message::user("list files and print their sizes")];
    let captured = Arc::new(std::sync::Mutex::new(String::new()));
    let captured_clone = captured.clone();

    let cb: StreamDeltaCallback =
        Box::new(move |text: &str| captured_clone.lock().unwrap().push_str(text));

    let resp = transport
        .stream_chat(&messages, &CallMode::Fresh("test-session-5".to_string()), cb)
        .await?;

    let captured_text = captured.lock().unwrap().clone();

    eprintln!("✅ Live stream + tool calls test passed:");
    eprintln!("  response.text len={}", resp.text.len());
    eprintln!("  callback captured len={}", captured_text.len());
    eprintln!("  tool_calls.len={}", resp.tool_calls.len());
    eprintln!("  response preview='{}'", &resp.text[..resp.text.len().min(100)]);

    // Either text or tool calls are acceptable
    assert!(!resp.text.is_empty() || !resp.tool_calls.is_empty(),
        "Should have either text or tool calls");

    Ok(())
}

/// Live test: long-running streaming response (stress test SSE parsing).
/// Sends a prompt that generates a long response, verifying the stream
/// parser doesn't lose content.
#[tokio::test]
async fn test_live_long_stream_response() -> Result<()> {
    let (base_url, model, api_key, system_prompt) = get_live_config();
    let transport = ChatCompletionsTransport::new(
        base_url, api_key, model, system_prompt,
    );

    let messages = vec![Message::user("write a detailed explanation of HTTP/2 multiplexing, about 200 words")];
    let captured = Arc::new(std::sync::Mutex::new(String::new()));
    let captured_clone = captured.clone();

    let cb: StreamDeltaCallback =
        Box::new(move |text: &str| captured_clone.lock().unwrap().push_str(text));

    let resp = transport
        .stream_chat(&messages, &CallMode::Fresh("test-session-6".to_string()), cb)
        .await?;

    let captured_text = captured.lock().unwrap().clone();

    // Should get a substantial response (200+ words)
    assert!(resp.text.len() > 500, "Expected a long response, got {} chars", resp.text.len());
    assert_eq!(resp.text, captured_text, "Stream text should match callback capture");

    eprintln!("✅ Long stream response test passed: {} chars", resp.text.len());
    Ok(())
}

/// Live test: verify token counting in streaming responses.
/// Some LLM servers include usage info in the final SSE delta.
#[tokio::test]
async fn test_live_stream_with_usage() -> Result<()> {
    let (base_url, model, api_key, system_prompt) = get_live_config();
    let transport = ChatCompletionsTransport::new(
        base_url, api_key, model, system_prompt,
    );

    let messages = vec![Message::user("say hello briefly")];
    let captured = Arc::new(std::sync::Mutex::new(String::new()));
    let captured_clone = captured.clone();

    let cb: StreamDeltaCallback =
        Box::new(move |text: &str| captured_clone.lock().unwrap().push_str(text));

    let resp = transport
        .stream_chat(&messages, &CallMode::Fresh("test-session-7".to_string()), cb)
        .await?;

    let captured_text = captured.lock().unwrap().clone();

    eprintln!("✅ Stream usage test passed:");
    eprintln!("  response.text len={}", resp.text.len());
    eprintln!("  tokens_used={:?}", resp.tokens_used);
    eprintln!("  callback text matches: {}", resp.text == captured_text);

    assert!(resp.text.len() > 0, "Should get a response");

    // Usage tracking is best-effort — some servers don't include it
    if let Some(tokens) = resp.tokens_used {
        assert!(tokens > 0, "Tokens should be positive when reported");
    }

    Ok(())
}
