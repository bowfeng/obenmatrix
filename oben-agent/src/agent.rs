//! Agent — shared between CLI and TUI.
//!
//! Wraps a `SessionManager` and `ConversationLoop`, handling:
//! - Lazy session creation (only on first input)
//! - Session switching / loading
//! - Call mode management (Fresh → Incremental)
//! - Session save after each turn
//!
//! Neither CLI nor TUI concerns are included — this is pure business logic.

use std::sync::{Arc, Mutex};

use anyhow::Result;
use tracing::info;
use oben_models::{CallMode, Message, StreamDeltaCallback};
use oben_sessions::SessionManager;

use crate::compact_context::CompactContextEngine;
use crate::context::ContextEngine;
use crate::conversation::{ConversationLoop, DefaultConversationLoop};

/// Configuration for building an `Agent`.
pub struct AgentConfig {
    /// Pre-built system prompt text.
    pub system_prompt_text: String,
    /// Transport for LLM calls.
    pub transport: oben_transport::ChatCompletionsTransport,
    /// Registered tools.
    pub tools: std::sync::Arc<oben_tools::ToolRegistry>,
    /// Max iteration budget per turn.
    pub max_iterations: usize,
    /// Max messages in context.
    pub max_messages: usize,
    /// Compression configuration.
    pub context_config: crate::compression::CompressionConfig,
}

/// An interactive agent that owns session lifecycle and conversation.
///
/// Both the CLI `run_chat` and the TUI event loop use this struct.
/// It does not touch stdin/stdout — those are handled by the caller.
///
/// **Async API** — `turn()` returns a `Future`. The caller manages the
/// tokio runtime (typically via `#[tokio::main(flavor = "multi_thread")]`
/// in main.rs, so multiple agents can run concurrently).
pub struct Agent {
    conversation: DefaultConversationLoop,
    session_manager: SessionManager,
    context_engine: Arc<Mutex<dyn ContextEngine>>,
    system_prompt_text: String,
    session_id: Option<String>,
    call_mode: Option<CallMode>,
}

impl Agent {
    /// Create a new agent. Does NOT own a tokio runtime.
    pub fn new(config: AgentConfig) -> Result<Self> {
        let context_engine: Arc<Mutex<dyn ContextEngine>> =
            Arc::new(Mutex::new(CompactContextEngine::with_config(config.context_config)));
        let mut session = Self {
            conversation: ConversationLoop::new(
                config.transport,
                config.tools,
                config.max_iterations,
                config.max_messages,
                context_engine.clone(),
            ),
            session_manager: SessionManager::new()?,
            context_engine,
            system_prompt_text: config.system_prompt_text,
            session_id: None,
            call_mode: None,
        };
        session.eager_load_active_session();
        Ok(session)
    }

    pub fn loaded_session_name(&self) -> Option<String> {
        self.session_id.as_ref().and_then(|sid| {
            self.session_manager.active_session()
                .filter(|s| s.id == *sid)
                .map(|s| s.name.clone())
        })
    }

    fn eager_load_active_session(&mut self) {
        if let Some(active) = self.session_manager.active_session() {
            let sid = active.id.clone();
            let _ = self.session_manager.switch_session(&sid);
            self.session_id = Some(sid);
            self.call_mode = Some(CallMode::Fresh(self.session_id.clone().expect("loaded")));
        }
    }

    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    pub fn session_manager(&self) -> &SessionManager {
        &self.session_manager
    }

    pub fn session_manager_mut(&mut self) -> &mut SessionManager {
        &mut self.session_manager
    }

    /// Execute one conversation turn — async.
    pub async fn turn(
        &mut self,
        input: &str,
        stream: bool,
        delta_callback: Option<StreamDeltaCallback>,
    ) -> Result<String> {
        // ── Phase 1: sync session ID resolution ──────────────────
        let sid = {
            let active_id = self.session_manager.active_session().map(|s| s.id.clone());
            match active_id {
                Some(sid) => self
                    .session_manager
                    .switch_session(&sid)
                    .map(|s| s.id.clone())
                    .unwrap_or_else(|_| {
                        self.session_manager
                            .new_session(&format!(
                                "chat-{}",
                                chrono::Utc::now().format("%Y%m%d-%H%M%S")
                            ))
                            .id
                            .clone()
                    }),
                None => self
                    .session_manager
                    .new_session(&format!(
                        "chat-{}",
                        chrono::Utc::now().format("%Y%m%d-%H%M%S")
                    ))
                    .id
                    .clone(),
            }
        };
        self.session_id = Some(sid.clone());

        self.context_engine.lock().unwrap().on_session_start(
            &sid, "default", None,
        );

        match &self.call_mode {
            None => { self.call_mode = Some(CallMode::Fresh(sid.clone())); }
            Some(mode) => {
                if matches!(mode, CallMode::Fresh(_)) {
                    self.call_mode = Some(CallMode::Incremental(sid.clone()));
                }
            }
        }
        let call_mode = self.call_mode.as_ref().unwrap().clone();
        let input_msg = Message::user(input);

        // ── Phase 2: async work via ConversationLoop ────────────
        self.conversation.preflight_check(&mut self.session_manager, &sid).await?;

        let response = if stream {
            self.conversation
                .run_turn_with_streaming(&mut self.session_manager, &sid, input_msg, &call_mode, delta_callback)
                .await?
        } else {
            self.conversation
                .run_turn(&mut self.session_manager, &sid, input_msg, &call_mode)
                .await?
        };

        self.session_manager.save(None)?;

        Ok(response)
    }

    pub fn continue_session(&mut self, key: &str) -> Result<String> {
        self.session_manager.init()?;
        let sid = self.session_manager.find_key(key)
            .ok_or_else(|| anyhow::anyhow!("Session not found: {}. Run `oben sessions list` to see available sessions.", key))?;

        self.session_manager.switch_session(&sid)?;
        self.session_id = Some(sid.clone());
        self.call_mode = Some(CallMode::Fresh(sid.clone()));

        let name = self.session_manager.active_session().map(|s| s.name.clone()).unwrap_or(key.to_string());
        self.context_engine.lock().unwrap().on_session_start(&sid, "default", None);
        Ok(name)
    }

    pub fn reset(&mut self) -> Result<()> {
        self.context_engine.lock().unwrap().on_session_reset();
        if let Some(session) = self.session_manager.active_session_mut() {
            session.messages.clear();
        }
        Ok(())
    }

    pub fn conversation(&mut self) -> &mut DefaultConversationLoop {
        &mut self.conversation
    }
}
