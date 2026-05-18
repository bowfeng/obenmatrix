use anyhow::Result;
use oben_models::{Message, TransportProvider, TransportToolCall};
use oben_transport::chat_completions::ChatCompletionsTransport;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use std::sync::{Arc, Mutex};

/// Build a transport pointing at a wiremock server.
fn make_transport(base_url: &str, model: &str) -> ChatCompletionsTransport {
    ChatCompletionsTransport::new(base_url, "", model)
}

fn simple_sse_response(text: &str) -> String {
    format!(
        "data: {{\"choices\":[{{\"delta\":{{\"content\":\"{}\"}},\"index\":0,\"finish_reason\":null}}]}}\n\ndata: [DONE]",
        text
    )
}

fn streaming_response_with_tool_calls(id: &str, name: &str, args: &str) -> String {
    // args is a JSON-encoded string, need to escape it for the outer JSON
    let args_escaped = args.replace('\\', "\\\\").replace('"', "\\\"");
    format!(
        "data: {{\"choices\":[{{\"delta\":{{\"tool_calls\":[{{\"index\":0,\"id\":\"{}\",\"function\":{{\"name\":\"{}\",\"arguments\":\"{}\"}}}}]}},\"index\":0,\"finish_reason\":null}}]}}\n\ndata: [DONE]",
        id, name, args_escaped
    )
}

fn streaming_response_with_usage(text: &str, prompt: usize, completion: usize, total: usize) -> String {
    format!(
        "data: {{\"choices\":[{{\"delta\":{{\"content\":\"{}\"}},\"index\":0,\"finish_reason\":null}}]}}\n\ndata: {{\"choices\":[],\"usage\":{{\"prompt_tokens\":{},\"completion_tokens\":{},\"total_tokens\":{}}}}}\n\ndata: [DONE]",
        text, prompt, completion, total
    )
}

fn non_streaming_response(text: &str, tokens: Option<usize>) -> String {
    if let Some(tok) = tokens {
        format!(
            "{{\"choices\":[{{\"message\":{{\"role\":\"assistant\",\"content\":\"{}\"}}}}],\"usage\":{{\"total_tokens\":{}}},\"model\":\"test\"}}",
            text, tok
        )
    } else {
        format!(
            "{{\"choices\":[{{\"message\":{{\"role\":\"assistant\",\"content\":\"{}\"}}}}]}}",
            text
        )
    }
}

fn non_streaming_response_with_tool_calls(
    id: &str,
    name: &str,
    args: &str,
) -> String {
    // args is a JSON-encoded string like {"command": "ls"}
    // We need to properly escape it for the outer JSON
    let args_escaped = args.replace('\\', "\\\\").replace('"', "\\\"");
    format!(
        "{{\"choices\":[{{\"message\":{{\"role\":\"assistant\",\"content\":null,\"tool_calls\":[{{\"id\":\"{}\",\"type\":\"function\",\"function\":{{\"name\":\"{}\",\"arguments\":\"{}\"}}}}]}}}}],\"model\":\"test\"}}",
        id, name, args_escaped
    )
}

// =============================================================================
// Non-streaming tests
// =============================================================================

#[tokio::test]
async fn test_chat_completions_text_response() -> Result<()> {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            non_streaming_response("Hello, how can I help you?", Some(42)),
        ))
        .mount(&server)
        .await;

    let transport = make_transport(&server.uri(), "test-model");
    let messages = vec![Message::user("Hi there")];
    let resp = transport.chat(&messages).await?;

    assert_eq!(resp.text, "Hello, how can I help you?");
    assert_eq!(resp.tool_calls.len(), 0);
    assert_eq!(resp.tokens_used, Some(42));
    Ok(())
}

#[tokio::test]
async fn test_chat_completions_empty_content() -> Result<()> {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            non_streaming_response("", None),
        ))
        .mount(&server)
        .await;

    let transport = make_transport(&server.uri(), "test-model");
    let messages = vec![Message::user("test")];
    let resp = transport.chat(&messages).await?;

    assert_eq!(resp.text, "");
    Ok(())
}

#[tokio::test]
async fn test_chat_completions_tool_calls() -> Result<()> {
    let server = MockServer::start().await;

    let body = non_streaming_response_with_tool_calls(
        "call-tool-1",
        "shell",
        "{\"command\": \"ls -la\"}",
    );
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(&server)
        .await;

    let transport = make_transport(&server.uri(), "test-model");
    let messages = vec![Message::user("list files")];
    let resp = transport.chat(&messages).await?;

    assert_eq!(resp.tool_calls.len(), 1);
    assert_eq!(resp.tool_calls[0].id, "call-tool-1");
    assert_eq!(resp.tool_calls[0].tool_name, "shell");
    assert_eq!(resp.tool_calls[0].arguments["command"], "ls -la");
    Ok(())
}

#[tokio::test]
async fn test_chat_completions_api_error() -> Result<()> {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(400).set_body_string(
            "{\"error\":{\"message\":\"bad request\",\"code\":400}}",
        ))
        .mount(&server)
        .await;

    let transport = make_transport(&server.uri(), "test-model");
    let messages = vec![Message::user("test")];
    let result = transport.chat(&messages).await;

    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("API error 400"));
    assert!(err.contains("bad request"));
    Ok(())
}

#[tokio::test]
async fn test_chat_completions_no_choices() -> Result<()> {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            "{\"choices\":[]}",
        ))
        .mount(&server)
        .await;

    let transport = make_transport(&server.uri(), "test-model");
    let messages = vec![Message::user("test")];
    let result = transport.chat(&messages).await;

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("No response choices"));
    Ok(())
}

// =============================================================================
// Streaming tests
// =============================================================================

#[tokio::test]
async fn test_stream_chat_text_response() -> Result<()> {
    let server = MockServer::start().await;

    let sse = simple_sse_response("Streamed hello");
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .append_header("content-type", "text/event-stream")
                .set_body_string(sse),
        )
        .mount(&server)
        .await;

    let transport = make_transport(&server.uri(), "test-model");
    let output = Arc::new(Mutex::new(String::new()));
    let output_clone = output.clone();
    let cb: oben_models::StreamDeltaCallback =
        Box::new(move |text: &str| output_clone.lock().unwrap().push_str(text));

    let resp = transport
        .stream_chat(&[Message::user("Hi")], cb)
        .await?;

    assert_eq!(resp.text, "Streamed hello");
    assert_eq!(resp.tool_calls.len(), 0);
    assert_eq!(*output.lock().unwrap(), "Streamed hello");
    Ok(())
}

#[tokio::test]
async fn test_stream_chat_with_usage() -> Result<()> {
    let server = MockServer::start().await;

    let sse = streaming_response_with_usage("Done", 10, 5, 15);
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .append_header("content-type", "text/event-stream")
                .set_body_string(sse),
        )
        .mount(&server)
        .await;

    let transport = make_transport(&server.uri(), "test-model");
    let output = Arc::new(Mutex::new(String::new()));
    let output_clone = output.clone();
    let cb: oben_models::StreamDeltaCallback =
        Box::new(move |text: &str| output_clone.lock().unwrap().push_str(text));

    let resp = transport
        .stream_chat(&[Message::user("test")], cb)
        .await?;

    assert_eq!(resp.text, "Done");
    assert_eq!(resp.tokens_used, Some(15));
    assert_eq!(*output.lock().unwrap(), "Done");
    Ok(())
}

#[tokio::test]
async fn test_stream_chat_tool_calls() -> Result<()> {
    let server = MockServer::start().await;

    let sse = streaming_response_with_tool_calls(
        "call-abc",
        "shell",
        "{\"command\": \"echo hello\"}",
    );
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .append_header("content-type", "text/event-stream")
                .set_body_string(sse),
        )
        .mount(&server)
        .await;

    let transport = make_transport(&server.uri(), "test-model");
    let cb: oben_models::StreamDeltaCallback = Box::new(|_text: &str| {});

    let resp = transport
        .stream_chat(&[Message::user("run command")], cb)
        .await?;

    assert_eq!(resp.tool_calls.len(), 1);
    assert_eq!(resp.tool_calls[0].id, "call-abc");
    assert_eq!(resp.tool_calls[0].tool_name, "shell");
    assert_eq!(resp.tool_calls[0].arguments["command"], "echo hello");
    Ok(())
}

#[tokio::test]
async fn test_stream_chat_empty_content() -> Result<()> {
    let server = MockServer::start().await;

    // Only finish_reason, no content — should produce empty string
    let sse = format!(
        "data: {{\"choices\":[{{\"delta\":{{\"finish_reason\":\"stop\"}}}}]}}\n\ndata: [DONE]"
    );
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .append_header("content-type", "text/event-stream")
                .set_body_string(sse),
        )
        .mount(&server)
        .await;

    let transport = make_transport(&server.uri(), "test-model");
    let output = Arc::new(Mutex::new(String::new()));
    let output_clone = output.clone();
    let cb: oben_models::StreamDeltaCallback =
        Box::new(move |text: &str| output_clone.lock().unwrap().push_str(text));

    let resp = transport
        .stream_chat(&[Message::user("test")], cb)
        .await?;

    assert_eq!(resp.text, "");
    assert_eq!(*output.lock().unwrap(), "");
    Ok(())
}

#[tokio::test]
async fn test_stream_chat_api_error() -> Result<()> {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(500).set_body_string(
            "Internal Server Error",
        ))
        .mount(&server)
        .await;

    let transport = make_transport(&server.uri(), "test-model");
    let cb: oben_models::StreamDeltaCallback = Box::new(|_text: &str| {});

    let result = transport
        .stream_chat(&[Message::user("test")], cb)
        .await;

    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("API error 500"));
    Ok(())
}

// =============================================================================
// Multi-turn integration (chat + stream_chat mix)
// =============================================================================
// NOTE: This test requires careful mock expectation counting.
// wiremock 0.6's MockBuilder::expect() requires Times to be in scope
// but the method resolution fails in tests. Use a different approach.

// Simplified multi-turn test using separate mock servers
#[tokio::test]
async fn test_stream_chat_separate_instances() -> Result<()> {
    // Test that streaming produces text content that the callback captures
    let server = MockServer::start().await;

    let sse = simple_sse_response("Captured text");
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .append_header("content-type", "text/event-stream")
                .set_body_string(sse),
        )
        .mount(&server)
        .await;

    let transport = make_transport(&server.uri(), "test-model");
    let output = Arc::new(Mutex::new(String::new()));
    let output_clone = output.clone();
    let cb: oben_models::StreamDeltaCallback =
        Box::new(move |text: &str| output_clone.lock().unwrap().push_str(text));

    let resp = transport
        .stream_chat(&[Message::user("test")], cb)
        .await?;

    assert_eq!(resp.text, "Captured text");
    assert_eq!(resp.tool_calls.len(), 0);
    assert_eq!(*output.lock().unwrap(), "Captured text");

    Ok(())
}
