/// CLI coordinator — replaces the legacy `Agent::interactive_chat` loop with a structured
/// coordinator that owns hooks, termination policy, and the loop itself.
use std::sync::Arc;
use oben_agent::coordinator::tree::InterruptHub;

use anyhow::{anyhow, Result};

use oben_agent::context::ContextEngine;
use oben_agent::hooks::{HookBuilder, HookEngine};
use oben_agent::interaction::InteractionProvider;
use oben_agent::{ConversationConfig, ConversationCoordinator, ConversationResult};

use oben_config::AppConfig;
use oben_models::{CallMode, Message, SessionManager, TransportProvider};
use oben_tools::ToolRegistry;

/// Configuration for CLI conversation behavior.
pub struct CliConfig {
    /// Shared conversation configuration (retry, fallback, dispatch, etc.).
    pub conversation: ConversationConfig,
    /// Hook engine shared with Agent (Arc to avoid duplicate instantiation).
    pub hooks: Arc<HookEngine>,
    /// Whether to stream output.
    pub stream: bool,
}

impl CliConfig {
    pub fn with_stream(mut self, stream: bool) -> Self {
        self.stream = stream;
        self
    }
}

/// CLI coordinator — the full interactive chat loop for terminal-based interaction.
pub struct CliCoordinator {
    config: CliConfig,
    call_mode: Option<CallMode>,
    interrupt_hub: Arc<InterruptHub>,
}

impl CliCoordinator {
    /// Create a new CLI coordinator from conversation config and shared hooks.
    pub fn from_conversation(conversation: ConversationConfig, hooks: Arc<HookEngine>) -> Self {
        let config = CliConfig {
            conversation,
            hooks,
            stream: false,
        };
        let max_spawn_depth = config.conversation.max_spawn_depth;
        Self {
            config,
            call_mode: None,
            interrupt_hub: Arc::new(InterruptHub::new(max_spawn_depth)),
        }
    }

    /// Create a new CLI coordinator with explicit config.
    pub fn from_config(config: CliConfig) -> Self {
        let max_spawn_depth = config.conversation.max_spawn_depth;
        Self {
            config,
            call_mode: None,
            interrupt_hub: Arc::new(InterruptHub::new(max_spawn_depth)),
        }
    }
}

#[async_trait::async_trait]
impl ConversationCoordinator for CliCoordinator {
    async fn run(
        &mut self,
        context_engine: &mut dyn ContextEngine,
        transport: Arc<dyn TransportProvider + Send + Sync>,
        tools: Arc<ToolRegistry>,
        session_manager: &mut dyn SessionManager,
    ) -> Result<ConversationResult> {
        let interaction = oben_agent::interaction::StdioProvider::new();
        let mut is_resumed_session = session_manager.active_session().is_some();

        // Fire loop-start hooks.
        self.config.hooks.emit_loop_start();

        loop {
            self.config.hooks.emit_pre_turn();

            interaction.write_raw(b"> ");
            interaction.flush();

            let input = match interaction.read_input().await {
                Some(line) if line.trim().is_empty() => {
                    self.config.hooks.emit_loop_end("no_input");
                    return Err(anyhow!("No more input available"));
                }
                Some(line) => line.trim().to_string(),
                None => {
                    self.config.hooks.emit_loop_end("stdin_closed");
                    return Err(anyhow!("stdin closed"));
                }
            };

            if interaction.should_exit(&input) {
                self.config.hooks.emit_loop_end("quit");
                return Ok(ConversationResult::Exit);
            }

            let sid = session_manager.active_session()
                .map(|s| s.id.clone())
                .unwrap_or_else(|| {
                    let id = oben_agent::agent::generate_session_name();
                    let _ = session_manager.new_session(&id);
                    session_manager.active_session().unwrap().id.clone()
                });

            let call_mode_val = match &self.call_mode {
                Some(CallMode::Fresh(_)) => {
                    self.call_mode = Some(CallMode::Incremental(sid.clone()));
                    CallMode::Fresh(sid.clone())
                }
                Some(CallMode::Incremental(_)) => {
                    self.call_mode.as_ref().unwrap().clone()
                }
                None => {
                    let mode = CallMode::Fresh(sid.clone());
                    self.call_mode = Some(mode.clone());
                    mode
                }
            };

            let input_msg = Message::user(&input);

            let response = oben_agent::coordinator::execute_turn(
                context_engine,
                &*transport,
                &tools,
                session_manager,
                &sid,
                input_msg,
                &call_mode_val,
                &self.config.conversation,
            ).await;

            let response_text = if let Ok(resp) = response {
                Some(resp)
            } else {
                let e = response.unwrap_err();
                let err_str = format!("Turn error: {}", e);
                drop(e);
                let _ = session_manager.incremental_save(None);
                self.config.hooks.emit_loop_end(&err_str);
                return Err(anyhow!(err_str));
            };
            let _ = session_manager.incremental_save(None);

            if let Some(ref resp) = response_text {
                if self.config.stream {
                    interaction.print_newline();
                }
                interaction.print_info(&resp);
                interaction.flush();
            }
            interaction.print_newline();

            let msg_count = session_manager.active_session()
                .map_or(0, |s| s.messages.len());

            // Post-turn: broadcast to all hooks (nudge hook may trigger sub-turn via callback)
            let response_str = response_text.as_deref().unwrap_or_default();
            self.config.hooks.post_turn(response_str, msg_count);

            is_resumed_session = false;
        }
    }

    fn request_interrupt(&self, message: Option<String>) {
        // DFS interrupt from deepest subagents first (leaf-level nodes
        // are interrupted before their parents, matching the expected
        // termination propagation order).
        self.interrupt_hub.dfs_interrupt_children(message);
    }
}
