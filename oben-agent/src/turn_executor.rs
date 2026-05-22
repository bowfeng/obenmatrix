/// Turn executor — deep module for the core agent turn cycle.
///
/// **Stateless**: `TurnExecutor` owns nothing — all resources (ContextEngine,
/// Budget, Transport, Tools) are passed as function parameters.
///
/// Encapsulates the full turn cycle: budget check → compression → LLM call
/// → tool dispatch → repeat until no more tool calls.
///
/// This is a **deep** module: callers cross one small interface (`execute_turn`)
/// and get a large amount of behaviour per unit of interface they learn.

use std::sync::Arc;

use anyhow::Result;
use tracing::info;

use crate::context::ContextEngine;
use oben_models::{Message, MessageRole, Session, TransportProvider, SessionStore};

// ---------------------------------------------------------------------------
// TurnResult — what the executor returns after executing a turn
// ---------------------------------------------------------------------------

pub struct TurnResult {
    pub text: String,
    pub messages: Vec<Message>,
}

// ---------------------------------------------------------------------------
// TurnExecutor — stateless turn cycle
// ---------------------------------------------------------------------------

/// Stateless executor — the full agent turn cycle.
///
/// **Deep module**: one method, high leverage. All resources (ContextEngine,
/// Budget, Transport, Tools) are passed as function parameters — `TurnExecutor`
/// owns none of them.
///
/// **Responsibilities**: budget check → compression → LLM call → tool dispatch → repeat.
///
/// Callers don't need to understand the internals of token tracking,
/// compression decisions, or streaming setup.
pub struct TurnExecutor;

impl TurnExecutor {
    /// Execute one turn: budget check → compress → LLM → dispatch → repeat.
    ///
    /// **Parameters** — all resources passed as parameters:
    /// - `context_engine`: token tracking & compression
    /// - `transport`: LLM API (dyn trait)
    /// - `tools`: tool registry
    /// - `store`: session store
    /// - `session_id`: which session to operate on
    /// - `user_message`: the user's input
    /// - `call_mode`: Fresh or Incremental
    /// - `delta_callback`: optional streaming callback
    pub async fn execute_turn(
        context_engine: &mut dyn ContextEngine,
        transport: &dyn TransportProvider,
        tools: &Arc<oben_tools::ToolRegistry>,
        store: &mut dyn SessionStore,
        session_id: &str,
        user_message: Message,
        call_mode: &oben_models::CallMode,
        delta_callback: Option<oben_models::StreamDeltaCallback>,
    ) -> Result<TurnResult> {
        let session = store
            .session_mut(session_id)
            .ok_or_else(|| anyhow::anyhow!("Session not found: {}", session_id))?;

        // Add user message to session
        session.messages.push(user_message);

        // Streaming setup: lock-free channel with batched callback dispatch.
        let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(4096);
        let mut callback_handle: Option<tokio::task::JoinHandle<()>> = None;
        let has_callback = delta_callback.is_some();

        if let Some(cb) = delta_callback {
            let cb = Arc::new(std::sync::Mutex::new(cb));
            callback_handle = Some(tokio::task::spawn(async move {
                let mut buf = String::new();
                const FLUSH_THRESHOLD: usize = 512;
                while let Some(chunk) = rx.recv().await {
                    buf.push_str(&chunk);
                    if buf.len() >= FLUSH_THRESHOLD {
                        let text = std::mem::take(&mut buf);
                        cb.lock().unwrap()(&text);
                    }
                }
                // Channel closed — flush remaining output
                if !buf.is_empty() {
                    cb.lock().unwrap()(&buf);
                }
            }));
        }

        // Core loop with tool dispatch
        loop {
            // Budget check — managed by Agent, not TurnExecutor

            // Auto-compression: check if context is getting full
            if context_engine.should_compact(&session.messages) {
                info!(
                    "Auto-compression: context full ({} messages, {} est. tokens), compressing",
                    session.messages.len(),
                    context_engine.estimate_tokens(&session.messages),
                );
                context_engine
                    .compact(&mut session.messages, Some(transport), None)
                    .await?;
            }

            // Get LLM response (streaming or non-streaming).
            let response = if has_callback {
                let tx_clone = tx.clone();
                let wrapper: oben_models::StreamDeltaCallback =
                    Box::new(move |text: &str| {
                        // try_send is non-blocking, channel buffer is 4096.
                        let _ = tx_clone.try_send(text.to_string());
                    });
                transport.stream_chat(&session.messages, call_mode, wrapper).await?
            } else {
                transport.chat(&session.messages, call_mode).await?
            };

            let tool_calls = &response.tool_calls;
            let text = &response.text;

            // Update token tracking from API response
            if let Some(tokens) = response.tokens_used {
                context_engine
                    .update_from_response(tokens, 0, tokens);
            }

            // Add assistant response to session
            let assistant_msg = if !tool_calls.is_empty() {
                let tool_call_data = tool_calls
                    .iter()
                    .map(oben_models::ToolCall::from_transport)
                    .collect();
                Message::assistant_tool_calls(tool_call_data)
            } else {
                Message::assistant(text.trim().to_string())
            };
            session.messages.push(assistant_msg);

            if tool_calls.is_empty() {
                // Flush remaining buffered output before returning
                if has_callback {
                    drop(tx);
                    if let Some(handle) = callback_handle.take() {
                        let _ = handle.await;
                    }
                }

                // When text is empty after tool results, return the tool
                // results instead of empty string.
                if text.trim().is_empty() {
                    if let Some(last_tool_result) = session.messages.last().and_then(|m| {
                        if m.role == MessageRole::Tool {
                            m.content.to_text_ref()
                        } else {
                            None
                        }
                    }) {
                        if !last_tool_result.is_empty() {
                            return Ok(TurnResult {
                                text: last_tool_result.to_string(),
                                messages: session.messages.clone(),
                            });
                        }
                    }
                }
                return Ok(TurnResult {
                    text: text.trim().to_string(),
                    messages: session.messages.clone(),
                });
            }

            // Dispatch tool calls
            for call in tool_calls {
                let result = tools.execute(&call.tool_name, &call.arguments).await;
                session.messages.push(Message::tool_result(&call.id, &result.output));
            }
        }
    }


}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compact::CompactCofig;
    use crate::compact_context::CompactContextEngine;
    use crate::turn_executor::TurnExecutor;
    use oben_models::{Session, SessionStore, TransportResponse, TransportToolCall};

    /// In-memory test double for SessionStore — no SQLite needed.
    struct TestSessionStore {
        sessions: std::collections::HashMap<String, Session>,
    }

    impl TestSessionStore {
        fn new() -> Self { Self { sessions: std::collections::HashMap::new() } }

        fn insert(&mut self, name: &str, msgs: Vec<Message>) -> String {
            let session = Session::new(name);
            let id = session.id.clone();
            let mut s = session;
            s.messages = msgs;
            self.sessions.insert(id.clone(), s);
            id
        }
    }

    impl SessionStore for TestSessionStore {
        fn session_mut(&mut self, session_id: &str) -> Option<&mut Session> {
            self.sessions.get_mut(session_id)
        }
        fn session(&self, session_id: &str) -> Option<&Session> {
            self.sessions.get(session_id)
        }
    }

    struct MockTransport {
        responses: Vec<TransportResponse>,
        call_count: Arc<std::sync::Mutex<usize>>,
    }

    #[async_trait::async_trait]
    impl TransportProvider for MockTransport {
        fn name(&self) -> &str { "mock" }

        async fn chat(&self, _messages: &[Message], _mode: &oben_models::CallMode) -> Result<TransportResponse> {
            let mut count = self.call_count.lock().unwrap();
            *count += 1;
            let idx = (*count - 1).min(self.responses.len() - 1);
            Ok(self.responses[idx].clone())
        }

        async fn stream_chat(
            &self,
            _messages: &[Message],
            _mode: &oben_models::CallMode,
            mut _cb: oben_models::StreamDeltaCallback,
        ) -> Result<TransportResponse> {
            let mut count = self.call_count.lock().unwrap();
            *count += 1;
            let idx = (*count - 1).min(self.responses.len() - 1);
            let resp = self.responses[idx].clone();
            if !resp.text.is_empty() {
                _cb(&resp.text);
            }
            Ok(resp)
        }
    }

    // ── Non-streaming tests ────────────────────────────────────────────

    fn make_engine() -> Box<dyn crate::context::ContextEngine> {
        Box::new(CompactContextEngine::new())
    }

    #[tokio::test]
    async fn test_execute_turn_text_response() {
        let mock = MockTransport {
            responses: vec![TransportResponse {
                text: "Hello!".to_string(),
                tool_calls: vec![],
                tokens_used: Some(10),
            }],
            call_count: Arc::new(std::sync::Mutex::new(0)),
        };
        let mut context_engine = make_engine();
        let tools = Arc::new(oben_tools::ToolRegistry::new());

        let mut store = TestSessionStore::new();
        let sid = store.insert("test-session", Vec::new());

        let result = TurnExecutor::execute_turn(
            &mut *context_engine, &mock, &tools,
            &mut store, &sid,
            Message::user("Hi"),
            &oben_models::CallMode::Fresh(sid.clone()),
            None,
        ).await.unwrap();
        assert_eq!(result.text, "Hello!");
        assert_eq!(result.messages.len(), 2);
    }

    #[tokio::test]
    async fn test_execute_turn_with_tool_call() {
        let mock = MockTransport {
            responses: vec![
                TransportResponse {
                    text: "Let me check.".to_string(),
                    tool_calls: vec![TransportToolCall {
                        id: "call-1".to_string(),
                        tool_name: "shell".to_string(),
                        arguments: serde_json::json!({"command": "ls"}),
                    }],
                    tokens_used: Some(20),
                },
                TransportResponse {
                    text: "Done!".to_string(),
                    tool_calls: vec![],
                    tokens_used: Some(10),
                },
            ],
            call_count: Arc::new(std::sync::Mutex::new(0)),
        };
        let mut context_engine = make_engine();
        let tools = Arc::new(oben_tools::ToolRegistry::new());

        let mut store = TestSessionStore::new();
        let sid = store.insert("test-session", Vec::new());

        let result = TurnExecutor::execute_turn(
            &mut *context_engine, &mock, &tools,
            &mut store, &sid,
            Message::user("list files"),
            &oben_models::CallMode::Fresh(sid.clone()),
            None,
        ).await.unwrap();
        assert_eq!(result.text, "Done!");
        assert_eq!(result.messages.len(), 4);
    }

    #[tokio::test]
    async fn test_execute_turn_empty_response() {
        let mock = MockTransport {
            responses: vec![TransportResponse {
                text: "".to_string(),
                tool_calls: vec![],
                tokens_used: Some(5),
            }],
            call_count: Arc::new(std::sync::Mutex::new(0)),
        };
        let mut context_engine = make_engine();
        let tools = Arc::new(oben_tools::ToolRegistry::new());

        let mut store = TestSessionStore::new();
        let sid = store.insert("test-session", Vec::new());

        let result = TurnExecutor::execute_turn(
            &mut *context_engine, &mock, &tools,
            &mut store, &sid,
            Message::user("Hi"),
            &oben_models::CallMode::Fresh(sid.clone()),
            None,
        ).await.unwrap();
        assert_eq!(result.text, "");
        assert_eq!(result.messages.len(), 2);
    }

    // ── Streaming tests ────────────────────────────────────────────────

    #[tokio::test]
    async fn test_streaming_basic() {
        let mock = MockTransport {
            responses: vec![TransportResponse {
                text: "Hello from stream!".to_string(),
                tool_calls: vec![],
                tokens_used: Some(10),
            }],
            call_count: Arc::new(std::sync::Mutex::new(0)),
        };
        let mut context_engine = make_engine();
        let tools = Arc::new(oben_tools::ToolRegistry::new());

        let mut store = TestSessionStore::new();
        let sid = store.insert("test-session", Vec::new());

        let output = Arc::new(std::sync::Mutex::new(String::new()));
        let output_clone = output.clone();
        let cb: oben_models::StreamDeltaCallback =
            Box::new(move |text: &str| output_clone.lock().unwrap().push_str(text));

        let result = TurnExecutor::execute_turn(
            &mut *context_engine, &mock, &tools,
            &mut store, &sid,
            Message::user("Hi"),
            &oben_models::CallMode::Fresh(sid.clone()),
            Some(cb),
        ).await.unwrap();
        assert_eq!(result.text, "Hello from stream!");
        assert_eq!(store.session(&sid).unwrap().messages.len(), 2);
        assert_eq!(*output.lock().unwrap(), "Hello from stream!");
    }

    #[tokio::test]
    async fn test_streaming_with_tool_calls() {
        let mock = MockTransport {
            responses: vec![
                TransportResponse {
                    text: "Let me check.".to_string(),
                    tool_calls: vec![TransportToolCall {
                        id: "call-1".to_string(),
                        tool_name: "shell".to_string(),
                        arguments: serde_json::json!({"command": "ls"}),
                    }],
                    tokens_used: Some(20),
                },
                TransportResponse {
                    text: "All done!".to_string(),
                    tool_calls: vec![],
                    tokens_used: Some(10),
                },
            ],
            call_count: Arc::new(std::sync::Mutex::new(0)),
        };
        let mut context_engine = make_engine();
        let tools = Arc::new(oben_tools::ToolRegistry::new());

        let mut store = TestSessionStore::new();
        let sid = store.insert("test-session", Vec::new());

        let output = Arc::new(std::sync::Mutex::new(String::new()));
        let output_clone = output.clone();
        let cb: oben_models::StreamDeltaCallback =
            Box::new(move |text: &str| output_clone.lock().unwrap().push_str(text));

        let result = TurnExecutor::execute_turn(
            &mut *context_engine, &mock, &tools,
            &mut store, &sid,
            Message::user("list files"),
            &oben_models::CallMode::Fresh(sid.clone()),
            Some(cb),
        ).await.unwrap();
        assert_eq!(result.text, "All done!");
        assert_eq!(*output.lock().unwrap(), "Let me check.All done!");
        assert_eq!(result.messages.len(), 4);
    }

    // ── Auto-compression tests ─────────────────────────────────────────

    fn make_engine_with_config(config: CompactCofig) -> Box<dyn crate::context::ContextEngine> {
        Box::new(CompactContextEngine::with_config(config))
    }

    #[tokio::test]
    async fn test_auto_compression_fires() {
        let mock = MockTransport {
            responses: vec![
                TransportResponse {
                    text: "## Active Task\nTest completed.".to_string(),
                    tool_calls: vec![],
                    tokens_used: Some(50),
                },
                TransportResponse {
                    text: "Hello from compressed context!".to_string(),
                    tool_calls: vec![],
                    tokens_used: Some(50),
                },
            ],
            call_count: Arc::new(std::sync::Mutex::new(0)),
        };
        let config = CompactCofig {
            context_length: 1000,
            threshold_percent: 0.5,
            protect_first_n: 2,
            tail_token_budget: 100,
            tail_min_messages: 2,
            tail_overhead: 1.5,
            ineffective_threshold: 10.0,
            max_ineffective_consecutive: 2,
            ..Default::default()
        };
        let mut context_engine = make_engine_with_config(config);
        let tools = Arc::new(oben_tools::ToolRegistry::new());

        let long_content = "The quick brown fox jumps over the lazy dog. ".repeat(100);
        let msgs: Vec<Message> = (0..10)
            .map(|i| Message::user(&format!("Message {}: {}", i, long_content)))
            .collect();

        let mut store = TestSessionStore::new();
        let sid = store.insert("test-session", msgs);

        let result = TurnExecutor::execute_turn(
            &mut *context_engine, &mock, &tools,
            &mut store, &sid,
            Message::user("Hi"),
            &oben_models::CallMode::Fresh(sid.clone()),
            None,
        ).await.unwrap();

        assert_eq!(*mock.call_count.lock().unwrap(), 2);
        assert_eq!(result.text, "Hello from compressed context!");
        let msg_count = store.session(&sid).unwrap().messages.len();
        assert!(msg_count < 12, "Messages should be compressed, got {}", msg_count);
    }

    #[tokio::test]
    async fn test_no_compression_when_under_threshold() {
        let mock = MockTransport {
            responses: vec![TransportResponse {
                text: "Hello!".to_string(),
                tool_calls: vec![],
                tokens_used: Some(50),
            }],
            call_count: Arc::new(std::sync::Mutex::new(0)),
        };
        let config = CompactCofig {
            context_length: 10_000,
            threshold_percent: 0.5,
            protect_first_n: 3,
            tail_token_budget: 20_000,
            tail_min_messages: 3,
            tail_overhead: 1.5,
            ineffective_threshold: 10.0,
            max_ineffective_consecutive: 2,
            ..Default::default()
        };
        let mut context_engine = make_engine_with_config(config);
        let tools = Arc::new(oben_tools::ToolRegistry::new());

        let mut store = TestSessionStore::new();
        let sid = store.insert("test-session", vec![Message::user("Hi")]);

        let result = TurnExecutor::execute_turn(
            &mut *context_engine, &mock, &tools,
            &mut store, &sid,
            Message::user("Hello"),
            &oben_models::CallMode::Fresh(sid.clone()),
            None,
        ).await.unwrap();

        assert_eq!(*mock.call_count.lock().unwrap(), 1);
        assert_eq!(result.text, "Hello!");
    }
}
