//! Interactive chat session — shared between CLI and TUI.
//!
//! Wraps a `SessionManager` and `ConversationLoop`, handling:
//! - Lazy session creation (only on first input)
//! - Session switching / loading
//! - Call mode management (Fresh → Incremental)
//! - Session save after each turn
//!
//! Neither CLI nor TUI concerns are included — this is pure business logic.

use anyhow::Result;
use tracing::info;
use oben_models::{CallMode, Message, StreamDeltaCallback};
use oben_sessions::SessionManager;

use crate::ConversationLoop;

/// Configuration for building a `ChatSession`.
///
/// Carries everything needed from `oben-cli` / `oben-tui` to construct
/// the underlying `ConversationLoop` and `SessionManager`.
pub struct ChatSessionConfig {
    /// Pre-built system prompt text (assembled by the caller).
    pub system_prompt_text: String,
    /// Transport for LLM calls.
    pub transport: oben_transport::ChatCompletionsTransport,
    /// Registered tools.
    pub tools: std::sync::Arc<oben_tools::ToolRegistry>,
    /// Max iteration budget per turn.
    pub max_iterations: usize,
    /// Max messages in context.
    pub max_messages: usize,
}

/// An interactive chat session that owns session lifecycle and conversation.
///
/// Both the CLI `run_chat` and the TUI event loop use this struct.
/// It does not touch stdin/stdout — those are handled by the caller.
#[allow(dead_code)]
pub struct ChatSession {
    conversation: ConversationLoop,
    memory: SessionManager,
    system_prompt_text: String,
    session_id: Option<String>,
    call_mode: Option<CallMode>,
}

impl ChatSession {
    /// Create a new chat session from the given configuration.
    ///
    /// Tries to load the currently active session immediately. If one exists,
    /// it will be loaded and returned so the caller can display session info
    /// before the interactive loop. If no active session exists, returns None
    /// and the session will be created lazily on the first user input.
    pub fn new(config: ChatSessionConfig) -> Result<Self> {
        let mut session = Self {
            conversation: ConversationLoop::new(
                config.transport,
                config.tools,
                config.max_iterations,
                config.max_messages,
            ),
            memory: SessionManager::new()?,
            system_prompt_text: config.system_prompt_text,
            session_id: None,
            call_mode: None,
        };
        // Try to load any pre-existing active session right away
        session.eager_load_active_session();
        Ok(session)
    }

    /// Return the name of the session that was loaded by `new()`.
    /// Returns None if no active session was loaded (lazy creation pending).
    pub fn loaded_session_name(&self) -> Option<String> {
        self.session_id.as_ref().and_then(|sid| {
            self.memory.active_session()
                .filter(|s| s.id == *sid)
                .map(|s| s.name.clone())
        })
    }

    /// Try to load the currently active session (without creating a new one).
    fn eager_load_active_session(&mut self) {
        if let Some(active) = self.memory.active_session() {
            let sid = active.id.clone();
            let _ = self.memory.switch_session(&sid);
            let _ = self.memory.load(Some(&sid));
            self.session_id = Some(sid);
            self.call_mode = Some(CallMode::Fresh(self.session_id.clone().expect("session id not set after loading active session")));
        }
    }

    /// Return the current session ID (None before first input or no active session).
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    /// Access the underlying session manager for read-only ops.
    pub fn session_manager(&self) -> &SessionManager {
        &self.memory
    }

    /// Access the underlying session manager for write ops.
    pub fn session_manager_mut(&mut self) -> &mut SessionManager {
        &mut self.memory
    }

    /// Execute one conversation turn.
    ///
    /// **Lazy session creation:** the first call creates or switches to an
    /// active session. Subsequent calls reuse it.
    ///
    /// If `stream` is true and `delta_callback` is provided, text tokens
    /// are forwarded to the callback as they arrive.
    ///
    /// Returns the full response text.
    pub async fn turn(
        &mut self,
        input: &str,
        stream: bool,
        delta_callback: Option<StreamDeltaCallback>,
    ) -> Result<String> {
        // ── Lazy session creation (first input only) ──────────────
        let sid = self.session_id.get_or_insert_with(|| {
            let active_id = self.memory.active_session().map(|s| s.id.clone());
            match active_id {
                Some(sid) => self
                    .memory
                    .switch_session(&sid)
                    .map(|s| s.id.clone())
                    .unwrap_or_else(|_| {
                        self.memory
                            .new_session(&format!(
                                "chat-{}",
                                chrono::Utc::now().format("%Y%m%d-%H%M%S")
                            ))
                            .id
                            .clone()
                    }),
                None => self
                    .memory
                    .new_session(&format!(
                        "chat-{}",
                        chrono::Utc::now().format("%Y%m%d-%H%M%S")
                    ))
                    .id
                    .clone(),
            }
        });

        // ── Load & set call mode ─────────────────────────────────
        self.memory.load(Some(sid))?;

        match &self.call_mode {
            None => {
                self.call_mode = Some(CallMode::Fresh(sid.clone()));
            }
            Some(mode) => {
                if matches!(mode, CallMode::Fresh(_)) {
                    self.call_mode = Some(CallMode::Incremental(sid.clone()));
                }
            }
        }
        let call_mode = self.call_mode.as_ref().unwrap();

        // ── Preflight check — compress if session already over threshold ──
        let passes = self.conversation.preflight_check(
            &mut self.memory.active_session_mut().unwrap().messages,
        ).await;
        if let Ok(n) = passes {
            if n > 0 {
                info!("Preflight: {} compression pass(es) before turn", n);
            }
        }

        // ── Execute turn ─────────────────────────────────────────
        let response = if stream {
            self.conversation
                .run_turn_with_streaming(
                    &mut self.memory.active_session_mut().unwrap().messages,
                    Message::user(input),
                    call_mode,
                    delta_callback,
                )
                .await?
        } else {
            self.conversation
                .run_turn(
                    &mut self.memory.active_session_mut().unwrap().messages,
                    Message::user(input),
                    call_mode,
                )
                .await?
        };

        // ── Persist ──────────────────────────────────────────────
        self.memory.save(None)?;

        Ok(response)
    }

    /// Switch to an existing session by ID or name (load messages & set Fresh mode).
    ///
    /// Returns the session name if found, or an error if the session does not exist.
    pub fn continue_session(&mut self, key: &str) -> Result<String> {
        // Load all sessions into the in-memory cache first.
        self.memory.init()?;

        // Resolve name → UUID.
        let sid = self.memory.find_key(key)
            .ok_or_else(|| anyhow::anyhow!("Session not found: {}. Run `oben sessions list` to see available sessions.", key))?;

        self.memory.switch_session(&sid)?;
        self.memory.load(Some(&sid))?;
        self.session_id = Some(sid.clone());
        self.call_mode = Some(CallMode::Fresh(sid.clone()));

        let name = self.memory.active_session().map(|s| s.name.clone()).unwrap_or(key.to_string());
        Ok(name)
    }

    /// Access the underlying conversation loop (for compaction, etc.).
    pub fn conversation(&mut self) -> &mut ConversationLoop {
        &mut self.conversation
    }
}
