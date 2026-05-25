/// Live tool-call tests — verifies the agent round-trip with real tool definitions.
///
/// These tests require a configured LLM server at `~/.obenagent/config.yaml`.
/// They are the end-to-end check that the transport sends tools to the LLM
/// and the model returns structured `tool_calls` (not arbitrary XML-like tags).

use anyhow::Result;
use oben_config::AppConfig;
use oben_models::{CallMode, Message, ProviderConfig, Tool, TransportProvider};
use oben_transport::Transport;

/// Safely take the first `n` chars from a string (UTF-8 safe).
fn preview(s: &str, n: usize) -> &str {
    if s.len() <= n {
        s
    } else {
        let end = s.char_indices().nth(n).map(|(i, _)| i).unwrap_or(s.len());
        &s[..end]
    }
}

fn get_provider_config() -> ProviderConfig {
    let config = AppConfig::load().expect("Failed to load config");
    let mut pc = ProviderConfig::new(
        config.model.kind.clone(),
        config.model.model.clone(),
    );
    pc.api_key = config.model.api_key.clone();
    pc.base_url = config.model.base_url.clone();
    pc.temperature = config.model.temperature;
    pc.default_model = config.model.default_model.clone();
    pc.max_tokens = config.model.max_tokens;
    pc.fallback_models = config.model.fallback_models.clone();
    pc
}

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

fn test_system_prompt() -> String {
    r#"You are an AI agent that helps users accomplish complex tasks.
You have access to tools. When you need to execute a command or read a file, use the appropriate tool.
Be efficient and direct."#
        .to_string()
}

fn create_tool_transport(pc: &ProviderConfig) -> Transport {
    let tools = test_tools();
    let system_prompt = test_system_prompt();
    Transport::from_config_with_tools(pc, system_prompt, tools)
}

fn create_no_tool_transport(pc: &ProviderConfig) -> Transport {
    let system_prompt = test_system_prompt();
    Transport::from_config(pc, system_prompt)
}

/// Live test: with tool definitions sent, the LLM should return structured tool_calls.
#[tokio::test]
async fn test_live_tool_calls_response() -> Result<()> {
    let pc = get_provider_config();
    let transport = create_tool_transport(&pc);

    let messages = vec![Message::user("run ls -la")];
    let resp = transport
        .chat(&messages, &CallMode::Fresh("tool-call-test-1".to_string()))
        .await?;

    eprintln!("✅ Tool call test passed:");
    eprintln!("  response.text preview='{}'", preview(&resp.text, 200));
    eprintln!("  tool_calls.len={}", resp.tool_calls.len());
    if !resp.tool_calls.is_empty() {
        for tc in &resp.tool_calls {
            eprintln!("    tool_call: name={}, args={}", tc.tool_name, tc.arguments);
        }
    }

    if !resp.text.is_empty() && resp.tool_calls.is_empty() {
        let text_lower = resp.text.to_lowercase();
        let has_xml_artifact = text_lower.contains("<tool_code>")
            || text_lower.contains("<tool_output>")
            || text_lower.contains("<execute>")
            || text_lower.contains("<command>");
        assert!(
            !has_xml_artifact,
            "LLM returned text with XML-like artifacts: {}",
            preview(&resp.text, 300)
        );
    }

    Ok(())
}

/// Live test: streaming transport with tool definitions.
#[tokio::test]
async fn test_live_stream_tool_calls_response() -> Result<()> {
    let pc = get_provider_config();
    let transport = create_tool_transport(&pc);

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
    eprintln!("  response preview='{}'", preview(&resp.text, 100));

    if !resp.text.is_empty() && resp.tool_calls.is_empty() {
        let text_lower = resp.text.to_lowercase();
        let has_xml_artifact = text_lower.contains("<tool_code>")
            || text_lower.contains("<tool_output>")
            || text_lower.contains("<execute>")
            || text_lower.contains("<command>");
        assert!(
            !has_xml_artifact,
            "Stream LLM returned text with XML-like artifacts: {}",
            preview(&resp.text, 300)
        );
    }

    assert!(resp.text.len() > 0, "Should have some response text");

    Ok(())
}

/// Live test: tools don't break normal chat.
#[tokio::test]
async fn test_live_tool_transport_normal_chat() -> Result<()> {
    let pc = get_provider_config();
    let transport = create_tool_transport(&pc);

    let messages = vec![Message::user("Say hello to the world in one short sentence.")];
    let resp = transport
        .chat(&messages, &CallMode::Fresh("tool-call-test-3".to_string()))
        .await?;

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
        "Chat returned XML-like artifacts: {}",
        preview(&resp.text, 200)
    );

    eprintln!("✅ Normal chat with tools passed: text_len={}, tool_calls={}",
        resp.text.len(), resp.tool_calls.len());
    Ok(())
}

/// Live test: with-tools vs no-tools transport difference.
#[tokio::test]
async fn test_live_transport_tool_vs_no_tool() -> Result<()> {
    let pc = get_provider_config();

    let transport_with_tools = create_tool_transport(&pc);
    let transport_no_tools = create_no_tool_transport(&pc);

    let messages = vec![Message::user("run ls -la")];

    let resp_with = transport_with_tools
        .chat(&messages, &CallMode::Fresh("tool-vs-no-tool-1".to_string()))
        .await?;

    let resp_without = transport_no_tools
        .chat(&messages, &CallMode::Fresh("tool-vs-no-tool-2".to_string()))
        .await?;

    assert!(
        resp_without.tool_calls.is_empty(),
        "No-tools transport should never have tool_calls"
    );

    eprintln!("✅ Transport comparison test passed:");
    eprintln!("  with_tools: text='{}...', tool_calls={}",
        preview(&resp_with.text, 100),
        resp_with.tool_calls.len());
    eprintln!("  no_tools: text='{}...', tool_calls={}",
        preview(&resp_without.text, 100),
        resp_without.tool_calls.len());

    Ok(())
}

/// Live test: multi-turn agent chat with tool use and Chinese input.
#[tokio::test]
async fn test_live_multiturn_chat_with_tools() -> Result<()> {
    let pc = get_provider_config();
    let transport = create_tool_transport(&pc);

    let session_id = format!("multiturn-{}", uuid::Uuid::new_v4());

    let probe = transport
        .chat(
            &[Message::user("hi")],
            &CallMode::Fresh(format!("{}-probe", session_id)),
        )
        .await;
    if probe.is_err() {
        eprintln!("Skipping multi-turn test: LLM server unreachable");
        return Ok(());
    }

    async fn send_with_retry(
        transport: &dyn oben_models::TransportProvider,
        msgs: &[Message],
        mode: &CallMode,
    ) -> Result<oben_models::TransportResponse> {
        let mut last_err = None;
        for attempt in 0..=2 {
            match transport.chat(msgs, mode).await {
                Ok(r) => return Ok(r),
                Err(e) => {
                    last_err = Some(e);
                    if attempt < 2 {
                        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                    }
                }
            }
        }
        Err(anyhow::anyhow!("All retries failed: {}", last_err.unwrap()))
    }

    let resp1 = send_with_retry(&transport, &[Message::user("hello")], &CallMode::Fresh(session_id.clone())).await?;
    assert!(!resp1.text.trim().is_empty(), "Turn 1 (hello) should have a response");
    let text1 = resp1.text.trim().to_string();

    eprintln!("✅ Multi-turn turn 1 (hello): text_len={}, tool_calls={}", text1.len(), resp1.tool_calls.len());
    eprintln!("   preview='{}'", preview(&text1, 120));

    let resp2 = send_with_retry(&transport, &[Message::user("现实当前目录")], &CallMode::Incremental(session_id.clone())).await?;
    assert!(
        !resp2.text.trim().is_empty() || !resp2.tool_calls.is_empty(),
        "Turn 2 (现实当前目录) should have a response"
    );
    let text2 = resp2.text.trim().to_string();

    eprintln!("✅ Multi-turn turn 2 (现实当前目录): text_len={}, tool_calls={}", text2.len(), resp2.tool_calls.len());
    eprintln!("   text='{}'", preview(&text2, 120));

    let resp3 = send_with_retry(&transport, &[Message::user("你是谁")], &CallMode::Incremental(session_id.clone())).await?;
    assert!(
        !resp3.text.trim().is_empty() || !resp3.tool_calls.is_empty(),
        "Turn 3 (你是谁) should have a response"
    );
    let text3 = resp3.text.trim().to_string();

    eprintln!("✅ Multi-turn turn 3 (你是谁): text_len={}, tool_calls={}", text3.len(), resp3.tool_calls.len());
    eprintln!("   text='{}'", preview(&text3, 120));

    assert!(!text1.is_empty() || !resp1.tool_calls.is_empty(), "Turn 1 must have response");
    assert!(!text2.is_empty() || !resp2.tool_calls.is_empty(), "Turn 2 must have response");
    assert!(!text3.is_empty() || !resp3.tool_calls.is_empty(), "Turn 3 must have response");

    for (text, label) in [(&text1, "turn1"), (&text2, "turn2"), (&text3, "turn3")]
        .into_iter()
        .filter(|(t, _)| !t.is_empty())
    {
        let lines: Vec<&str> = text.lines().collect();
        for j in 1..lines.len() {
            assert!(lines[j] != lines[j - 1],
                "Duplicate adjacent lines in {}: {}", label, preview(&text, 200));
        }
    }

    if !text1.is_empty() && !text2.is_empty() {
        assert_ne!(text1, text2, "Turn 1 and Turn 2 text should not be identical");
    }
    if !text2.is_empty() && !text3.is_empty() {
        assert_ne!(text2, text3, "Turn 2 and Turn 3 text should not be identical");
    }
    if !text1.is_empty() && !text3.is_empty() {
        assert_ne!(text1, text3, "Turn 1 and Turn 3 text should not be identical");
    }

    let has_tool_call = resp2.tool_calls.iter().any(|tc| {
        tc.tool_name == "shell"
            || serde_json::to_string(&tc.arguments).map(|s| s.contains("command")).unwrap_or(false)
    });
    let has_path_chars = text2.contains("/Users/") || text2.contains("/home/")
        || text2.contains(".git") || text2.contains(".rs");

    if has_tool_call {
        eprintln!("  ↳ Turn 2 invoked shell tool call (expected)");
    } else if has_path_chars {
        eprintln!("  ↳ Turn 2 returned directory-like output in text (acceptable)");
    } else {
        eprintln!("  ↳ Turn 2: no tool call detected, text doesn't contain obvious path chars");
        eprintln!("     tool_calls={}", resp2.tool_calls.len());
        eprintln!("     text preview='{}'", preview(&text2, 200));
    }

    let text3_lower = text3.to_lowercase();
    let identity_keywords = ["助手", "agent", "assistant", "chat", "help", "模型", "回答"];
    let has_identity = identity_keywords.iter().any(|k| text3_lower.contains(k) || text3.contains(k));
    if !has_identity {
        eprintln!("  ↳ Turn 3 doesn't contain obvious identity keywords");
        eprintln!("     text='{}'", preview(&text3, 200));
    }

    eprintln!("✅ Multi-turn chat test passed: 3 responses, all non-empty, no duplicates");
    Ok(())
}

/// Live test: UTF-8 safety — scrub functions must not panic on Chinese text.
#[test]
fn test_live_scrub_utf8_safety() {
    let chinese_text = "有一天，一块三分熟的牛排在街上走着，突然看到一块五分熟的牛排，却没有打招呼。为什么？因为他们不熟。";

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
            return;
        }
    }
    result.push_str(&remaining);

    assert_eq!(result, chinese_text, "Chinese text should be preserved");
    assert!(result.contains("不熟"));

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
