/// Conversation loop — coordinator that wires the deep `TurnExecutor`.
///
/// The `ConversationLoop` is a thin coordinator layer. The actual turn logic
/// lives in `TurnExecutor` (deep module), and this layer provides:
/// - Session lifecycle hooks
/// - Compression summary tracking
/// - The two public entry points (`run_turn` / `run_turn_with_streaming`)

use std::path::PathBuf;
use anyhow::Result;

use crate::compression::CompressionConfig;
use crate::turn_executor::TurnExecutor;
use crate::system_prompt;
use oben_models::Message;

/// Configuration for building the 3-tier system prompt.
pub struct SystemPromptConfig {
    identity: String,
    tools_list: Vec<String>,
    skills_dirs: Vec<PathBuf>,
    context_cwd: Option<PathBuf>,
    custom_message: Option<String>,
    memory_context: Option<String>,
}

impl SystemPromptConfig {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        identity: String,
        tools_list: Vec<String>,
        skills_dirs: Vec<PathBuf>,
        context_cwd: Option<PathBuf>,
        custom_message: Option<String>,
        memory_context: Option<String>,
    ) -> Self {
        Self { identity, tools_list, skills_dirs, context_cwd, custom_message, memory_context }
    }

    pub fn build_system_prompt(&self, session_id: &str, model_name: &str) -> String {
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

/// The main agent loop — a thin coordinator that wires the deep `TurnExecutor`.
pub struct ConversationLoop {
    executor: TurnExecutor,
    last_compression_summary: Option<String>,
}

impl ConversationLoop {
    pub fn new(
        transport: impl oben_models::TransportProvider + 'static,
        tools: std::sync::Arc<oben_tools::ToolRegistry>,
        max_iterations: usize,
        max_messages: usize,
    ) -> Self {
        Self::with_config(
            transport,
            tools,
            max_iterations,
            max_messages,
            CompressionConfig::default(),
        )
    }

    pub fn with_config(
        transport: impl oben_models::TransportProvider + 'static,
        tools: std::sync::Arc<oben_tools::ToolRegistry>,
        max_iterations: usize,
        _max_messages: usize,
        engine_config: CompressionConfig,
    ) -> Self {
        Self {
            executor: TurnExecutor::with_config(transport, tools, max_iterations, _max_messages, engine_config),
            last_compression_summary: None,
        }
    }

    /// Run one conversation turn — delegates to `TurnExecutor::execute_turn(None)`.
    pub async fn run_turn(
        &mut self,
        messages: &mut Vec<oben_models::Message>,
        user_message: Message,
        call_mode: &oben_models::CallMode,
    ) -> Result<String> {
        let result = self.executor.execute_turn(messages, user_message, call_mode, None).await?;
        Ok(result.text)
    }

    /// Run one conversation turn with streaming — delegates to `execute_turn(Some(cb))`.
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
        let cb: oben_models::StreamDeltaCallback = Box::new(delta_callback.unwrap());
        let result = self.executor.execute_turn(messages, user_message, call_mode, Some(cb)).await?;
        Ok(result.text)
    }

    /// Compress context if needed — coordinator concern.
    pub async fn maybe_compress(&mut self, messages: &mut Vec<Message>) -> Result<()> {
        self.executor.maybe_compress(messages).await?;
        Ok(())
    }

    /// Preflight check — coordinator concern.
    pub async fn preflight_check(
        &mut self,
        messages: &mut Vec<Message>,
    ) -> Result<usize> {
        self.executor.preflight_check(messages).await
    }

    pub fn on_session_start(
        &mut self,
        session_id: &str,
        model_name: &str,
        context_length: Option<usize>,
    ) {
        self.executor.on_session_start(session_id, model_name, context_length);
    }

    pub fn on_session_reset(&mut self) {
        self.executor.on_session_reset();
    }

    pub fn on_session_end(&mut self, session_id: &str) {
        self.executor.on_session_end(session_id);
    }

    pub fn message_count(&self, messages: &[Message]) -> usize {
        self.executor.message_count(messages)
    }
}
