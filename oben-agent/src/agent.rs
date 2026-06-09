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
//! - Cross-thread interrupt and steer (Tier 1)
//! - Fallback model chain (Tier 2)
//! - Rich callback system (Tier 2)
//! - Activity tracking (Tier 2)
//! - Concurrent tool dispatch (Tier 2)
//! - Nudge / background review (Tier 2)

use std::sync::Arc;
use tokio::sync::Mutex;

use anyhow::Result;

pub(crate) fn generate_session_name() -> String {
    let ts = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    let r = rand::random::<u64>() % 1_000_000;
    format!("{}-{:06}", ts, r)
}
use oben_models::{CallMode, Message, StreamDeltaCallback};
use oben_sessions::SessionManager;

use crate::callbacks::AgentCallbacks;
use crate::conversation::{ChatCallbacks, ConversationLoop};
use crate::fallback::FallbackChain;
use crate::interrupt::InterruptState;
use crate::nudge::NudgeConfig;
use crate::system_prompt_cache::SystemPromptCache;

/// Configuration for building an `Agent`.
pub struct AgentConfig {
    /// System prompt for the agent.
    pub system_prompt: String,
    /// Transport for LLM calls — a trait object so the registry can return any registered transport.
    pub transport: std::sync::Arc<dyn oben_models::providers::TransportProvider + Send + Sync>,
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
    /// Fallback model chain.
    pub fallback_models: Vec<crate::fallback::FallbackConfig>,
    /// Agent callbacks for platform integration.
    pub callbacks: AgentCallbacks,
    /// Concurrent dispatch configuration.
    pub concurrent_dispatch_config: crate::concurrent_dispatch::ConcurrentDispatchConfig,
    /// Nudge / background memory review config (tier-2).
    /// Set `memory_nudge_interval` to 0 to disable.
    pub nudge_config: Option<NudgeConfig>,
}

/// An interactive agent — owns all resources, delegates to ConversationLoop.
pub struct Agent {
    /// Transport — owned by Agent, shared across sessions.
    transport: std::sync::Arc<dyn oben_models::providers::TransportProvider + Send + Sync>,
    /// Tools registry — owned by Agent.
    tools: Arc<oben_tools::ToolRegistry>,
    /// Context engine — owned by Agent, manages token tracking & compression.
    context_engine: Box<dyn crate::context::ContextEngine>,
    /// Call mode — tracked per-session (Fresh on first turn, Incremental after).
    call_mode: Option<oben_models::CallMode>,
    /// Session manager — owns session lifecycle and persistence.
    session_manager: Arc<Mutex<SessionManager>>,
    /// Interrupt state — shared via Arc with ConversationLoop/TurnExecutor.
    interrupt_state: Arc<InterruptState>,
    /// Fallback model chain.
    fallback_chain: FallbackChain,
    /// Agent callbacks.
    callbacks: AgentCallbacks,
    /// System prompt prefix cache.
    prompt_cache: SystemPromptCache,
    /// Concurrent dispatch config.
    dispatch_config: crate::concurrent_dispatch::ConcurrentDispatchConfig,
    /// Nudge / background memory review config.
    nudge_config: NudgeConfig,
}

impl Agent {
    /// Create a new agent. Does NOT own a tokio runtime.
    pub async fn new(config: AgentConfig) -> Result<Self> {
        let session_manager = Arc::new(Mutex::new(SessionManager::new()?));

        let mut agent = Self {
            transport: config.transport,
            tools: config.tools,
            context_engine: Box::new(crate::compact_context::CompactContextEngine::with_config(
                config.context_config,
            )),
            call_mode: None,
            session_manager,
            interrupt_state: Arc::new(InterruptState::new()),
            fallback_chain: FallbackChain::new(config.fallback_models),
            callbacks: config.callbacks,
            prompt_cache: SystemPromptCache::new(),
            dispatch_config: config.concurrent_dispatch_config,
            nudge_config: config.nudge_config.unwrap_or_default(),
        };
        // Initialize the prompt cache with the initial system prompt.
        // The cache will be updated on each compaction/session change.
        agent.prompt_cache.set_prompt(&config.system_prompt);
        agent.eager_load_active_session().await;
        Ok(agent)
    }

    /// Create a minimal Agent for testing. Does NOT call real transport or LLMs.
    pub async fn new_for_test() -> Self {
        use crate::fallback::FallbackChain;
        use oben_models::providers::TransportProvider;
        use oben_models::{CallMode, TransportResponse};

        // A no-op transport stub
        struct TestTransport;
        #[::async_trait::async_trait]
        impl TransportProvider for TestTransport {
            fn name(&self) -> &str { "test" }
            async fn chat(
                &self,
                _messages: &[Message],
                _mode: &CallMode,
            ) -> Result<TransportResponse> {
                Ok(TransportResponse {
                    text: String::new(),
                    tool_calls: Vec::new(),
                    tokens_used: None,
                })
            }
            async fn stream_chat(
                &self,
                _messages: &[Message],
                _mode: &CallMode,
                _callback: StreamDeltaCallback,
            ) -> Result<TransportResponse> {
                Ok(TransportResponse {
                    text: String::new(),
                    tool_calls: Vec::new(),
                    tokens_used: None,
                })
            }
        }

        let session_manager = Arc::new(Mutex::new(SessionManager::new().unwrap()));
        let transport = Arc::new(TestTransport) as Arc<dyn TransportProvider + Send + Sync>;
        let tools = Arc::new(oben_tools::ToolRegistry::new());

        Self {
            transport,
            tools,
            context_engine: Box::new(crate::compact_context::CompactContextEngine::new()),
            call_mode: None,
            session_manager,
            interrupt_state: Arc::new(InterruptState::new()),
            fallback_chain: FallbackChain::new(Vec::new()),
            callbacks: AgentCallbacks::default(),
            prompt_cache: SystemPromptCache::new(),
            dispatch_config: crate::concurrent_dispatch::ConcurrentDispatchConfig::default(),
            nudge_config: NudgeConfig::default(),
        }
    }

    async fn eager_load_active_session(&mut self) {
        let sid = self
            .session_manager
            .lock()
            .await
            .active_session()
            .map(|s| s.id.clone());
        if let Some(sid) = sid {
            let _ = self.session_manager.lock().await.switch_session(&sid);
        }
    }

    /// Access the session manager for listing/saving outside the turn cycle.
    pub fn session_manager(&self) -> Arc<Mutex<SessionManager>> {
        Arc::clone(&self.session_manager)
    }

    /// Mutably access the session manager (for admin ops: load, delete, new).
    pub fn session_manager_mut(&mut self) -> Arc<Mutex<SessionManager>> {
        Arc::clone(&self.session_manager)
    }

    // ── Tier 1: Interrupt & Steer ────────────────────────────────────────

    /// Interrupt the agent's current tool-calling loop.
    pub fn interrupt(&self, message: Option<&str>) {
        let msg = message.map(|s| s.to_string());
        self.interrupt_state.request_interrupt(msg);
    }

    /// Clear any pending interrupt request.
    pub fn clear_interrupt(&self) {
        self.interrupt_state.clear_interrupt();
    }

    /// Inject a user note into the next tool result without interrupting.
    pub fn steer(&self, text: &str) -> bool {
        self.interrupt_state.steer(text)
    }

    /// Get a shared reference to the interrupt state.
    ///
    /// This allows external code (e.g. TUI event loop) to call
    /// `request_interrupt()` without acquiring the tokio::sync::Mutex
    /// that the spawn task holds across `turn()`.  Prevents deadlock.
    pub fn get_interrupt_state(&self) -> Arc<crate::interrupt::InterruptState> {
        Arc::clone(&self.interrupt_state)
    }

    // ── Tier 2: Fallback Models ──────────────────────────────────────────

    /// Set the fallback model chain.
    pub fn set_fallback_chain(&mut self, chain: Vec<crate::fallback::FallbackConfig>) {
        self.fallback_chain = FallbackChain::new(chain);
    }

    /// Check if a fallback was activated.
    pub fn fallback_activated(&self) -> bool {
        self.fallback_chain.is_activated()
    }

    /// Get the active fallback config (if any).
    pub fn active_fallback(&self) -> Option<&crate::fallback::FallbackConfig> {
        self.fallback_chain.active_fallback()
    }

    /// Get fallback chain status for diagnostics.
    pub fn fallback_status(&self) -> String {
        if self.fallback_chain.is_activated() {
            if let Some(fb) = self.active_fallback() {
                format!("Fallback active: {}/{}", fb.provider, fb.model)
            } else {
                "Fallback active (unknown)".to_string()
            }
        } else {
            "Primary model active".to_string()
        }
    }

    // ── Tier 2: Callbacks ────────────────────────────────────────────────

    /// Set the agent callbacks.
    pub fn set_callbacks(&mut self, callbacks: AgentCallbacks) {
        self.callbacks = callbacks;
    }

    /// Get reference to current callbacks.
    pub fn callbacks(&self) -> &AgentCallbacks {
        &self.callbacks
    }

    // ── Tier 2: System Prompt Cache ──────────────────────────────────────

    /// Set the cached system prompt after building a new one.
    pub fn set_cached_prompt(&mut self, prompt: &str) {
        self.prompt_cache.set_prompt(prompt);
    }

    /// Get the cached system prompt, if available.
    pub fn get_cached_prompt(&self) -> Option<&str> {
        self.prompt_cache.get_prompt()
    }

    /// Check if we have a cached prompt.
    pub fn has_cached_prompt(&self) -> bool {
        self.prompt_cache.has_prompt()
    }

    // ── Tier 2: Activity Tracking ────────────────────────────────────────

    /// Get activity summary for diagnostics.
    pub fn get_activity_summary(&self) -> crate::interrupt::ActivitySummary {
        self.interrupt_state.get_activity_summary(None, 0, 0)
    }

    // ── Tier 2: Concurrent Dispatch ──────────────────────────────────────

    /// Get the concurrent dispatch config.
    pub fn dispatch_config(&self) -> &crate::concurrent_dispatch::ConcurrentDispatchConfig {
        &self.dispatch_config
    }

    /// Execute one conversation turn.
    pub async fn turn(
        &mut self,
        input: &str,
        _stream: bool,
        delta_callback: Option<StreamDeltaCallback>,
        interrupt: Option<Arc<crate::interrupt::InterruptState>>,
    ) -> Result<String> {
        let sid = self.resolve_session().await;

        let call_mode = match &self.call_mode {
            Some(m) => m.clone(),
            None => {
                let mode = oben_models::CallMode::Fresh(sid.clone());
                self.call_mode = Some(mode.clone());
                mode
            }
        };

        let input_msg = oben_models::Message::user(input);
        let sm = Arc::clone(&self.session_manager);

        let response = ConversationLoop::execute_turn_with_options(
            &mut self.context_engine,
            &self.transport,
            &self.tools,
            &mut *sm.lock().await,
            &sid,
            input_msg,
            &call_mode,
            delta_callback,
            crate::conversation::TurnOptions {
                retry_config: crate::retry::RetryConfig::default(),
                budget: None,
                interrupt,
                callbacks: Some(std::mem::replace(
                    &mut self.callbacks,
                    crate::callbacks::AgentCallbacks::default(),
                )),
                fallback: None,
            },
        )
        .await?;

        sm.lock().await.incremental_save(None)?;

        Ok(response)
    }

    /// Resolve session ID (lazy create if no active session).
    async fn resolve_session(&mut self) -> String {
        let sm = Arc::clone(&self.session_manager);
        let sid = {
            let guard = sm.lock().await;
            let active_id = guard.active_session().map(|s| s.id.clone());
            drop(guard);
            match active_id {
                Some(sid) => match sm.lock().await.switch_session(&sid) {
                    Ok(s) => s.id.clone(),
                    Err(_) => sm
                        .lock()
                        .await
                        .new_session(&generate_session_name())
                        .id
                        .clone(),
                },
                None => sm
                    .lock()
                    .await
                    .new_session(&generate_session_name())
                    .id
                    .clone(),
            }
        };
        sid
    }

    /// Switch to an existing session by ID or name.
    pub async fn continue_session(&mut self, key: &str) -> Result<String> {
        let sm = Arc::clone(&self.session_manager);
        sm.lock().await.init()?;
        let sid = {
            sm.lock().await.find_key(key).ok_or_else(|| {
                anyhow::anyhow!(
                    "Session not found: {}. Run `oben sessions list` to see available sessions.",
                    key
                )
            })?
        };
        sm.lock().await.switch_session(&sid)?;
        let name = {
            sm.lock()
                .await
                .active_session()
                .map(|s| s.name.clone())
                .unwrap_or(key.to_string())
        };
        Ok(name)
    }

    /// Reset the current session: delete it (messages + DB record) and enter
    /// a no-session state. The next [turn] will lazily create a new session
    /// via [resolve_session].
    pub async fn reset(&mut self) -> Result<()> {
        let sm = Arc::clone(&self.session_manager);
        let sid = { sm.lock().await.active_session().map(|s| s.id.clone()) };
        if let Some(sid) = sid {
            // Delete from DB and in-memory cache; sets active_session_id = None
            sm.lock().await.delete_session(&sid)?;
        }
        // Reset call mode so the next turn starts as Fresh.
        self.call_mode = None;
        Ok(())
    }

    /// Compact (summarize) the current session context.
    ///
    /// Retrieves the active session messages, mutates them in-place via the
    /// context engine's compaction logic, then saves the session back.
    ///
    /// Returns a `CompactOutcome` describing what happened:
    /// - `AlreadyCompact` — messages within budget, no compaction needed
    /// - `NoMiddleMessages` — all messages protected (head/tail), no LLM call
    /// - `Ineffective` — compression attempted but savings below threshold
    /// - `Compressed` — messages successfully compacted with a summary
    pub async fn compact_session(&mut self) -> crate::compact::CompactOutcome {
        let sm = Arc::clone(&self.session_manager);
        // Ensure session manager is initialized
        if let Err(e) = sm.lock().await.init() {
            tracing::error!("Session manager init failed: {e}");
            return crate::compact::CompactOutcome::AlreadyCompact;
        }

        // Get the active session
        let (sid, mut messages) = {
            let mut guard = sm.lock().await;
            let sid = match guard.active_session().map(|s| s.id.clone()) {
                Some(id) => id,
                None => return crate::compact::CompactOutcome::AlreadyCompact,
            };
            let messages = match guard.active_session_mut() {
                Some(s) => s.messages.clone(),
                None => return crate::compact::CompactOutcome::AlreadyCompact,
            };
            (sid, messages)
        };

        // Check if compaction is needed
        if !self.context_engine.should_compact(&messages) {
            return crate::compact::CompactOutcome::AlreadyCompact;
        }

        // Perform compaction
        let status = match self
            .context_engine
            .compact(&mut messages, Some(self.transport.as_ref()), None)
            .await
        {
            Ok(status) => status,
            Err(e) => {
                tracing::error!("ContextEngine::compact failed: {e}");
                return crate::compact::CompactOutcome::AlreadyCompact;
            }
        };

        match status {
            crate::context::CompactStatus::Compacted => {
                // Save the compacted messages back to the session.
                // The incremental save() path (tail.append) can't handle compaction
                // because persisted_message_count > messages.len() after compression.
                // save_compacted() handles: clear old DB messages, insert compacted set,
                // update in-memory session and persisted_message_count.
                match sm.lock().await.save_compacted(&sid, &messages) {
                    Ok(()) => {}
                    Err(e) => {
                        tracing::error!("Failed to save compacted session {sid}: {e}");
                    }
                }
                crate::compact::CompactOutcome::Compressed {
                    original_count: 0,
                    compacted_count: 0,
                    savings_pct: 0.0,
                }
            }
            crate::context::CompactStatus::Unchanged => {
                // Compression was ineffective — messages unchanged, nothing to persist.
                tracing::warn!(
                    "Manual compaction ineffective (session {}), skipping DB update",
                    sid
                );
                crate::compact::CompactOutcome::Ineffective {
                    original_tokens: 0,
                    compacted_tokens: 0,
                    savings_pct: 0.0,
                }
            }
        }
    }

    /// Create a new session and switch to it. The old session is preserved
    /// and can be restored later via [continue_session].
    pub async fn new_session(&mut self) -> Result<String> {
        let sm = Arc::clone(&self.session_manager);
        let new_id = sm
            .lock()
            .await
            .new_session(&generate_session_name())
            .id
            .clone();
        // Reset call mode so next turn starts Fresh
        self.call_mode = None;
        Ok(new_id)
    }

    /// Get the currently loaded session display name (title if set, else internal name).
    pub async fn loaded_session_name(&self) -> Option<String> {
        let sm = Arc::clone(&self.session_manager);
        let guard = sm.lock().await;
        guard
            .active_session()
            .map(|s| s.metadata.title.as_deref().unwrap_or(&s.name).to_string())
    }

    /// Get the active session name.
    pub async fn active_session_name(&self) -> Option<String> {
        self.loaded_session_name().await
    }

    /// Get the active session messages.
    pub async fn loaded_session_messages(&self) -> Result<Vec<oben_models::Message>> {
        let sm = Arc::clone(&self.session_manager);
        let guard = sm.lock().await;
        let msgs = guard
            .active_session()
            .map(|s| s.messages.clone())
            .unwrap_or_default();
        Ok(msgs)
    }

    /// Initialize the session manager (for admin ops in async context).
    pub async fn init_session_manager(&mut self) -> Result<()> {
        let sm = Arc::clone(&self.session_manager);
        let res = sm.lock().await.init();
        res
    }

    /// Find a session key by name (for admin ops in async context).
    pub async fn find_session_key(&self, key: &str) -> Option<String> {
        let sm = Arc::clone(&self.session_manager);
        let result = sm.lock().await.find_key(key);
        result
    }

    /// Switch to a session by ID (for app-level session loading).
    pub async fn switch_session_to(&mut self, id: &str) -> Result<()> {
        let sm = Arc::clone(&self.session_manager);
        let _ = sm.lock().await.switch_session(id)?;
        Ok(())
    }

    /// List all sessions (wrapper for SessionsPanel).
    pub async fn list_sessions_full(&self) -> Vec<oben_models::Session> {
        let sessions = self.session_manager.lock().await.list_sessions_full();
        sessions
    }

    /// Get session messages (wrapper for SessionsPanel preview).
    pub async fn get_session_messages(
        &self,
        session_id: &str,
    ) -> Result<Vec<oben_models::Message>> {
        let msgs = self
            .session_manager
            .lock()
            .await
            .get_session_messages(session_id)?;
        Ok(msgs)
    }

    /// Load a session's messages into the in-memory cache (for refresh_list).
    pub async fn load_session_messages(&mut self, session_id: &str) -> Result<()> {
        self.session_manager.lock().await.ensure_session_loaded(session_id)?;
        Ok(())
    }

    /// Delete a session (wrapper for SessionsPanel).
    pub async fn delete_session(&mut self, session_id: &str) -> Result<()> {
        self.session_manager.lock().await.delete(session_id)?;
        Ok(())
    }

    /// Switch to a session (wrapper for SessionsPanel compact/switch).
    pub async fn switch_session(&mut self, session_id: &str) -> Result<()> {
        let _session = self
            .session_manager
            .lock()
            .await
            .switch_session(session_id)?;
        Ok(())
    }

    /// Close current session (wrapper for SessionsPanel).
    pub async fn close_session(&mut self) -> Result<()> {
        self.session_manager.lock().await.close()?;
        Ok(())
    }

    /// Run an interactive chat.
    pub async fn interactive_chat(
        &mut self,
        stream: bool,
        continue_with: Option<&str>,
        callbacks: ChatCallbacks,
    ) -> Result<()> {
        let sm = Arc::clone(&self.session_manager);
        if let Some(key) = continue_with {
            let resolved = if key == "latest" {
                sm.lock()
                    .await
                    .active_session()
                    .map(|s| s.name.clone())
                    .unwrap_or_else(|| key.to_string())
            } else {
                key.to_string()
            };
            let name = self.continue_session(&resolved).await?;
            if let Some(s) = sm.lock().await.active_session() {
                let count = s.messages.len();
                (callbacks.print_info)(&format!(
                    "Continuing session: {} ({} messages)\n",
                    name, count
                ));
                print_session_messages(&s.messages, 10);
                (callbacks.print_info)("");
            }
        } else if let Some(name) = self.loaded_session_name().await {
            if let Some(s) = sm.lock().await.active_session() {
                (callbacks.print_info)(&format!(
                    "Session: {} ({} messages)\n",
                    name,
                    s.messages.len()
                ));
            }
        }
        (callbacks.print_info)("🦀 ObenAgent ready. Type 'quit' or 'exit' to stop.\n");
        (callbacks.print_flush)();

        let sm = Arc::clone(&self.session_manager);
        let _result = ConversationLoop::run_loop(
            &mut self.context_engine,
            &self.transport,
            &self.tools,
            &mut *sm.lock().await,
            &mut self.call_mode,
            stream,
            callbacks,
            &self.nudge_config,
        )
        .await;
        _result
    }

    /// Run a background memory/skill review turn (nudge).
    ///
    /// This is the internal implementation of C.15 — the "turn nudge" that
    /// automatically reviews conversation history and optionally updates
    /// MEMORY.md / USER.md via the memory tool.
    ///
    /// Mirrors Hermes' `_spawn_background_review` + `_run_review_in_thread`:
    /// executes a bounded review turn on the same agent with a custom
    /// prompt. The result is delivered via the provided callback.
    pub async fn trigger_nudge(
        &mut self,
        _user_message: &str,
        memory_interval: usize,
        skill_interval: usize,
        on_complete: impl FnOnce(bool, String) + Send + 'static,
    ) {
        // Quick-exit if nudge is disabled.
        if memory_interval == 0 && skill_interval == 0 {
            return;
        }

        let sm = Arc::clone(&self.session_manager);
        let has_memory_tool = {
            let guard = sm.lock().await;
            !guard
                .active_session()
                .map(|s| s.messages.iter().any(|m| !m.tool_calls.is_none()))
                .unwrap_or(true)
        };

        if !has_memory_tool {
            on_complete(
                false,
                "Nudge skipped: no memory tools available.".to_string(),
            );
            return;
        }

        // Build the nudge prompt (mirrors Hermes _MEMORY_REVIEW_PROMPT).
        let prompt = self.build_nudge_prompt(memory_interval > 0, skill_interval > 0);

        let budget = crate::budget::IterationBudget::new(16);
        let turn_options = crate::conversation::TurnOptions {
            retry_config: Default::default(),
            budget: Some(budget),
            interrupt: None,
            callbacks: None, // suppress user-facing callbacks during review
            fallback: None,
        };

        let sm = Arc::clone(&self.session_manager);
        let sid = {
            let guard = sm.lock().await;
            let active_id = guard.active_session().map(|s| s.id.clone());
            drop(guard);
            let sid = match active_id {
                Some(sid) => sid,
                None => sm
                    .lock()
                    .await
                    .new_session(&generate_session_name())
                    .id
                    .clone(),
            };
            sm.lock()
                .await
                .switch_session(&sid)
                .map(|s| s.id.clone())
                .unwrap_or_else(|_| {
                    // Already ensured we have a valid session above; fall back to a fresh one
                    generate_session_name()
                })
        };
        let call_mode = self
            .call_mode
            .clone()
            .unwrap_or(CallMode::Fresh(sid.clone()));

        let review_msg = Message::user(&prompt);
        let sm = Arc::clone(&self.session_manager);
        let response_text = ConversationLoop::execute_turn_with_options(
            &mut self.context_engine,
            &self.transport,
            &self.tools,
            &mut *sm.lock().await,
            &sid,
            review_msg,
            &call_mode,
            None,
            turn_options,
        )
        .await;

        let _result = match response_text {
            Ok(text) => {
                let text_lower = text.to_lowercase();
                let is_noop = text_lower.contains("nothing to")
                    || text_lower.contains("nothing worth")
                    || text_lower.contains("no changes needed");

                let updated = !is_noop;
                let summary = if is_noop {
                    "Review: nothing worth saving this session.".to_string()
                } else {
                    "Review: checked memory — may have updated.".to_string()
                };
                on_complete(updated, summary);
            }
            Err(e) => {
                tracing::info!("Nudge review failed (non-fatal): {}", e);
                on_complete(false, format!("Review failed: {}", e));
            }
        };
    }

    fn build_nudge_prompt(&self, memory_enabled: bool, skill_enabled: bool) -> String {
        if memory_enabled && skill_enabled {
            format!(
                "Review the conversation above and consider the following:\n\n\
                 MEMORY REVIEW:\n\
                 1. Has the user revealed things about themselves — their persona, desires, \
                 preferences, or personal details worth remembering?\n\
                 2. Has the user expressed expectations about how you should behave, their work \
                 preferences, or their communication style?\n\
                 3. What are the most important lessons from recent interactions?\n\n\
                 SKILL REVIEW:\n\
                 1. Did the user reveal a recurring task or workflow that should become a skill?\n\
                 2. Are there any tools or integrations worth setting up?\n\n\
                 Only suggest changes if there's something genuinely useful to save. \
                 If nothing worth saving, say so briefly and stop.\n\n\
                 Think through your reasoning before deciding."
            )
        } else if memory_enabled {
            format!(
                "Review the conversation above and consider the following:\n\n\
                 MEMORY REVIEW:\n\
                 1. Has the user revealed things about themselves — their persona, desires, \
                 preferences, or personal details worth remembering?\n\
                 2. Has the user expressed expectations about how you should behave, their work \
                 preferences, or their communication style?\n\
                 3. What are the most important lessons from recent interactions?\n\n\
                 Only suggest changes if there's something genuinely useful to save. \
                 If nothing worth saving, say so briefly and stop.\n\n\
                 Think through your reasoning before deciding."
            )
        } else {
            format!(
                "Review the conversation above and consider the following:\n\n\
                 SKILL REVIEW:\n\
                 1. Did the user reveal a recurring task or workflow that should become a skill?\n\
                 2. Are there any tools or integrations worth setting up?\n\n\
                 Only suggest changes if there's something genuinely useful to save. \
                 If nothing worth saving, say so briefly and stop.\n\n\
                 Think through your reasoning before deciding."
            )
        }
    }
}

fn print_session_messages(messages: &[Message], max_show: usize) {
    if messages.is_empty() {
        tracing::info!(message_count = 0, "(no messages)");
        return;
    }
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
        let text_display = if text.len() > 120 {
            format!("{}...", &text[..117])
        } else {
            text.to_string()
        };
        tracing::info!(role, display = %text_display, "message preview");
    }
    if overflow > 0 {
        tracing::info!(overflow, more_messages = true, "... more messages");
    }
}
