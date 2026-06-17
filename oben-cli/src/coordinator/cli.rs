/// CLI coordinator — replaces the legacy `Agent::interactive_chat` loop with a structured
/// coordinator that owns hooks, termination policy, and the loop itself.
use std::sync::Arc;
use oben_agent::coordinator::tree::InterruptHub;

use anyhow::{anyhow, Result};

use oben_agent::context::ContextWindowManager;
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
    /// Maximum number of user turns in the conversation (None = unlimited).
    pub max_turns: Option<usize>,
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

/// Streaming hook that writes each delta directly to stdout.
pub struct CliStreamingHook;

impl CliStreamingHook {
    pub fn new() -> Self { Self }
}

impl oben_agent::hooks::kind::Hook for CliStreamingHook {
    fn id(&self) -> &str { "cli_streaming" }
    fn priority(&self) -> u32 { 1 }
}

impl oben_agent::hooks::kind::StreamingHooks for CliStreamingHook {
    fn on_stream_delta(&self, text: &str) {
        if !text.is_empty() {
            use std::io::Write;
            let _ = std::io::stdout().write_all(text.as_bytes());
            let _ = std::io::stdout().flush();
        }
    }
}

impl CliCoordinator {
    /// Create a new CLI coordinator from conversation config and shared hooks.
    /// Registers a CLI streaming hook if stream is enabled.
    pub fn from_conversation(
        conversation: ConversationConfig,
        hooks: Arc<HookEngine>,
        stream: bool,
        max_turns: Option<usize>,
    ) -> Self {
        if stream {
            hooks.register_streaming(Box::new(CliStreamingHook::new()));
        }
        let config = CliConfig {
            conversation,
            hooks,
            stream,
            max_turns,
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
        context_window_manager: &mut dyn ContextWindowManager,
        transport: Arc<dyn TransportProvider + Send + Sync>,
        tools: Arc<ToolRegistry>,
        session_manager: &mut dyn SessionManager,
    ) -> Result<ConversationResult> {
        let interaction = oben_agent::interaction::StdioProvider::new();

        // Fire loop-start hooks.
        self.config.hooks.emit_loop_start();

        let mut turn_count: usize = 0;
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

            // Check max turns limit before executing turn
            if let Some(max_turns) = self.config.max_turns {
                if turn_count >= max_turns {
                    self.config.hooks.emit_loop_end("max_turns_reached");
                    return Ok(ConversationResult::BudgetExhausted);
                }
            }
            turn_count += 1;

            // Get or create session via CWM — this is now the single source of truth
            // for which session is active (SessionManager no longer tracks active identity).
            let sid = match context_window_manager.session_id() {
                Some(id) => id,
                None => {
                    let id = oben_agent::agent::generate_session_name();
                    context_window_manager.set_active_session(session_manager, id.clone());
                    id
                }
            };

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

            let response = oben_agent::coordinator::execute_turn_full(
                context_window_manager,
                &*transport,
                &tools,
                session_manager,
                &sid,
                input_msg,
                &call_mode_val,
                &self.config.conversation,
                Some(Arc::clone(&self.config.hooks)),
                None,
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
                // In streaming mode, the HookEngine streaming hooks already printed
                // the deltas during execute_turn_full. Print a newline + finalize the line.
                // In non-streaming mode, print the full response here.
                interaction.print_newline();
                if !self.config.stream {
                    interaction.print_info(resp);
                    interaction.flush();
                }
            } else {
                interaction.print_newline();
            }

            let msg_count = session_manager.session(&sid)
                .map_or(0, |s| s.messages.len());

            // Post-turn: broadcast to all hooks (nudge hook may trigger sub-turn via callback)
            let response_str = response_text.as_deref().unwrap_or_default();
            self.config.hooks.post_turn(response_str, msg_count);
        }
    }

    fn request_interrupt(&self, message: Option<String>) {
        // DFS interrupt from deepest subagents first (leaf-level nodes
        // are interrupted before their parents, matching the expected
        // termination propagation order).
        self.interrupt_hub.dfs_interrupt_children(message);
    }
}
