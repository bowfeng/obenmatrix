/// Conversation loop — the main agent turn cycle.
/// Maps to `agent/conversation_loop.py::run_conversation`.

use anyhow::Result;
use oben_models::{Message, TransportProvider};
use tracing::info;

use crate::{
    budget::IterationBudget,
    context::{ContextEngine, ContextEngineConfig},
};

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
        session_id: &str,
    ) -> Result<String> {
        // Add user message to session
        messages.push(user_message);

        // Fresh call: tells transport to build all messages for this session.
        info!("Calling LLM... ({} messages in context)", messages.len());


        let mut call_mode = oben_models::CallMode::Fresh(session_id.to_string());

        // Core loop with tool dispatch
        loop {
            self.budget.check()?;

            // Get LLM response — transport uses internal cache + incremental append.
            let response = self.transport.chat(messages, call_mode).await?;
            let tool_calls = &response.tool_calls;
            let text = &response.text;

            // Update token tracking from API response
            if let Some(tokens) = response.tokens_used {
                self.context_engine.update_from_response(tokens, 0, tokens);
            }

            // Add assistant response to session
            let assistant_text = if !tool_calls.is_empty() {
                tool_calls
                    .iter()
                    .map(|tc| format!("[Calling {}]", tc.tool_name))
                    .collect::<Vec<_>>()
                    .join(", ")
            } else {
                text.clone()
            };
            messages.push(Message::assistant(assistant_text));

            if tool_calls.is_empty() {
                return Ok(text.clone());
            }

            // Dispatch tool calls
            for call in tool_calls {
                let result = self.tools.execute(&call.tool_name, &call.arguments).await;
                messages
                    .push(Message::tool_result(&call.id, &result.output));
            }

            // Incremental: only pass the newly added messages to the transport.
            // The transport merges them into its cached JSON state.
            call_mode = oben_models::CallMode::Incremental(session_id.to_string());
        }



    }

    ///
    /// If `delta_callback` is provided, text tokens from *every* LLM call
    /// (including tool-result-followed-by-LLM calls) are streamed to it.
    pub async fn run_turn_with_streaming<F>(
        &mut self,
        messages: &mut Vec<oben_models::Message>,
        user_message: Message,
        session_id: &str,
        delta_callback: Option<F>,
    ) -> Result<String>
    where
        F: FnMut(&str) + Send + 'static,
    {
        // Add user message to session
        messages.push(user_message);

        // Fresh call: tells transport to build all messages for this session.
        info!("Calling LLM... ({} messages in context)", messages.len());


        let mut call_mode = oben_models::CallMode::Fresh(session_id.to_string());

        // Core loop with tool dispatch
        let shared: Option<std::sync::Arc<std::sync::Mutex<F>>> =
            delta_callback.map(|cb| std::sync::Arc::new(std::sync::Mutex::new(cb)));

        loop {
            self.budget.check()?;

            // Get LLM response (streaming or non-streaming).
            // First call: Fresh triggers full build; subsequent: Incremental appends.
            let response = if let Some(ref shared) = shared {
                let shared_clone = shared.clone();
                let wrapper: oben_models::StreamDeltaCallback =
                    Box::new(move |text: &str| {
                        shared_clone.lock().unwrap()(text);
                    });
                self.transport.stream_chat(messages, call_mode, wrapper).await?
            } else {
                self.transport.chat(messages, call_mode).await?
            };
            let tool_calls = &response.tool_calls;
            let text = &response.text;

            // Update token tracking from API response
            if let Some(tokens) = response.tokens_used {
                self.context_engine.update_from_response(tokens, 0, tokens);
            }

            // Add assistant response to session
            let assistant_text = if !tool_calls.is_empty() {
                tool_calls
                    .iter()
                    .map(|tc| format!("[Calling {}]", tc.tool_name))
                    .collect::<Vec<_>>()
                    .join(", ")
            } else {
                text.clone()
            };
            messages.push(Message::assistant(assistant_text));

            if tool_calls.is_empty() {
                return Ok(text.clone());
            }

            // Incremental: only pass the newly added messages to the transport.
            // The transport merges them into its cached JSON state.
            for call in tool_calls {
                let result = self.tools.execute(&call.tool_name, &call.arguments).await;
                messages
                    .push(Message::tool_result(&call.id, &result.output));
            }

            call_mode = oben_models::CallMode::Incremental(session_id.to_string());
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

    pub fn message_count(&self, messages: &[Message]) -> usize {
        messages.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

        async fn chat(&self, _messages: &[Message], _mode: oben_models::CallMode) -> Result<TransportResponse> {
            let mut count = self.call_count.lock().unwrap();
            *count += 1;
            let idx = (*count - 1).min(self.responses.len() - 1);
            Ok(self.responses[idx].clone())
        }

        async fn stream_chat(
            &self,
            _messages: &[Message],
            _mode: oben_models::CallMode,
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

        let result = loop_.run_turn(&mut messages, Message::user("Hi"), "test-session").await.unwrap();
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

        let result = loop_.run_turn(&mut messages, Message::user("list files"), "test-session").await.unwrap();
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

        let result = loop_.run_turn(&mut messages, Message::user("Hi"), "test-session").await.unwrap();
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

        let result = loop_.run_turn(&mut messages, Message::user("Hi"), "test-session").await;
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
            .run_turn(&mut messages, Message::user("Hello"), "test-session")
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
            .run_turn_with_streaming(&mut messages, Message::user("Hi"), "test-session", Some(cb))
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
            .run_turn_with_streaming(&mut messages, Message::user("list files"), "test-session", Some(cb))
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
            .run_turn_with_streaming(&mut messages, Message::user("Hi"), "test-session", Some(cb))
            .await
            .unwrap();
        assert_eq!(result, "Hello!");
        assert_eq!(messages.len(), 2); // user + assistant
        assert_eq!(*output.lock().unwrap(), "Hello!");
    }
}
