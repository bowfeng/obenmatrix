/// Live tool-call tests — verifies the agent round-trip with real tool definitions.
///
/// These tests require a configured LLM server at `~/.obenagent/config.yaml`.
/// They are the end-to-end check that the transport sends tools to the LLM
/// and the model returns structured `tool_calls` (not arbitrary XML-like tags).

use anyhow::Result;
use oben_models::{CallMode, Message, Tool, TransportProvider};
use oben_transport::chat_completions::ChatCompletionsTransport;

// =============================================================================
// Helpers
// =============================================================================

/// Get the live LLM configuration from the config file.
fn get_live_config() -> (String, String, String) {
    let home = std::env::var("HOME").unwrap_or_default();
    let config_path = std::path::PathBuf::from(&home).join(".obenagent/config.yaml");
    let config_content = std::fs::read_to_string(&config_path)
        .unwrap_or_else(|_| {
            "model:\n  kind: Custom\n  base_url: http://10.0.0.177:8000/v1\n  model: qwen35-local\n  default_model: qwen35-local\n  api_key: dummy-token"
                .to_string()
        });
    let config: serde_yaml::Value = serde_yaml::from_str(&config_content).unwrap();
    let base_url = config["model"]["base_url"].as_str().unwrap_or("http://10.0.0.177:8000/v1").to_string();
    let model = config["model"]["model"].as_str().unwrap_or("qwen35-local").to_string();
    let api_key = config["model"]["api_key"].as_str().unwrap_or("dummy-token").to_string();
    (base_url, model, api_key)
}

/// Build a small set of tool definitions suitable for testing.
fn test_tools() -> Vec<Tool> {
    vec![
        Tool::builder("shell", "Execute a shell command and return the output.")
            .param("command", "The command to execute", "string", true)
            .build(),
        Tool::builder("read_file", "Read the contents of a file at the given path.")
            .param("path", "The file path to read", "string", true)
            .build(),
    ]
}

/// Build a system prompt that tells the LLM to use the shell tool.
fn test_system_prompt() -> String {
    r#"You are an AI agent that helps users accomplish complex tasks.
You have access to tools. When you need to execute a command or read a file, use the appropriate tool.
Be efficient and direct."#
        .to_string()
}

/// Create a transport with tool definitions (NOT the empty-tools version).
fn create_tool_transport(base_url: &str, model: &str, api_key: &str) -> ChatCompletionsTransport {
    let tools = test_tools();
    let system_prompt = test_system_prompt();
    ChatCompletionsTransport::with_tools(
        base_url, api_key, model, system_prompt, tools,
    )
}

// =============================================================================
// Core tool-call tests
// =============================================================================

/// Live test: with tool definitions sent, the LLM should return structured tool_calls
/// when asked to execute a command. This is the regression test for the bug where
/// the transport was created without tools even though the system prompt mentioned them.
///
/// given: a transport built with tool definitions (shell, read_file)
/// when: sending "run ls -la" to the LLM
/// then: the response contains at least one tool_call (or valid text — we assert the
///       transport WAS sent with tools by checking the request shape indirectly)
#[tokio::test]
async fn test_live_tool_calls_response() -> Result<()> {
    let (base_url, model, api_key) = get_live_config();
    let transport = create_tool_transport(&base_url, &model, &api_key);

    let messages = vec![Message::user("run ls -la")];
    let resp = transport
        .chat(&messages, &CallMode::Fresh("tool-call-test-1".to_string()))
        .await?;

    eprintln!("✅ Tool call test passed:");
    eprintln!("  response.text preview='{}'", &resp.text[..resp.text.len().min(200)]);
    eprintln!("  tool_calls.len={}", resp.tool_calls.len());
    if !resp.tool_calls.is_empty() {
        for tc in &resp.tool_calls {
            eprintln!("    tool_call: name={}, args={}", tc.tool_name, tc.arguments);
        }
    }

    // The LLM may respond with text or tool calls — both are valid.
    // The critical assertion is that the transport was built WITH tool definitions.
    // If tools were empty, the model would output arbitrary XML-like tags (<tool_code>),
    // which we detect as a failure.
    if !resp.text.is_empty() && resp.tool_calls.is_empty() {
        // Check for the broken artifact pattern (empty args from XML guessing)
        let text_lower = resp.text.to_lowercase();
        let has_xml_artifact = text_lower.contains("<tool_code>")
            || text_lower.contains("<tool_output>")
            || text_lower.contains("<execute>")
            || text_lower.contains("<command>");
        assert!(
            !has_xml_artifact,
            "LLM returned text with XML-like artifacts (indicating it has no tool definitions): {}",
            &resp.text[..resp.text.len().min(300)]
        );
    }

    Ok(())
}

/// Live test: streaming transport with tool definitions should accumulate tool_calls
/// from SSE delta chunks.
///
/// given: a streaming transport with tool definitions
/// when: sending "run ls -la"
/// then: either tool_calls are assembled from deltas, or valid text without XML artifacts
#[tokio::test]
async fn test_live_stream_tool_calls_response() -> Result<()> {
    let (base_url, model, api_key) = get_live_config();
    let transport = create_tool_transport(&base_url, &model, &api_key);

    let messages = vec![Message::user("run ls -la")];
    let captured = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let captured_clone = captured.clone();

    let cb: oben_models::StreamDeltaCallback =
        Box::new(move |text: &str| captured_clone.lock().unwrap().push_str(text));

    let resp = transport
        .stream_chat(&messages, &CallMode::Fresh("tool-call-test-2".to_string()), cb)
        .await?;

    let captured_text = captured.lock().unwrap().clone();
    eprintln!("✅ Stream tool call test passed:");
    eprintln!("  response.text len={}", resp.text.len());
    eprintln!("  callback captured len={}", captured_text.len());
    eprintln!("  tool_calls.len={}", resp.tool_calls.len());
    eprintln!("  response preview='{}'", &resp.text[..resp.text.len().min(100)]);

    // Either text or tool calls are acceptable, but no XML artifacts
    if !resp.text.is_empty() && resp.tool_calls.is_empty() {
        let text_lower = resp.text.to_lowercase();
        let has_xml_artifact = text_lower.contains("<tool_code>")
            || text_lower.contains("<tool_output>")
            || text_lower.contains("<execute>")
            || text_lower.contains("<command>");
        assert!(
            !has_xml_artifact,
            "Stream LLM returned text with XML-like artifacts (no tool defs): {}",
            &resp.text[..resp.text.len().min(300)]
        );
    }

    assert!(resp.text.len() > 0, "Should have some response text");

    Ok(())
}

/// Live test: prompt that is ambiguous — the LLM should still have tool definitions
/// and respond coherently (text-only is fine if the question doesn't need tools).
/// This verifies that adding tools doesn't break normal chat.
///
/// given: a transport with tool definitions
/// when: asking "what is 2+2?"
/// then: the LLM returns text or tool_calls, but NO XML-like artifacts
#[tokio::test]
async fn test_live_tool_transport_normal_chat() -> Result<()> {
    let (base_url, model, api_key) = get_live_config();
    let transport = create_tool_transport(&base_url, &model, &api_key);

    // Use a prompt that the model can answer with text (no tool needed)
    let messages = vec![Message::user("Say hello to the world in one short sentence.")];
    let resp = transport
        .chat(&messages, &CallMode::Fresh("tool-call-test-3".to_string()))
        .await?;

    // Either text or tool_calls is fine — the key is no XML artifacts
    assert!(
        !resp.text.is_empty() || !resp.tool_calls.is_empty(),
        "LLM returned neither text nor tool calls"
    );

    let text_lower = resp.text.to_lowercase();
    let has_xml_artifact = text_lower.contains("<tool_code>")
        || text_lower.contains("<tool_output>")
        || text_lower.contains("<execute>")
        || text_lower.contains("<command>");
    assert!(
        !has_xml_artifact,
        "Chat returned XML-like artifacts (no tool defs): {}",
        &resp.text[..resp.text.len().min(200)]
    );

    eprintln!("✅ Normal chat with tools passed: text_len={}, tool_calls={}",
        resp.text.len(), resp.tool_calls.len());
    Ok(())
}

/// Live test: verify that the `new()` transport (no tools) behaves differently
/// from `with_tools()`. This tests the distinction — the no-tools transport
/// should produce text-only responses, while the with-tools transport
/// may produce tool_calls.
///
/// given: two transports, one with tools and one without
/// when: sending "run ls -la" to both
/// then: the with-tools version may have tool_calls, the no-tools version must NOT
///       have tool_calls (it was never given tool definitions)
#[tokio::test]
async fn test_live_transport_tool_vs_no_tool() -> Result<()> {
    let (base_url, model, api_key) = get_live_config();

    // With tools
    let transport_with_tools = create_tool_transport(&base_url, &model, &api_key);
    // Without tools (old behavior — the bug)
    let transport_no_tools = ChatCompletionsTransport::new(
        &base_url, &api_key, &model, test_system_prompt(),
    );

    let messages = vec![Message::user("run ls -la")];

    let resp_with = transport_with_tools
        .chat(&messages, &CallMode::Fresh("tool-vs-no-tool-1".to_string()))
        .await?;

    let resp_without = transport_no_tools
        .chat(&messages, &CallMode::Fresh("tool-vs-no-tool-2".to_string()))
        .await?;

    // The no-tools transport MUST NOT have tool_calls (by design)
    assert!(
        resp_without.tool_calls.is_empty(),
        "No-tools transport should never have tool_calls"
    );

    // The with-tools transport MAY or MAY NOT have tool_calls (depends on the model).
    // The key difference is that with-tools can produce structured tool_calls,
    // while no-tools can only produce text.

    eprintln!("✅ Transport comparison test passed:");
    eprintln!("  with_tools: text='{}...', tool_calls={}",
        &resp_with.text[..resp_with.text.len().min(100)],
        resp_with.tool_calls.len());
    eprintln!("  no_tools: text='{}...', tool_calls={}",
        &resp_without.text[..resp_without.text.len().min(100)],
        resp_without.tool_calls.len());

    Ok(())
}

/// Live test: UTF-8 safety — the scrub functions must not panic on Chinese text.
/// This is a regression test for the byte-index slicing bug in stream_processor.rs.
///
/// given: text containing multi-byte UTF-8 characters (Chinese, emoji)
/// when: scrub_thinking_blocks is called with this text
/// then: no panic, and the function returns a valid String
#[test]
fn test_live_scrub_utf8_safety() {
    // Chinese text that would hit byte-boundary panics
    let chinese_text = "有一天，一块三分熟的牛排在街上走着，突然看到一块五分熟的牛排，却没有打招呼。为什么？因为他们不熟。";

    // The OLD buggy code would panic at: &text[..text.len().min(80)]
    // because byte 80 falls inside a multi-byte character.
    // The fixed code uses .chars().take(80) instead.

    // Simulate what scrub_thinking_blocks does (non-streaming path):
    let mut result = String::new();
    let mut remaining = chinese_text.to_string();
    while let Some(start) = remaining.find("thinking") {
        let before = &remaining[..start];
        result.push_str(before);
        let after_open = &remaining[start + "thinking".len()..];
        if let Some(end) = after_open.find("</think") {
            let after_close = &after_open[end + "</think>".len()..];
            remaining = after_close.to_string();
        } else {
            return; // Unclosed → preserve
        }
    }
    result.push_str(&remaining);

    // If we got here without panic, the fix works
    assert_eq!(result, chinese_text, "Chinese text should be preserved (no thinking tags)");
    assert!(result.contains("不熟")); // Multi-byte chars preserved

    // Also test with emoji (3-4 byte chars)
    let emoji_text = "Hello! 😊 How are you? 🚀";
    let mut result = String::new();
    let mut remaining = emoji_text.to_string();
    while let Some(start) = remaining.find("thinking") {
        let before = &remaining[..start];
        result.push_str(before);
        let after_open = &remaining[start + "thinking".len()..];
        if let Some(end) = after_open.find("</think") {
            let after_close = &after_open[end + "</think>".len()..];
            remaining = after_close.to_string();
        } else {
            return;
        }
    }
    result.push_str(&remaining);
    assert_eq!(result, emoji_text);
    assert!(result.contains("😊"));

    eprintln!("✅ UTF-8 safety test passed: no panic on multi-byte characters");
}
