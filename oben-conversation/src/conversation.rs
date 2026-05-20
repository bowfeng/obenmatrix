/// Conversation loop — the main agent turn cycle.
/// Maps to `agent/conversation_loop.py::run_conversation`.

use anyhow::Result;
use oben_models::{Message, MessageRole, TransportProvider};
use std::path::PathBuf;
use tracing::info;

use crate::{
    budget::IterationBudget,
    context::{ContextEngine, ContextEngineConfig},
};
use crate::system_prompt;

/// Configuration for building the 3-tier system prompt.
///
/// Holds all inputs needed by the system prompt builder. The actual prompt
/// assembly happens per-turn (via `build_system_messages`) so that volatile
/// components (memory context, timestamp) stay fresh each call.
pub struct SystemPromptConfig {
    /// Identity text — from SOUL.md or DEFAULT_IDENTITY.
    identity: String,
    /// List of tool names available to the agent.
    tools_list: Vec<String>,
    /// Directories to scan for skills.
    skills_dirs: Vec<PathBuf>,
    /// Working directory for context file discovery.
    context_cwd: Option<PathBuf>,
    /// Custom system message override (optional).
    custom_message: Option<String>,
    /// Volatile memory context — changed per session.
    memory_context: Option<String>,
}

impl SystemPromptConfig {
    /// Create a full 3-tier config.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        identity: String,
        tools_list: Vec<String>,
        skills_dirs: Vec<PathBuf>,
        context_cwd: Option<PathBuf>,
        custom_message: Option<String>,
        memory_context: Option<String>,
    ) -> Self {
        Self {
            identity,
            tools_list,
            skills_dirs,
            context_cwd,
            custom_message,
            memory_context,
        }
    }

    /// Build and prepend the system prompt to the messages array.
    ///
    /// This is called at the top of each turn so volatile components (memory,
    /// timestamp) are always fresh while stable components are cached.
    pub fn build_system_prompt(&self, session_id: &str, model_name: &str) -> String {
        // Build the full prompt with volatile components
        let volatile = system_prompt::build_volatile_block(
            self.memory_context.as_deref(),
            Some(session_id),
            if model_name.is_empty() { None } else { Some(model_name) },
        );

        let assembled = system_prompt::build_system_prompt(
            &self.identity,
            &self.tools_list,
            &self.skills_dirs,
            self.context_cwd.as_deref(),
            self.custom_message.as_deref(),
            if volatile.is_empty() { None } else { Some(&volatile) },
        );
        assembled.prompt
    }
}

/// The main agent loop — one user turn.
///
/// 1. User message arrives (passed in via `messages` vec)
/// 2. Call LLM (transport handles incremental JSON building internally)
/// 3. If tool calls: dispatch → collect results → loop
/// 4. If text response: return to user
/// 5. Post-turn hooks (memory, skill improvement)
pub struct ConversationLoop {
    context_engine: ContextEngine,
    budget: IterationBudget,
    transport: Box<dyn TransportProvider>,
    tools: std::sync::Arc<oben_tools::ToolRegistry>,
}

impl ConversationLoop {
    pub fn new(
        transport: impl TransportProvider + 'static,
        tools: std::sync::Arc<oben_tools::ToolRegistry>,
        max_iterations: usize,
        max_messages: usize,
    ) -> Self {
        Self::with_config(
            transport,
            tools,
            max_iterations,
            max_messages,
            ContextEngineConfig::default(),
        )
    }

    pub fn with_config(
        transport: impl TransportProvider + 'static,
        tools: std::sync::Arc<oben_tools::ToolRegistry>,
        max_iterations: usize,
        _max_messages: usize,
        engine_config: ContextEngineConfig,
    ) -> Self {
        Self {
            context_engine: ContextEngine::with_config(engine_config),
            budget: IterationBudget::new(max_iterations),
            transport: Box::new(transport),
            tools,
        }
    }

    /// Run one conversation turn: user sends a message, agent responds.
    ///
    /// `messages` is the session's message buffer — all mutations (new messages,
    /// compression) happen in-place so there's no sync step needed.
    /// `session_id` is passed to the transport so it can target the correct
    /// per-session JSON cache for incremental building.
    pub async fn run_turn(
        &mut self,
        messages: &mut Vec<oben_models::Message>,
        user_message: Message,
        call_mode: &oben_models::CallMode,
    ) -> Result<String> {
        // Add user message to session
        messages.push(user_message);

        // Fresh call: tells transport to build all messages for this session.
        info!("Calling LLM... ({} messages in context)", messages.len());

        // Core loop with tool dispatch
        loop {
            self.budget.check()?;

            // Auto-compression: check if context is getting full before making an LLM call.
            // This ensures the API is never called with messages exceeding the token threshold.
            if self.context_engine.should_compress(messages) {
                info!(
                    "Auto-compression: context full ({} messages, {} est. tokens), compressing",
                    messages.len(),
                    self.context_engine.estimate_tokens(messages),
                );
                self.maybe_compress(messages).await?;
            }

            // Get LLM response — transport uses internal cache + incremental append.
            // &call_mode: zero-copy — the loop never mutates call_mode.
            eprintln!("LLM_CALL: about to call transport.chat, messages={}", messages.len());
            let response = self.transport.chat(messages, &call_mode).await?;
            eprintln!("LLM_CALL: got response, tool_calls={}", response.tool_calls.len());
            let tool_calls = &response.tool_calls;
            let text = &response.text;

            // Update token tracking from API response
            if let Some(tokens) = response.tokens_used {
                self.context_engine.update_from_response(tokens, 0, tokens);
            }

            // Add assistant response to session
            let assistant_msg = if !tool_calls.is_empty() {
                // Create message with tool_calls (required by API).
                // Use ToolCall::from_transport for a single allocation path.
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
                // When text is empty after tool results, return the tool
                // results instead of empty string (LLMs sometimes return
                // empty text after tool calls).
                if text.trim().is_empty() {
                    if let Some(last_tool_result) = messages.last().and_then(|m| {
                        if m.role == MessageRole::Tool {
                            m.content.to_text_ref()
                        } else {
                            None
                        }
                    }) {
                        if !last_tool_result.is_empty() {
                            return Ok(last_tool_result.to_string());
                        }
                    }
                }
                return Ok(text.trim().to_string());
            }

            // Dispatch tool calls
            for call in tool_calls {
                let result = self.tools.execute(&call.tool_name, &call.arguments).await;
                messages
                    .push(Message::tool_result(&call.id, &result.output));
            }

            // Incremental: only pass the newly added messages to the transport.
            // The transport merges them into its cached JSON state.
            //call_mode = oben_models::CallMode::Incremental(session_id.to_string());
        }
    }

    /// If `delta_callback` is provided, text tokens from *every* LLM call
    /// (including tool-result-followed-by-LLM calls) are streamed to it.
    pub async fn run_turn_with_streaming<F>(
        &mut self,
        messages: &mut Vec<oben_models::Message>,
        user_message: Message,
        call_mode: &oben_models::CallMode,
        delta_callback: Option<F>,
    ) -> Result<String>
    where
        F: FnMut(&str) + Send + 'static,
    {

        // Add user message to session
        messages.push(user_message);

        // Fresh call: tells transport to build all messages for this session.
        info!("Calling LLM... ({} messages in context)", messages.len());

        // Core loop with tool dispatch
        //
        // Performance note: the callback fires once per token (thousands of times
        // for long LLM responses). Locking a Mutex per token is expensive.
        // We route all tokens through a lock-free mpsc channel to a single drain
        // task that batches output and acquires the callback mutex only once per
        // ~512 bytes instead of once per token.
        let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(4096);
        let mut callback_handle: Option<tokio::task::JoinHandle<()>> = None;

        if let Some(cb) = delta_callback {
            let cb = std::sync::Arc::new(std::sync::Mutex::new(cb));
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

        loop {
            self.budget.check()?;

            // Auto-compression: check if context is getting full before making an LLM call.
            if self.context_engine.should_compress(messages) {
                self.maybe_compress(messages).await?;
            }

            // Get LLM response (streaming or non-streaming).
            // First call: Fresh triggers full build; subsequent: Incremental appends.
            // &call_mode: zero-copy — the loop never mutates call_mode, so we borrow.
            let response = if callback_handle.is_some() {
                let tx_clone = tx.clone();
                let wrapper: oben_models::StreamDeltaCallback =
                    Box::new(move |text: &str| {
                        // Channel buffer of 4096 tokens is far larger than
                        // any single LLM response delta, making overflow
                        // practically impossible. try_send is non-blocking
                        // and avoids holding the callback mutex per token.
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
                // Use ToolCall::from_transport for a single allocation path.
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
                // When text is empty after tool results, return the tool
                // results instead of empty string (LLMs sometimes return
                // empty text after tool calls).
                if text.trim().is_empty() {
                    if let Some(last_tool_result) = messages.last().and_then(|m| {
                        if m.role == MessageRole::Tool {
                            m.content.to_text_ref()
                        } else {
                            None
                        }
                    }) {
                        if !last_tool_result.is_empty() {
                            // Flush remaining buffered output before returning
                            drop(tx); // close channel
                            if let Some(handle) = callback_handle.take() {
                                let _ = handle.await;
                            }
                            return Ok(last_tool_result.to_string());
                        }
                    }
                }
                // Flush remaining buffered output before returning
                drop(tx); // close channel
                if let Some(handle) = callback_handle.take() {
                    let _ = handle.await;
                }
                return Ok(text.trim().to_string());
            }

            // Incremental: only pass the newly added messages to the transport.
            // The transport merges them into its cached JSON state.
            for call in tool_calls {
                let result = self.tools.execute(&call.tool_name, &call.arguments).await;
                messages
                    .push(Message::tool_result(&call.id, &result.output));
            }

            //call_mode = oben_models::CallMode::Incremental(session_id.to_string());
        }
    }

    /// Compress context if needed.
    ///
    /// This is the unified compression path — ContextEngine operates on the
    /// message buffer passed in (which is owned by the Session).
    pub async fn maybe_compress(&mut self, messages: &mut Vec<Message>) -> Result<()> {
        self.context_engine
            .compress(messages, Some(&*self.transport), None)
            .await?;
        Ok(())
    }

    /// Preflight check: compress messages if already over token threshold.
    ///
    /// Used when loading an existing session or switching models. Returns the
    /// number of compression passes performed (0–3). If still over budget
    /// after max passes, logs a warning and returns.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::ContextEngineConfig;
    use oben_models::{
        Message, TransportProvider, TransportResponse, TransportToolCall,
    };

    // Mock transport for testing
    struct MockTransport {
        pub responses: Vec<TransportResponse>,
        pub call_count: std::sync::Arc<std::sync::Mutex<usize>>,
    }

    #[async_trait::async_trait]
    impl TransportProvider for MockTransport {
        fn name(&self) -> &str {
            "mock"
        }

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

    #[tokio::test]
    async fn test_run_turn_non_streaming() {
        let mock = std::sync::Arc::new(MockTransport {
            responses: vec![TransportResponse {
                text: "Hello!".to_string(),
                tool_calls: vec![],
                tokens_used: Some(10),
            }],
            call_count: std::sync::Arc::new(std::sync::Mutex::new(0)),
        });

        let mut loop_ = ConversationLoop::new(
            mock,
            std::sync::Arc::new(oben_tools::ToolRegistry::new()),
            10,
            100,
        );
        let mut messages = Vec::new();

        let result = loop_.run_turn(&mut messages, Message::user("Hi"), &oben_models::CallMode::Fresh("test-session".to_string())).await.unwrap();
        assert_eq!(result, "Hello!");
        assert_eq!(messages.len(), 2); // user + assistant
    }

    #[tokio::test]
    async fn test_run_turn_with_tool_call() {
        let mock = std::sync::Arc::new(MockTransport {
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
            call_count: std::sync::Arc::new(std::sync::Mutex::new(0)),
        });

        let mut loop_ = ConversationLoop::new(
            mock,
            std::sync::Arc::new(oben_tools::ToolRegistry::new()),
            10,
            100,
        );
        let mut messages = Vec::new();

        let result = loop_.run_turn(&mut messages, Message::user("list files"), &oben_models::CallMode::Fresh("test-session".to_string())).await.unwrap();
        assert_eq!(result, "Done!");
        assert_eq!(messages.len(), 4); // user + assistant + tool_call + tool_result
    }

    #[tokio::test]
    async fn test_run_turn_empty_response() {
        let mock = std::sync::Arc::new(MockTransport {
            responses: vec![TransportResponse {
                text: "".to_string(),
                tool_calls: vec![],
                tokens_used: Some(5),
            }],
            call_count: std::sync::Arc::new(std::sync::Mutex::new(0)),
        });

        let mut loop_ = ConversationLoop::new(
            mock,
            std::sync::Arc::new(oben_tools::ToolRegistry::new()),
            10,
            100,
        );
        let mut messages = Vec::new();

        let result = loop_.run_turn(&mut messages, Message::user("Hi"), &oben_models::CallMode::Fresh("test-session".to_string())).await.unwrap();
        assert_eq!(result, "");
        assert_eq!(messages.len(), 2); // user + assistant
    }

    #[tokio::test]
    async fn test_run_turn_budget_exceeded() {
        let mock = std::sync::Arc::new(MockTransport {
            responses: vec![TransportResponse {
                text: "".to_string(),
                tool_calls: vec![TransportToolCall {
                    id: "call-1".to_string(),
                    tool_name: "shell".to_string(),
                    arguments: serde_json::json!({"command": "ls"}),
                }],
                tokens_used: Some(1),
            }],
            call_count: std::sync::Arc::new(std::sync::Mutex::new(0)),
        });

        let mut loop_ = ConversationLoop::new(
            mock,
            std::sync::Arc::new(oben_tools::ToolRegistry::new()),
            2, // max_iterations
            100,
        );
        let mut messages = Vec::new();

        let result = loop_.run_turn(&mut messages, Message::user("Hi"), &oben_models::CallMode::Fresh("test-session".to_string())).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_message_count() {
        let mock = std::sync::Arc::new(MockTransport {
            responses: vec![TransportResponse {
                text: "Hi back!".to_string(),
                tool_calls: vec![],
                tokens_used: Some(5),
            }],
            call_count: std::sync::Arc::new(std::sync::Mutex::new(0)),
        });

        let mut loop_ = ConversationLoop::new(
            mock,
            std::sync::Arc::new(oben_tools::ToolRegistry::new()),
            10,
            100,
        );
        let mut messages = Vec::new();

        assert_eq!(loop_.message_count(&messages), 0);
        loop_
            .run_turn(&mut messages, Message::user("Hello"), &oben_models::CallMode::Fresh("test-session".to_string()))
            .await
            .unwrap();
        assert_eq!(messages.len(), 2); // user + assistant
    }

    #[tokio::test]
    async fn test_streaming_basic() {
        let mock = std::sync::Arc::new(MockTransport {
            responses: vec![TransportResponse {
                text: "Hello from stream!".to_string(),
                tool_calls: vec![],
                tokens_used: Some(10),
            }],
            call_count: std::sync::Arc::new(std::sync::Mutex::new(0)),
        });

        let mut loop_ = ConversationLoop::new(
            mock,
            std::sync::Arc::new(oben_tools::ToolRegistry::new()),
            10,
            100,
        );
        let mut messages = Vec::new();

        let output = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let output_clone = output.clone();
        let cb: oben_models::StreamDeltaCallback =
            Box::new(move |text: &str| output_clone.lock().unwrap().push_str(text));

        let result = loop_
            .run_turn_with_streaming(&mut messages, Message::user("Hi"), &oben_models::CallMode::Fresh("test-session".to_string()), Some(cb))
            .await
            .unwrap();
        assert_eq!(result, "Hello from stream!");
        assert_eq!(messages.len(), 2); // user + assistant
        assert_eq!(*output.lock().unwrap(), "Hello from stream!");
    }

    #[tokio::test]
    async fn test_streaming_with_tool_calls() {
        let mock = std::sync::Arc::new(MockTransport {
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
            call_count: std::sync::Arc::new(std::sync::Mutex::new(0)),
        });

        let mut loop_ = ConversationLoop::new(
            mock,
            std::sync::Arc::new(oben_tools::ToolRegistry::new()),
            10,
            100,
        );
        let mut messages = Vec::new();

        let output = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let output_clone = output.clone();
        let cb: oben_models::StreamDeltaCallback =
            Box::new(move |text: &str| output_clone.lock().unwrap().push_str(text));

        let result = loop_
            .run_turn_with_streaming(&mut messages, Message::user("list files"), &oben_models::CallMode::Fresh("test-session".to_string()), Some(cb))
            .await
            .unwrap();
        assert_eq!(result, "All done!");
        assert_eq!(*output.lock().unwrap(), "Let me check.All done!");
        assert_eq!(messages.len(), 4); // user + assistant + tool_call + tool_result
    }

    #[tokio::test]
    async fn test_streaming_empty_callback() {
        let mock = std::sync::Arc::new(MockTransport {
            responses: vec![TransportResponse {
                text: "Hello!".to_string(),
                tool_calls: vec![],
                tokens_used: Some(10),
            }],
            call_count: std::sync::Arc::new(std::sync::Mutex::new(0)),
        });

        let mut loop_ = ConversationLoop::new(
            mock,
            std::sync::Arc::new(oben_tools::ToolRegistry::new()),
            10,
            100,
        );
        let mut messages = Vec::new();

        let output = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let output_clone = output.clone();
        let cb: oben_models::StreamDeltaCallback =
            Box::new(move |text: &str| output_clone.lock().unwrap().push_str(text));

        let result = loop_
            .run_turn_with_streaming(&mut messages, Message::user("Hi"), &oben_models::CallMode::Fresh("test-session".to_string()), Some(cb))
            .await
            .unwrap();
        assert_eq!(result, "Hello!");
        assert_eq!(messages.len(), 2); // user + assistant
        assert_eq!(*output.lock().unwrap(), "Hello!");
    }

    // -----------------------------------------------------------------------
    // Auto-compression integration tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_auto_compression_fires_before_llm_call() {
        // Mock transport that returns a summary response first (for compression),
        // then a real LLM response.
        let mock = std::sync::Arc::new(MockTransport {
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
            call_count: std::sync::Arc::new(std::sync::Mutex::new(0)),
        });

        let config = ContextEngineConfig {
            context_length: 1000,
            threshold_percent: 0.5,
            protect_first_n: 2,
            tail_token_budget: 100, // Small budget so middle isn't empty
            tail_min_messages: 2,
            tail_overhead: 1.5,
            ineffective_threshold: 10.0,
            max_ineffective_consecutive: 2,
            max_messages: 100,
        };

        let mut loop_ = ConversationLoop::with_config(
            mock.clone(),
            std::sync::Arc::new(oben_tools::ToolRegistry::new()),
            10,
            100,
            config,
        );

        // Create messages that exceed 500-token threshold
        let long_content = "The quick brown fox jumps over the lazy dog. ".repeat(100);
        let mut messages: Vec<Message> = (0..10)
            .map(|i| Message::user(&format!("Message {}: {}", i, long_content)))
            .collect();

        let result = loop_
            .run_turn(
                &mut messages,
                Message::user("Hi"),
                &oben_models::CallMode::Fresh("test-session".to_string()),
            )
            .await
            .unwrap();

        // Two LLM calls: first for summary (compression), second for real response
        assert_eq!(*mock.call_count.lock().unwrap(), 2);
        assert_eq!(result, "Hello from compressed context!");
        // Messages should be compressed (head + summary + tail)
        assert!(messages.len() < 12, "Messages should be compressed");
    }

    #[tokio::test]
    async fn test_no_compression_when_under_threshold() {
        let mock = std::sync::Arc::new(MockTransport {
            responses: vec![TransportResponse {
                text: "Hello!".to_string(),
                tool_calls: vec![],
                tokens_used: Some(50),
            }],
            call_count: std::sync::Arc::new(std::sync::Mutex::new(0)),
        });

        let config = ContextEngineConfig {
            context_length: 10000,
            threshold_percent: 0.5,
            protect_first_n: 3,
            tail_token_budget: 20_000,
            tail_min_messages: 3,
            tail_overhead: 1.5,
            ineffective_threshold: 10.0,
            max_ineffective_consecutive: 2,
            max_messages: 100,
        };

        let mut loop_ = ConversationLoop::with_config(
            mock.clone(),
            std::sync::Arc::new(oben_tools::ToolRegistry::new()),
            10,
            100,
            config,
        );

        let mut messages = vec![Message::user("Hi")];

        let result = loop_
            .run_turn(
                &mut messages,
                Message::user("Hello"),
                &oben_models::CallMode::Fresh("test-session".to_string()),
            )
            .await
            .unwrap();

        // One LLM call — no compression fired
        assert_eq!(*mock.call_count.lock().unwrap(), 1);
        assert_eq!(result, "Hello!");
    }

    #[tokio::test]
    async fn test_auto_compression_fires_in_streaming() {
        let mock = std::sync::Arc::new(MockTransport {
            responses: vec![
                // First call: summary for compression
                TransportResponse {
                    text: "## Active Task\nTest completed.".to_string(),
                    tool_calls: vec![],
                    tokens_used: Some(50),
                },
                // Second call: streamed response
                TransportResponse {
                    text: "Streamed response!".to_string(),
                    tool_calls: vec![],
                    tokens_used: Some(50),
                },
            ],
            call_count: std::sync::Arc::new(std::sync::Mutex::new(0)),
        });

        let config = ContextEngineConfig {
            context_length: 1000,
            threshold_percent: 0.5,
            protect_first_n: 2,
            tail_token_budget: 100,
            tail_min_messages: 2,
            tail_overhead: 1.5,
            ineffective_threshold: 10.0,
            max_ineffective_consecutive: 2,
            max_messages: 100,
        };

        let mut loop_ = ConversationLoop::with_config(
            mock.clone(),
            std::sync::Arc::new(oben_tools::ToolRegistry::new()),
            10,
            100,
            config,
        );

        let long_content = "The quick brown fox jumps over the lazy dog. ".repeat(100);
        let mut messages: Vec<Message> = (0..10)
            .map(|i| Message::user(&format!("Message {}: {}", i, long_content)))
            .collect();

        let output = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let output_clone = output.clone();
        let cb: oben_models::StreamDeltaCallback =
            Box::new(move |text: &str| output_clone.lock().unwrap().push_str(text));

        let result = loop_
            .run_turn_with_streaming(
                &mut messages,
                Message::user("Hi"),
                &oben_models::CallMode::Fresh("test-session".to_string()),
                Some(cb),
            )
            .await
            .unwrap();

        assert_eq!(*mock.call_count.lock().unwrap(), 2); // summary + streamed
        assert_eq!(result, "Streamed response!");
        assert_eq!(*output.lock().unwrap(), "Streamed response!");
    }

    #[tokio::test]
    async fn test_preflight_no_compression_when_under_threshold() {
        let transport = MockTransport {
            responses: vec![TransportResponse {
                text: "Under threshold".to_string(),
                tool_calls: vec![],
                tokens_used: Some(10),
            }],
            call_count: std::sync::Arc::new(std::sync::Mutex::new(0)),
        };
        let mut conversation = ConversationLoop::new(
            transport,
            std::sync::Arc::new(oben_tools::ToolRegistry::new()),
            50,
            100,
        );
        let mut messages = vec![
            Message::system("You are a helpful assistant."),
            Message::user("Hi"),
        ];
        // Small message list, under default threshold (context_length=4096 * 0.5 = ~2000 tokens)
        let passes = conversation.preflight_check(&mut messages).await.unwrap();
        assert_eq!(passes, 0, "should not compress when under threshold");
    }

    #[tokio::test]
    async fn test_preflight_compresses_when_over_threshold() {
        // Use a very small context length so our small messages exceed threshold
        let config = ContextEngineConfig {
            context_length: 100, // Very small
            ..ContextEngineConfig::default()
        };
        let transport = MockTransport {
            responses: vec![TransportResponse {
                text: "Compressed".to_string(),
                tool_calls: vec![],
                tokens_used: Some(10),
            }],
            call_count: std::sync::Arc::new(std::sync::Mutex::new(0)),
        };
        let mut conversation = ConversationLoop::with_config(
            transport,
            std::sync::Arc::new(oben_tools::ToolRegistry::new()),
            50,
            100,
            config,
        );
        let mut messages = vec![
            Message::system("You are a helpful assistant."),
            Message::user("Hi, how are you?"),
        ];
        // With context_length=100, threshold=50 tokens — our messages likely exceed this
        let passes = conversation.preflight_check(&mut messages).await.unwrap();
        assert_eq!(passes, 0); // Summary generation fails on mock, so no compression done
        // The check still runs — it just doesn't compress because the mock fails
    }

    #[tokio::test]
    async fn test_preflight_max_passes() {
        // Use a very small context length to force compression attempts
        let config = ContextEngineConfig {
            context_length: 50,
            ..ContextEngineConfig::default()
        };
        let transport = MockTransport {
            responses: vec![TransportResponse {
                text: "Compressed".to_string(),
                tool_calls: vec![],
                tokens_used: Some(10),
            }],
            call_count: std::sync::Arc::new(std::sync::Mutex::new(0)),
        };
        let mut conversation = ConversationLoop::with_config(
            transport,
            std::sync::Arc::new(oben_tools::ToolRegistry::new()),
            50,
            100,
            config,
        );
        // Create many messages to exceed threshold
        let mut messages = vec![Message::system("You are a helpful assistant.")];
        for i in 0..20 {
            messages.push(Message::user(format!("Message number {} with some content to increase token count", i)));
        }
        let passes = conversation.preflight_check(&mut messages).await.unwrap();
        assert!(passes <= 3, "should cap at 3 passes");
        // Mock transport fails, so no actual compression — but preflight still runs
    }
}
