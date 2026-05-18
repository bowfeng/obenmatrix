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
                tool_calls.iter().map(|tc| format!("[Calling {}]", tc.tool_name)).collect::<Vec<_>>().join(", ")
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
                self.context_manager.add_message(Message::tool_result(&call.id, &result.output));
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
            self.context_manager.add_message(Message::system(compressed));
        }
        Ok(())
    }

    pub fn message_count(&self) -> usize {
        self.context_manager.len()
    }
}
