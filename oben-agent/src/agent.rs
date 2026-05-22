//! Agent — shared between CLI and TUI.
//!
//! **Resource Ownership:**
//! Agent owns all runtime resources — Transport, Tools, Skills, SystemPrompt,
//! ContextEngine, and SessionManager. It delegates turn execution to `ConversationLoop`.
//!
//! **Responsibilities:**
//! - Own Transport, Tools, Skills, SystemPrompt, ContextEngine (resource management)
//! - Own SessionManager (session lifecycle, persistence)
//! - Delegate turn cycle to ConversationLoop
//! - Manage session switching
//! - Interactive chat loop
//! - Cross-thread interrupt and steer (Tier 1 features)
//!
//! **Tier 1 Features (Hermes AIAgent Parity):**
//! - Interrupt handling: cross-thread atomic flag for graceful stop
//! - Steer mechanism: inject text into next tool result without interrupting
//! - Iteration budget: with 80%/90% warnings (via ConversationLoop)
//! - Retry with backoff: jittered exponential retry (via ConversationLoop)
//! - Error classification: categorize API errors (via ConversationLoop)

use std::sync::Arc;

use anyhow::Result;
use oben_models::StreamDeltaCallback;
use oben_sessions::SessionManager;

use crate::conversation::{ChatCallbacks, ConversationLoop};
use crate::interrupt::InterruptState;

/// Configuration for building an `Agent`.
pub struct AgentConfig {
    /// System prompt for the agent.
    pub system_prompt: String,
    /// Transport for LLM calls.
    pub transport: oben_transport::ChatCompletionsTransport,
    /// Registered tools.
    pub tools: std::sync::Arc<oben_tools::ToolRegistry>,
    /// Skills directories.
    pub skills_dirs: Vec<std::path::PathBuf>,
    /// Compression configuration.
    pub context_config: crate::compact::CompactCofig,
    /// Max iteration budget per turn.
    pub max_iterations: usize,
    /// Max messages in context.
    pub max_messages: usize,
}

/// An interactive agent — owns all resources, delegates to ConversationLoop.
pub struct Agent {
    /// Transport — owned by Agent, shared across sessions.
    transport: Arc<oben_transport::ChatCompletionsTransport>,
    /// Tools registry — owned by Agent.
    tools: Arc<oben_tools::ToolRegistry>,
    /// Skills dirs — owned by Agent.
    skills_dirs: Vec<std::path::PathBuf>,
    /// System prompt — owned by Agent.
    system_prompt: String,
    /// Context engine — owned by Agent, manages token tracking & compression.
    context_engine: Box<dyn crate::context::ContextEngine>,
    /// Call mode — tracked per-session (Fresh on first turn, Incremental after).
    call_mode: Option<oben_models::CallMode>,
    /// Session manager — owns session lifecycle and persistence.
    session_manager: SessionManager,
    /// Interrupt state — shared via Arc with ConversationLoop/TurnExecutor.
    interrupt_state: Arc<InterruptState>,
}

impl Agent {
    /// Create a new agent. Does NOT own a tokio runtime.
    pub fn new(config: AgentConfig) -> Result<Self> {
        let mut agent = Self {
            transport: Arc::new(config.transport),
            tools: config.tools,
            skills_dirs: config.skills_dirs,
            system_prompt: config.system_prompt,
            context_engine: Box::new(crate::compact_context::CompactContextEngine::with_config(config.context_config)),
            call_mode: None,
            session_manager: SessionManager::new()?,
            interrupt_state: Arc::new(InterruptState::new()),
        };
        agent.eager_load_active_session();
        Ok(agent)
    }

    fn eager_load_active_session(&mut self) {
        if let Some(active) = self.session_manager.active_session() {
            let sid = active.id.clone();
            let _ = self.session_manager.switch_session(&sid);
        }
    }

    /// Access the session manager for listing/saving outside the turn cycle.
    pub fn session_manager(&self) -> &SessionManager {
        &self.session_manager
    }

    /// Mutably access the session manager (for admin ops: load, delete, new).
    pub fn session_manager_mut(&mut self) -> &mut SessionManager {
        &mut self.session_manager
    }

    // ── Tier 1: Interrupt & Steer ────────────────────────────────────────

    /// Interrupt the agent's current tool-calling loop.
    ///
    /// Call this from another thread (e.g. input handler, message receiver)
    /// to gracefully stop the agent and process a new message.
    ///
    /// Also signals long-running tool executions to terminate early, so the
    /// agent can respond immediately.
    ///
    /// # Arguments
    /// * `message` — Optional message that triggered the interrupt.
    pub fn interrupt(&self, message: Option<&str>) {
        let msg = message.map(|s| s.to_string());
        self.interrupt_state.request_interrupt(msg);
    }

    /// Clear any pending interrupt request.
    pub fn clear_interrupt(&self) {
        self.interrupt_state.clear_interrupt();
    }

    /// Inject a user note into the next tool result without interrupting.
    ///
    /// Unlike `interrupt()`, this does NOT stop the current tool call. The
    /// text is stashed and the agent loop appends it to the LAST tool
    /// result's content once the current tool batch finishes.
    ///
    /// Thread-safe: callable from any thread.
    ///
    /// # Returns
    /// `true` if the steer was accepted, `false` if the text was empty.
    pub fn steer(&self, text: &str) -> bool {
        self.interrupt_state.steer(text)
    }

    /// Get activity summary for diagnostics.
    pub fn get_activity_summary(&self) -> crate::interrupt::ActivitySummary {
        self.interrupt_state.get_activity_summary()
    }

    /// Execute one conversation turn.
    ///
    /// Full turn cycle:
    /// 1. Resolve session ID (lazy create if none)
    /// 2. Update call mode (Fresh → Incremental)
    /// 3. Execute turn (preflight + execute) via ConversationLoop
    /// 4. Save session
    pub async fn turn(
        &mut self,
        input: &str,
        _stream: bool,
        delta_callback: Option<StreamDeltaCallback>,
    ) -> Result<String> {
        // Phase 1: resolve session
        let sid = self.resolve_session();

        // Phase 2: update call mode (Fresh → Incremental)
        let call_mode = match &self.call_mode {
            Some(m) => m.clone(),
            None => {
                let mode = oben_models::CallMode::Fresh(sid.clone());
                self.call_mode = Some(mode.clone());
                mode
            }
        };

        let input_msg = oben_models::Message::user(input);

        // Phase 3: execute turn (preflight + execute in one borrow)
        let response = ConversationLoop::execute_turn(
            &mut self.context_engine,
            &self.transport,
            &self.tools,
            &mut self.session_manager,
            &sid,
            input_msg,
            &call_mode,
            delta_callback,
        ).await?;

        // Phase 4: persist
        self.session_manager.save(None)?;

        Ok(response)
    }

    /// Resolve session ID (lazy create if no active session).
    fn resolve_session(&mut self) -> String {
        let sid = {
            let active_id = self.session_manager.active_session().map(|s| s.id.clone());
            match active_id {
                Some(sid) => self.session_manager.switch_session(&sid)
                    .map(|s| s.id.clone())
                    .unwrap_or_else(|_| {
                        self.session_manager.new_session(&format!(
                            "chat-{}", chrono::Utc::now().format("%Y%m%d-%H%M%S")
                        )).id.clone()
                    }),
                None => self.session_manager.new_session(&format!(
                    "chat-{}", chrono::Utc::now().format("%Y%m%d-%H%M%S")
                )).id.clone(),
            }
        };
        sid
    }

    /// Switch to an existing session by ID or name.
    pub fn continue_session(&mut self, key: &str) -> Result<String> {
        self.session_manager.init()?;
        let sid = self.session_manager.find_key(key)
            .ok_or_else(|| anyhow::anyhow!("Session not found: {}. Run `oben sessions list` to see available sessions.", key))?;

        self.session_manager.switch_session(&sid)?;
        let name = self.session_manager.active_session()
            .map(|s| s.name.clone()).unwrap_or(key.to_string());
        Ok(name)
    }

    /// Reset the current session (clear messages).
    pub fn reset(&mut self) -> Result<()> {
        if let Some(session) = self.session_manager.active_session_mut() {
            session.messages.clear();
        }
        Ok(())
    }

    /// Get the currently loaded session name.
    pub fn loaded_session_name(&self) -> Option<String> {
        self.session_manager.active_session().map(|s| s.name.clone())
    }

    /// Get the active session name (alias for loaded_session_name).
    pub fn active_session_name(&self) -> Option<String> {
        self.loaded_session_name()
    }

    /// Run an interactive chat — delegates loop to ConversationLoop.
    ///
    /// Agent only handles session setup; ConversationLoop runs the
    /// turn loop with call_mode (Fresh → Incremental) management.
    pub async fn interactive_chat(
        &mut self,
        stream: bool,
        continue_with: Option<&str>,
        callbacks: ChatCallbacks,
    ) -> Result<()> {
        // Session setup — Agent owns session lifecycle
        if let Some(key) = continue_with {
            let resolved = if key == "latest" {
                self.session_manager.active_session()
                    .map(|s| s.name.clone()).unwrap_or_else(|| key.to_string())
            } else {
                key.to_string()
            };
            let name = self.continue_session(&resolved)?;
            if let Some(s) = self.session_manager.active_session() {
                let count = s.messages.len();
                (callbacks.print_info)(&format!("Continuing session: {} ({} messages)\n", name, count));
                print_session_messages(&s.messages, 10);
                (callbacks.print_info)("");
            }
        } else if let Some(name) = self.loaded_session_name() {
            if let Some(s) = self.session_manager.active_session() {
                (callbacks.print_info)(&format!("Session: {} ({} messages)\n", name, s.messages.len()));
            }
        }
        (callbacks.print_info)("🦀 ObenAgent ready. Type 'quit' or 'exit' to stop.\n");
        (callbacks.print_flush)();

        // Delegate loop to ConversationLoop
        ConversationLoop::run_loop(
            &mut self.context_engine,
            &self.transport,
            &self.tools,
            &mut self.session_manager,
            &mut self.call_mode,
            stream,
            callbacks,
        ).await
    }
}

fn print_session_messages(messages: &[oben_models::Message], max_show: usize) {
    if messages.is_empty() { println!("(no messages)"); return; }
    let show_count = messages.len().min(max_show);
    let show = &messages[..show_count];
    let overflow = messages.len().saturating_sub(max_show);
    for msg in show {
        let role = match msg.role {
            oben_models::MessageRole::User => "📝 你",
            oben_models::MessageRole::Assistant => "🤖 agent",
            oben_models::MessageRole::System => "📋 system",
            oben_models::MessageRole::Tool => "⚙️ tool",
        };
        let text = msg.content.to_text_ref().unwrap_or("<non-text>");
        let display = if text.len() > 120 { format!("{}...", &text[..117]) } else { text.to_string() };
        println!("  {} {}", role, display);
    }
    if overflow > 0 { println!("  ... {} more messages", overflow); }
}
