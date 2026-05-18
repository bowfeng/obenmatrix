/// Conversation loop — the main agent turn cycle.
/// Maps to `agent/conversation_loop.py::run_conversation`.

use anyhow::Result;
use oben_models::{Message, TransportProvider};
use tracing::info;

use crate::{
    budget::IterationBudget,
    compression::ContextCompressor,
    context::ContextManager,
    prompt::PromptBuilder,
};

/// The main agent loop — one user turn.
///
/// 1. User message arrives
/// 2. Build system prompt + context
/// 3. Call LLM
/// 4. If tool calls: dispatch → collect results → loop
/// 5. If text response: return to user
/// 6. Post-turn hooks (memory, skill improvement)
pub struct ConversationLoop {
    prompt_builder: PromptBuilder,
    context_manager: ContextManager,
    compressor: ContextCompressor,
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
        Self {
            prompt_builder: PromptBuilder::new(),
            context_manager: ContextManager::new(max_messages),
            compressor: ContextCompressor::new(),
            budget: IterationBudget::new(max_iterations),
            transport: Box::new(transport),
            tools,
        }
    }

    /// Run one conversation turn: user sends a message, agent responds.
    pub async fn run_turn(&mut self, user_message: Message) -> Result<String> {
        // Add user message to context
        self.context_manager.add_message(user_message);

        // Build messages for API
        let mut messages = self.prompt_builder.build_api_messages(&self.context_manager)?;

        info!("Calling LLM... ({} messages in context)", messages.len());

        let mut final_text = String::new();

        // Core loop with tool dispatch
        loop {
            self.budget.check()?;

            // Get LLM response
            let response = self.transport.chat(&messages).await?;
            let tool_calls = &response.tool_calls;
            let text = &response.text;
            final_text = text.clone();

            // Add assistant response to context
            let assistant_text = if !tool_calls.is_empty() {
                tool_calls
                    .iter()
                    .map(|tc| format!("[Calling {}]", tc.tool_name))
                    .collect::<Vec<_>>()
                    .join(", ")
            } else {
                text.clone()
            };
            self.context_manager.add_message(Message::assistant(assistant_text));

            if tool_calls.is_empty() {
                break;
            }

            // Dispatch tool calls
            for call in tool_calls {
                let result = self.tools.execute(&call.tool_name, &call.arguments).await;
                self.context_manager
                    .add_message(Message::tool_result(&call.id, &result.output));
            }

            // Rebuild messages with tool results
            messages.clear();
            messages.extend(self.prompt_builder.build_api_messages(&self.context_manager)?);
        }

        Ok(final_text)
    }

    /// Run one conversation turn with optional streaming.
    ///
    /// If `delta_callback` is provided, text tokens from *every* LLM call
    /// (including tool-result-followed-by-LLM calls) are streamed to it.
    pub async fn run_turn_with_streaming<F>(
        &mut self,
        user_message: Message,
        delta_callback: Option<F>,
    ) -> Result<String>
    where
        F: FnMut(&str) + Send + 'static,
    {
        // Add user message to context
        self.context_manager.add_message(user_message);

        // Build messages for API
        let mut messages = self.prompt_builder.build_api_messages(&self.context_manager)?;

        info!("Calling LLM... ({} messages in context)", messages.len());

        let mut final_text = String::new();

        // Core loop with tool dispatch
        // If streaming is enabled, wrap the callback in Arc<Mutex> so it can be
        // shared across multiple stream_chat calls within this conversation turn.
        let shared: Option<std::sync::Arc<std::sync::Mutex<F>>> =
            delta_callback.map(|cb| std::sync::Arc::new(std::sync::Mutex::new(cb)));

        loop {
            self.budget.check()?;

            // Get LLM response (streaming or non-streaming)
            let response = if let Some(ref shared) = shared {
                let shared_clone = shared.clone();
                let wrapper: oben_models::StreamDeltaCallback =
                    Box::new(move |text: &str| {
                        shared_clone.lock().unwrap()(text);
                    });
                self.transport.stream_chat(&messages, wrapper).await?
            } else {
                self.transport.chat(&messages).await?
            };
            let tool_calls = &response.tool_calls;
            let text = &response.text;
            final_text = text.clone();

            // Add assistant response to context
            let assistant_text = if !tool_calls.is_empty() {
                tool_calls
                    .iter()
                    .map(|tc| format!("[Calling {}]", tc.tool_name))
                    .collect::<Vec<_>>()
                    .join(", ")
            } else {
                text.clone()
            };
            self.context_manager.add_message(Message::assistant(assistant_text));

            if tool_calls.is_empty() {
                break;
            }

            // Dispatch tool calls
            for call in tool_calls {
                let result = self.tools.execute(&call.tool_name, &call.arguments).await;
                self.context_manager
                    .add_message(Message::tool_result(&call.id, &result.output));
            }

            // Rebuild messages with tool results
            messages.clear();
            messages.extend(self.prompt_builder.build_api_messages(&self.context_manager)?);
        }

        Ok(final_text)
    }

    /// Compress context if needed.
    pub fn maybe_compress(&mut self) -> Result<()> {
        if self.context_manager.needs_compression() {
            let compressed = self.compressor.summarize_context(&self.context_manager)?;
            self.context_manager.clear_messages();
            self.context_manager
                .add_message(Message::system(compressed));
        }
        Ok(())
    }

    pub fn message_count(&self) -> usize {
        self.context_manager.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oben_models::{
        Message, MessageContent, MessageRole, TransportProvider, TransportResponse,
        TransportToolCall,
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

        async fn chat(&self, _messages: &[Message]) -> Result<TransportResponse> {
            let mut count = self.call_count.lock().unwrap();
            *count += 1;
            let idx = (*count - 1).min(self.responses.len() - 1);
            Ok(self.responses[idx].clone())
        }

        async fn stream_chat(
            &self,
            _messages: &[Message],
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

        let result = loop_.run_turn(Message::user("Hi")).await.unwrap();
        assert_eq!(result, "Hello!");
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

        let result = loop_.run_turn(Message::user("list files")).await.unwrap();
        assert_eq!(result, "Done!");
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

        let result = loop_.run_turn(Message::user("Hi")).await.unwrap();
        assert_eq!(result, "");
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

        let result = loop_.run_turn(Message::user("Hi")).await;
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

        assert_eq!(loop_.message_count(), 0);
        loop_
            .run_turn(Message::user("Hello"))
            .await
            .unwrap();
        assert_eq!(loop_.message_count(), 2); // user + assistant
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

        let output = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let output_clone = output.clone();
        let cb: oben_models::StreamDeltaCallback =
            Box::new(move |text: &str| output_clone.lock().unwrap().push_str(text));

        let result = loop_
            .run_turn_with_streaming(Message::user("Hi"), Some(cb))
            .await
            .unwrap();
        assert_eq!(result, "Hello from stream!");
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

        let output = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let output_clone = output.clone();
        let cb: oben_models::StreamDeltaCallback =
            Box::new(move |text: &str| output_clone.lock().unwrap().push_str(text));

        let result = loop_
            .run_turn_with_streaming(Message::user("list files"), Some(cb))
            .await
            .unwrap();
        assert_eq!(result, "All done!");
        assert_eq!(*output.lock().unwrap(), "Let me check.All done!");
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

        let output = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let output_clone = output.clone();
        let cb: oben_models::StreamDeltaCallback =
            Box::new(move |text: &str| output_clone.lock().unwrap().push_str(text));

        let result = loop_
            .run_turn_with_streaming(Message::user("Hi"), Some(cb))
            .await
            .unwrap();
        assert_eq!(result, "Hello!");
        assert_eq!(*output.lock().unwrap(), "Hello!");
    }
}
