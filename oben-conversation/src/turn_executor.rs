/// Turn executor — deep module for the core agent turn cycle.
///
/// Encapsulates the full turn cycle: budget check → compression → LLM call
/// → tool dispatch → repeat until no more tool calls.
///
/// This is a **deep** module: callers cross one small interface (`execute_turn`)
/// and get a large amount of behaviour per unit of interface they learn.

use std::sync::Arc;
use anyhow::Result;
use tracing::info;

use crate::budget::IterationBudget;
use crate::context::ContextEngine;
use oben_models::{Message, MessageRole, TransportProvider};

// ---------------------------------------------------------------------------
// TurnResult — what the executor returns after executing a turn
// ---------------------------------------------------------------------------

pub struct TurnResult {
    pub text: String,
    pub messages: Vec<Message>,
}

// ---------------------------------------------------------------------------
// TurnExecutor — the deep turn cycle
// ---------------------------------------------------------------------------

/// Executes the full agent turn cycle: budget → compress → LLM → dispatch → repeat.
///
/// **Deep module**: one method, high leverage. Callers don't need to understand
/// the internals of token tracking, compression decisions, or streaming setup.
pub struct TurnExecutor {
    context_engine: ContextEngine,
    budget: IterationBudget,
    transport: Box<dyn TransportProvider>,
    tools: Arc<oben_tools::ToolRegistry>,
}

impl TurnExecutor {
    pub fn new(
        transport: impl TransportProvider + 'static,
        tools: Arc<oben_tools::ToolRegistry>,
        max_iterations: usize,
        max_messages: usize,
    ) -> Self {
        Self::with_config(
            transport,
            tools,
            max_iterations,
            max_messages,
            crate::compression::CompressionConfig::default(),
        )
    }

    pub fn with_config(
        transport: impl TransportProvider + 'static,
        tools: Arc<oben_tools::ToolRegistry>,
        max_iterations: usize,
        _max_messages: usize,
        engine_config: crate::compression::CompressionConfig,
    ) -> Self {
        Self {
            context_engine: ContextEngine::with_config(engine_config),
            budget: IterationBudget::new(max_iterations),
            transport: Box::new(transport),
            tools,
        }
    }

    /// Execute one turn: budget check → compress → LLM → tool dispatch → repeat.
    ///
    /// If `delta_callback` is provided, text tokens are streamed to it via a
    /// lock-free channel with batched flushing (~512 bytes per flush).
    pub async fn execute_turn(
        &mut self,
        messages: &mut Vec<Message>,
        user_message: Message,
        call_mode: &oben_models::CallMode,
        delta_callback: Option<oben_models::StreamDeltaCallback>,
    ) -> Result<TurnResult> {
        // Add user message to session
        messages.push(user_message);

        // Streaming setup: lock-free channel with batched callback dispatch.
        // The channel buffer of 4096 tokens is far larger than any single
        // LLM response delta, making overflow practically impossible.
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
            self.budget.check()?;

            // Auto-compression: check if context is getting full before making an LLM call.
            if self.context_engine.should_compress(messages) {
                info!(
                    "Auto-compression: context full ({} messages, {} est. tokens), compressing",
                    messages.len(),
                    self.context_engine.estimate_tokens(messages),
                );
                self.maybe_compress(messages).await?;
            }

            // Get LLM response (streaming or non-streaming).
            let response = if has_callback {
                let tx_clone = tx.clone();
                let wrapper: oben_models::StreamDeltaCallback =
                    Box::new(move |text: &str| {
                        // try_send is non-blocking, channel buffer is 4096.
                        let _ = tx_clone.try_send(text.to_string());
                    });
                self.transport.stream_chat(messages, &call_mode, wrapper).await?
            } else {
                self.transport.chat(messages, &call_mode).await?
            };

            let tool_calls = &response.tool_calls;
            let text = &response.text;

            // Update token tracking from API response
            if let Some(tokens) = response.tokens_used {
                self.context_engine.update_from_response(tokens, 0, tokens);
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
            messages.push(assistant_msg);

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
                    if let Some(last_tool_result) = messages.last().and_then(|m| {
                        if m.role == MessageRole::Tool {
                            m.content.to_text_ref()
                        } else {
                            None
                        }
                    }) {
                        if !last_tool_result.is_empty() {
                            return Ok(TurnResult {
                                text: last_tool_result.to_string(),
                                messages: messages.clone(),
                            });
                        }
                    }
                }
                return Ok(TurnResult {
                    text: text.trim().to_string(),
                    messages: messages.clone(),
                });
            }

            // Dispatch tool calls
            for call in tool_calls {
                let result = self.tools.execute(&call.tool_name, &call.arguments).await;
                messages.push(Message::tool_result(&call.id, &result.output));
            }
        }
    }

    /// Compress context if needed.
    pub async fn maybe_compress(&mut self, messages: &mut Vec<Message>) -> Result<()> {
        if !self.context_engine.should_compress(messages) {
            return Ok(());
        }
        self.context_engine
            .compress(messages, Some(&*self.transport), None)
            .await?;
        Ok(())
    }

    // ── Session lifecycle hooks ────────────────────────────────────────

    pub fn on_session_start(
        &mut self,
        session_id: &str,
        model_name: &str,
        context_length: Option<usize>,
    ) {
        self.context_engine.on_session_start(session_id, model_name, context_length);
    }

    pub fn on_session_reset(&mut self) {
        self.context_engine.on_session_reset();
    }

    pub fn on_session_end(&mut self, session_id: &str) {
        self.context_engine.on_session_end(session_id);
    }

    pub async fn preflight_check(
        &mut self,
        messages: &mut Vec<Message>,
    ) -> Result<usize> {
        self.context_engine
            .preflight_check(messages, Some(&*self.transport), None)
            .await
    }

    pub fn message_count(&self, messages: &[Message]) -> usize {
        messages.len()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compression::CompressionConfig;
    use oben_models::{TransportResponse, TransportToolCall};

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

    #[tokio::test]
    async fn test_execute_turn_text_response() {
        let mock = Arc::new(MockTransport {
            responses: vec![TransportResponse {
                text: "Hello!".to_string(),
                tool_calls: vec![],
                tokens_used: Some(10),
            }],
            call_count: Arc::new(std::sync::Mutex::new(0)),
        });

        let mut executor = TurnExecutor::new(
            mock,
            Arc::new(oben_tools::ToolRegistry::new()),
            10,
            100,
        );
        let mut messages = Vec::new();

        let result = executor.execute_turn(
            &mut messages,
            Message::user("Hi"),
            &oben_models::CallMode::Fresh("test-session".to_string()),
            None,
        ).await.unwrap();
        assert_eq!(result.text, "Hello!");
        assert_eq!(result.messages.len(), 2);
    }

    #[tokio::test]
    async fn test_execute_turn_with_tool_call() {
        let mock = Arc::new(MockTransport {
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
        });

        let mut executor = TurnExecutor::new(
            mock,
            Arc::new(oben_tools::ToolRegistry::new()),
            10,
            100,
        );
        let mut messages = Vec::new();

        let result = executor.execute_turn(
            &mut messages,
            Message::user("list files"),
            &oben_models::CallMode::Fresh("test-session".to_string()),
            None,
        ).await.unwrap();
        assert_eq!(result.text, "Done!");
        assert_eq!(result.messages.len(), 4);
    }

    #[tokio::test]
    async fn test_execute_turn_empty_response() {
        let mock = Arc::new(MockTransport {
            responses: vec![TransportResponse {
                text: "".to_string(),
                tool_calls: vec![],
                tokens_used: Some(5),
            }],
            call_count: Arc::new(std::sync::Mutex::new(0)),
        });

        let mut executor = TurnExecutor::new(
            mock,
            Arc::new(oben_tools::ToolRegistry::new()),
            10,
            100,
        );
        let mut messages = Vec::new();

        let result = executor.execute_turn(
            &mut messages,
            Message::user("Hi"),
            &oben_models::CallMode::Fresh("test-session".to_string()),
            None,
        ).await.unwrap();
        assert_eq!(result.text, "");
        assert_eq!(result.messages.len(), 2);
    }

    #[tokio::test]
    async fn test_execute_turn_budget_exceeded() {
        let mock = Arc::new(MockTransport {
            responses: vec![TransportResponse {
                text: "".to_string(),
                tool_calls: vec![TransportToolCall {
                    id: "call-1".to_string(),
                    tool_name: "shell".to_string(),
                    arguments: serde_json::json!({"command": "ls"}),
                }],
                tokens_used: Some(1),
            }],
            call_count: Arc::new(std::sync::Mutex::new(0)),
        });

        let mut executor = TurnExecutor::new(
            mock,
            Arc::new(oben_tools::ToolRegistry::new()),
            2,
            100,
        );
        let mut messages = Vec::new();

        let result = executor.execute_turn(
            &mut messages,
            Message::user("Hi"),
            &oben_models::CallMode::Fresh("test-session".to_string()),
            None,
        ).await;
        assert!(result.is_err());
    }

    // ── Streaming tests ────────────────────────────────────────────────

    #[tokio::test]
    async fn test_streaming_basic() {
        let mock = Arc::new(MockTransport {
            responses: vec![TransportResponse {
                text: "Hello from stream!".to_string(),
                tool_calls: vec![],
                tokens_used: Some(10),
            }],
            call_count: Arc::new(std::sync::Mutex::new(0)),
        });

        let mut executor = TurnExecutor::new(
            mock,
            Arc::new(oben_tools::ToolRegistry::new()),
            10,
            100,
        );
        let mut messages = Vec::new();

        let output = Arc::new(std::sync::Mutex::new(String::new()));
        let output_clone = output.clone();
        let cb: oben_models::StreamDeltaCallback =
            Box::new(move |text: &str| output_clone.lock().unwrap().push_str(text));

        let result = executor.execute_turn(
            &mut messages,
            Message::user("Hi"),
            &oben_models::CallMode::Fresh("test-session".to_string()),
            Some(cb),
        ).await.unwrap();
        assert_eq!(result.text, "Hello from stream!");
        assert_eq!(messages.len(), 2);
        assert_eq!(*output.lock().unwrap(), "Hello from stream!");
    }

    #[tokio::test]
    async fn test_streaming_with_tool_calls() {
        let mock = Arc::new(MockTransport {
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
        });

        let mut executor = TurnExecutor::new(
            mock,
            Arc::new(oben_tools::ToolRegistry::new()),
            10,
            100,
        );
        let mut messages = Vec::new();

        let output = Arc::new(std::sync::Mutex::new(String::new()));
        let output_clone = output.clone();
        let cb: oben_models::StreamDeltaCallback =
            Box::new(move |text: &str| output_clone.lock().unwrap().push_str(text));

        let result = executor.execute_turn(
            &mut messages,
            Message::user("list files"),
            &oben_models::CallMode::Fresh("test-session".to_string()),
            Some(cb),
        ).await.unwrap();
        assert_eq!(result.text, "All done!");
        assert_eq!(*output.lock().unwrap(), "Let me check.All done!");
        assert_eq!(result.messages.len(), 4);
    }

    // ── Session lifecycle tests ────────────────────────────────────────

    #[tokio::test]
    async fn test_session_lifecycle() {
        let mock = Arc::new(MockTransport {
            responses: vec![TransportResponse {
                text: "Hi back!".to_string(),
                tool_calls: vec![],
                tokens_used: Some(5),
            }],
            call_count: Arc::new(std::sync::Mutex::new(0)),
        });

        let mut executor = TurnExecutor::new(
            mock,
            Arc::new(oben_tools::ToolRegistry::new()),
            10,
            100,
        );

        executor.on_session_start("session-1", "gpt-4", Some(128_000));
        let mut messages = vec![Message::system("test")];
        executor.on_session_reset();
        let _ = executor.message_count(&messages);
        executor.on_session_end("session-1");
    }

    // ── Auto-compression tests ─────────────────────────────────────────

    #[tokio::test]
    async fn test_auto_compression_fires() {
        let mock = Arc::new(MockTransport {
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
        });

        let config = CompressionConfig {
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

        let mut executor = TurnExecutor::with_config(
            mock.clone(),
            Arc::new(oben_tools::ToolRegistry::new()),
            10,
            100,
            config,
        );

        let long_content = "The quick brown fox jumps over the lazy dog. ".repeat(100);
        let mut messages: Vec<Message> = (0..10)
            .map(|i| Message::user(&format!("Message {}: {}", i, long_content)))
            .collect();

        let result = executor.execute_turn(
            &mut messages,
            Message::user("Hi"),
            &oben_models::CallMode::Fresh("test-session".to_string()),
            None,
        ).await.unwrap();

        assert_eq!(*mock.call_count.lock().unwrap(), 2);
        assert_eq!(result.text, "Hello from compressed context!");
        assert!(messages.len() < 12, "Messages should be compressed");
    }

    #[tokio::test]
    async fn test_no_compression_when_under_threshold() {
        let mock = Arc::new(MockTransport {
            responses: vec![TransportResponse {
                text: "Hello!".to_string(),
                tool_calls: vec![],
                tokens_used: Some(50),
            }],
            call_count: Arc::new(std::sync::Mutex::new(0)),
        });

        let config = CompressionConfig {
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

        let mut executor = TurnExecutor::with_config(
            mock.clone(),
            Arc::new(oben_tools::ToolRegistry::new()),
            10,
            100,
            config,
        );

        let mut messages = vec![Message::user("Hi")];

        let result = executor.execute_turn(
            &mut messages,
            Message::user("Hello"),
            &oben_models::CallMode::Fresh("test-session".to_string()),
            None,
        ).await.unwrap();

        assert_eq!(*mock.call_count.lock().unwrap(), 1);
        assert_eq!(result.text, "Hello!");
    }
}
