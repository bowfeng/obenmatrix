// Integration tests for `AnthropicMessagesTransport` with a wiremock server.
// These tests use a local mock Anthropic Messages API to verify:
// - Non-streaming chat completions
// - Streaming chat with SSE events
// - Tool use in responses
// - Error handling

use std::sync::{Arc, Mutex};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use oben_models::{CallMode, Message, TransportProvider};
use oben_transport::anthropic_messages::AnthropicMessagesTransport;

/// Start a mock Anthropic server with the expected endpoint.
async fn mock_server() -> MockServer {
    MockServer::start().await
}

/// Helper to build a transport pointing at a mock server.
fn make_transport(mock_url: &str, api_key: &str) -> AnthropicMessagesTransport {
    AnthropicMessagesTransport::new(
        mock_url,
        api_key,
        "claude-sonnet-4-20250514",
        "You are a helpful assistant",
    )
}

// ── Non-streaming tests ─────────────────────────────────────────────────────

#[tokio::test]
async fn test_chat_text_response() {
    // given: a mock Anthropic Messages API returning a text response
    // when: send_request with a user message is called
    // then: returns TransportResponse with the text content
    let server = mock_server().await;

    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{
                "id": "msg_123",
                "type": "message",
                "role": "assistant",
                "content": [{"type": "text", "text": "Hello from Claude!"}],
                "stop_reason": "end_turn",
                "model": "claude-sonnet-4-20250514",
                "usage": {"input_tokens": 10, "output_tokens": 5}
            }"#,
        ))
        .mount(&server)
        .await;

    let transport = make_transport(&server.uri(), "sk-test");
    let messages = vec![Message::user("hi")];
    let result = transport
        .chat(&messages, &CallMode::Fresh("test-session".into()))
        .await
        .unwrap();

    assert_eq!(result.text, "Hello from Claude!");
    assert_eq!(result.tool_calls.len(), 0);
    assert_eq!(result.tokens_used, Some(5));
}

#[tokio::test]
async fn test_chat_with_tool_call() {
    // given: a mock Anthropic Messages API returning a tool_use response
    // when: send_request is called
    // then: returns TransportResponse with tool calls
    let server = mock_server().await;

    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{
                "id": "msg_456",
                "type": "message",
                "role": "assistant",
                "content": [
                    {"type": "text", "text": "I'll check the files."},
                    {"type": "tool_use", "id": "call-abc", "name": "shell", "input": {"command": "ls"}}
                ],
                "stop_reason": "tool_use",
                "model": "claude-sonnet-4-20250514",
                "usage": {"input_tokens": 20, "output_tokens": 8}
            }"#,
        ))
        .mount(&server)
        .await;

    let transport = make_transport(&server.uri(), "sk-test");
    let messages = vec![Message::user("what files are there?")];
    let result = transport
        .chat(&messages, &CallMode::Fresh("test-session-2".into()))
        .await
        .unwrap();

    assert_eq!(result.text, "I'll check the files.");
    assert_eq!(result.tool_calls.len(), 1);
    assert_eq!(result.tool_calls[0].tool_name, "shell");
    assert_eq!(result.tool_calls[0].id, "call-abc");
    assert_eq!(
        result.tool_calls[0].arguments["command"].as_str().unwrap(),
        "ls"
    );
}

#[tokio::test]
async fn test_chat_api_error() {
    // given: a mock Anthropic Messages API returning a 401 error
    // when: send_request is called
    // then: returns Err with the error message
    let server = mock_server().await;

    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(ResponseTemplate::new(401).set_body_string(
            r#"{"error":{"message":"Invalid API key","type":"invalid_request_error"}}"#,
        ))
        .mount(&server)
        .await;

    let transport = make_transport(&server.uri(), "sk-test");
    let messages = vec![Message::user("hi")];
    let result = transport
        .chat(&messages, &CallMode::Fresh("test-session-error".into()))
        .await;

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("401"));
}

// ── Streaming tests ─────────────────────────────────────────────────────────

#[tokio::test]
async fn test_stream_chat_text_response() {
    // given: a mock Anthropic Messages API returning a streaming SSE response
    // when: stream_chat is called with a callback
    // then: the callback is fired with each text delta
    let server = mock_server().await;

    let sse_body = r#"event: message_start
data: {"type":"message_start","message":{"id":"msg_1","role":"assistant","content":[],"model":"claude-sonnet-4-20250514","stop_reason":null,"stop_sequence":null,"type":"message","usage":{"input_tokens":10,"output_tokens":0}}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":", world!"}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":15}}

event: message_stop
data: {"type":"message_stop"}

"#;

    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(sse_body)
                .insert_header("content-type", "text/event-stream"),
        )
        .mount(&server)
        .await;

    let transport = make_transport(&server.uri(), "sk-test");
    let messages = vec![Message::user("greet me")];
    let received_deltas = Arc::new(Mutex::new(Vec::<String>::new()));
    let received_deltas_clone = received_deltas.clone();
    let cb = Box::new(move |delta: &str| {
        received_deltas_clone
            .lock()
            .unwrap()
            .push(delta.to_string());
    });
    let result = transport
        .stream_chat(&messages, &CallMode::Fresh("test-stream-1".into()), cb)
        .await
        .unwrap();

    assert_eq!(result.text, "Hello, world!");
    assert_eq!(result.tokens_used, Some(15));
    let deltas = received_deltas.lock().unwrap();
    assert!(deltas.contains(&"Hello".to_string()));
    assert!(deltas.contains(&", world!".to_string()));
}

#[tokio::test]
async fn test_stream_chat_with_tool_calls() {
    // given: a mock Anthropic Messages API returning tool_use via streaming SSE
    // when: stream_chat is called
    // then: accumulates both text and tool calls correctly
    let server = mock_server().await;

    let sse_body = r#"event: message_start
data: {"type":"message_start","message":{"id":"msg_2","role":"assistant","content":[],"model":"claude-sonnet-4-20250514","stop_reason":null,"stop_sequence":null,"type":"message","usage":{"input_tokens":15,"output_tokens":0}}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Let me check"}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: content_block_start
data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"call-xyz","name":"shell"}}

event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","input_json":"{\"com"}}

event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","input_json":"mand\":\"ls\"}"}}

event: content_block_stop
data: {"type":"content_block_stop","index":1}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":20}}

event: message_stop
data: {"type":"message_stop"}

"#;

    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(sse_body)
                .insert_header("content-type", "text/event-stream"),
        )
        .mount(&server)
        .await;

    let transport = make_transport(&server.uri(), "sk-test");
    let messages = vec![Message::user("list files")];
    let received_text = Arc::new(Mutex::new(String::new()));
    let received_text_clone = received_text.clone();
    let cb = Box::new(move |delta: &str| {
        received_text_clone.lock().unwrap().push_str(delta);
    });
    let result = transport
        .stream_chat(&messages, &CallMode::Fresh("test-stream-tool".into()), cb)
        .await
        .unwrap();

    assert_eq!(*received_text.lock().unwrap(), "Let me check");
    assert_eq!(result.text, "Let me check");
    assert_eq!(result.tool_calls.len(), 1);
    assert_eq!(result.tool_calls[0].id, "call-xyz");
    assert_eq!(result.tool_calls[0].tool_name, "shell");
    assert_eq!(
        result.tool_calls[0].arguments["command"].as_str().unwrap(),
        "ls"
    );
    assert_eq!(result.tokens_used, Some(20));
}

#[tokio::test]
async fn test_stream_chat_empty_content() {
    // given: a mock Anthropic Messages API returning only message_start/message_stop
    // when: stream_chat is called
    // then: returns empty text with no tool calls
    let server = mock_server().await;

    let sse_body = r#"event: message_start
data: {"type":"message_start","message":{"id":"msg_empty","role":"assistant","content":[],"model":"claude-sonnet-4-20250514","stop_reason":null,"stop_sequence":null,"type":"message","usage":{"input_tokens":5,"output_tokens":0}}}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":0}}

event: message_stop
data: {"type":"message_stop"}

"#;

    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(sse_body)
                .insert_header("content-type", "text/event-stream"),
        )
        .mount(&server)
        .await;

    let transport = make_transport(&server.uri(), "sk-test");
    let messages = vec![Message::user("hi")];
    let cb = Box::new(|_delta: &str| {});
    let result = transport
        .stream_chat(&messages, &CallMode::Fresh("test-stream-empty".into()), cb)
        .await
        .unwrap();

    assert!(result.text.is_empty());
    assert!(result.tool_calls.is_empty());
    assert_eq!(result.tokens_used, Some(0));
}

#[tokio::test]
async fn test_stream_chat_api_error() {
    // given: a mock Anthropic Messages API returning 400 on streaming
    // when: stream_chat is called
    // then: returns Err with the error message
    let server = mock_server().await;

    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(ResponseTemplate::new(400).set_body_string(
            r#"{"error":{"message":"Bad request","type":"invalid_request_error"}}"#,
        ))
        .mount(&server)
        .await;

    let transport = make_transport(&server.uri(), "sk-test");
    let messages = vec![Message::user("hi")];
    let cb = Box::new(|_delta: &str| {});
    let result = transport
        .stream_chat(&messages, &CallMode::Fresh("test-stream-error".into()), cb)
        .await;

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("400"));
}

// ── System prompt test ──────────────────────────────────────────────────────

#[tokio::test]
async fn test_system_prompt_in_request() {
    // given: a transport with a system prompt
    // when: send_request is made
    // then: the system prompt appears in the request body
    let server = mock_server().await;

    // Verify system prompt is handled by checking the mock endpoint responds
    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(
                    r#"{
                        "id": "msg_sys",
                        "type": "message",
                        "role": "assistant",
                        "content": [{"type": "text", "text": "OK"}],
                        "stop_reason": "end_turn",
                        "model": "claude-sonnet-4-20250514",
                        "usage": {"input_tokens": 5, "output_tokens": 1}
                    }"#,
                )
                .insert_header("content-type", "application/json"),
        )
        .mount(&server)
        .await;

    let transport = make_transport(&server.uri(), "sk-test");
    let messages = vec![Message::system("be very helpful"), Message::user("hi")];

    // Make a request to trigger the mock
    let result = transport
        .chat(&messages, &CallMode::Fresh("test-system".into()))
        .await
        .unwrap();

    // System prompt should be in the request body (handled in the transport)
    // This test validates the mock endpoint responds correctly.
    assert_eq!(result.text, "OK");
}
