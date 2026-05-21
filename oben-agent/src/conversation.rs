/// Conversation loop — coordinator that wires the deep `TurnExecutor`.
///
/// The `ConversationLoop` is a thin coordinator layer. The actual turn logic
/// lives in `TurnExecutor` (deep module), and this layer provides:
/// - Session lifecycle hooks
/// - Compression summary tracking
/// - The two public entry points (`run_turn` / `run_turn_with_streaming`)

use std::path::PathBuf;
use anyhow::Result;

use std::sync::{Arc, Mutex};

use crate::context::ContextEngine;
use crate::turn_executor::TurnExecutor;
use crate::system_prompt;
use oben_models::{Message, SessionStore};

/// Type alias for the default conversation loop.
pub type DefaultConversationLoop = ConversationLoop;

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
}

impl ConversationLoop {
    pub fn new(
        transport: impl oben_models::TransportProvider + 'static,
        tools: std::sync::Arc<oben_tools::ToolRegistry>,
        max_iterations: usize,
        max_messages: usize,
        context_engine: Arc<Mutex<dyn ContextEngine>>,
    ) -> Self {
        Self {
            executor: TurnExecutor::with_config(
                transport,
                tools,
                max_iterations,
                max_messages,
                context_engine,
            ),
        }
    }

    /// Run one conversation turn — delegates to `TurnExecutor::execute_turn(None)`.
    pub async fn run_turn(
        &mut self,
        store: &mut dyn SessionStore,
        session_id: &str,
        user_message: Message,
        call_mode: &oben_models::CallMode,
    ) -> Result<String> {
        let result = self.executor.execute_turn(store, session_id, user_message, call_mode, None).await?;
        Ok(result.text)
    }

    /// Run one conversation turn with streaming — delegates to `execute_turn(Some(cb))`.
    pub async fn run_turn_with_streaming<F>(
        &mut self,
        store: &mut dyn SessionStore,
        session_id: &str,
        user_message: Message,
        call_mode: &oben_models::CallMode,
        delta_callback: Option<F>,
    ) -> Result<String>
    where
        F: FnMut(&str) + Send + 'static,
    {
        let cb: oben_models::StreamDeltaCallback = Box::new(delta_callback.unwrap());
        let result = self.executor.execute_turn(store, session_id, user_message, call_mode, Some(cb)).await?;
        Ok(result.text)
    }

    /// Compress context if needed — coordinator concern.
    pub async fn maybe_compress(&mut self, store: &mut dyn SessionStore, session_id: &str) -> Result<()> {
        let session = store.session_mut(session_id)
            .ok_or_else(|| anyhow::anyhow!("Session not found: {}", session_id))?;
        self.executor.maybe_compress(session).await?;
        Ok(())
    }

    /// Preflight check — coordinator concern.
    pub async fn preflight_check(
        &mut self,
        store: &mut dyn SessionStore,
        session_id: &str,
    ) -> Result<usize> {
        let session = store.session_mut(session_id)
            .ok_or_else(|| anyhow::anyhow!("Session not found: {}", session_id))?;
        self.executor.preflight_check(session).await
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

    pub fn message_count(&self, store: &dyn SessionStore, session_id: &str) -> usize {
        store
            .session(session_id)
            .map(|s| s.messages.len())
            .unwrap_or(0)
    }
}
