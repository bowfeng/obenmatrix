//! Agent — shared between CLI and TUI.
//!
//! Wraps a `SessionManager` and `ConversationLoop`, handling:
//! - Lazy session creation (only on first input)
//! - Session switching / loading
//! - Call mode management (Fresh → Incremental)
//! - Session save after each turn
//!
//! Neither CLI nor TUI concerns are included — this is pure business logic.

use std::io::Write;
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

/// Callbacks passed to `interactive_chat` for flexible I/O.
///
/// Implement this to plug in custom input/output sources (e.g. TUI panels,
/// network sockets, test fixtures). The default CLI implementation uses
/// stdin/stdout.
pub struct ChatCallbacks {
    /// Print a session info line (e.g. "Continuing session: xxx").
    pub print_info: fn(&str),
    /// Print the prompt before reading input (e.g. "> ").
    pub print_prompt: fn(),
    /// Print a flush hint (optional, for custom streams).
    pub print_flush: fn(),
    /// Read the next user input line. Return `None` to exit.
    pub read_input: fn() -> Option<String>,
    /// Print a newline after response.
    pub print_newline: fn(),
    /// Exit condition callback. Return `true` to exit the loop.
    /// Called on each input before sending to the agent.
    pub should_exit: fn(&str) -> bool,
}

/// Default CLI callbacks.
impl ChatCallbacks {
    pub fn for_cli() -> Self {
        Self {
            print_info: |msg: &str| println!("{}", msg),
            print_prompt: || print!("> "),
            print_flush: || {
                let _ = std::io::stdout().flush();
            },
            read_input: || {
                let mut input = String::new();
                if std::io::stdin().read_line(&mut input).is_ok() {
                    Some(input.trim().to_string())
                } else {
                    Some(String::new())
                }
            },
            print_newline: || println!(),
            should_exit: |input: &str| input == "quit" || input == "exit",
        }
    }
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

    /// Run an interactive chat loop with custom I/O callbacks.
    ///
    /// This is the generic entry point used by both CLI and TUI.
    /// The caller provides `ChatCallbacks` to abstract stdin/stdio,
    /// or calls `turn()` directly from a custom event loop (TUI).
    ///
    /// **Example — custom TUI integration:**
    /// ```ignore
    /// agent.interactive_chat(ChatCallbacks {
    ///     read_input: || get_tui_input(),   // custom input source
    ///     should_exit: |_| false,           // TUI owns exit
    ///     ..ChatCallbacks::for_cli()
    /// }).await;
    /// ```
    pub async fn interactive_chat(&mut self, stream: bool, continue_with: Option<&str>, callbacks: ChatCallbacks) -> Result<()> {
        // ── Session continuation / display ──────────────────────
        if let Some(key) = continue_with {
            let resolved_key = if key == "latest" {
                self.session_manager().active_session()
                    .map(|s| s.name.clone()).unwrap_or_else(|| key.to_string())
            } else {
                key.to_string()
            };
            let name = self.continue_session(&resolved_key)?;
            if let Some(s) = self.session_manager().active_session() {
                let msg_count = s.messages.len();
                (callbacks.print_info)(&format!("Continuing session: {} ({} messages)\n", name, msg_count));
                print_session_messages(&s.messages, 10);
                (callbacks.print_info)("");
            }
        } else {
            if let Some(name) = self.loaded_session_name() {
                if let Some(s) = self.session_manager().active_session() {
                    (callbacks.print_info)(&format!("Session: {} ({} messages)\n", name, s.messages.len()));
                }
            }
        }
        (callbacks.print_info)("🦀 ObenAgent ready. Type 'quit' or 'exit' to stop.\n");
        (callbacks.print_flush)();

        // ── Interactive loop ────────────────────────────────────
        loop {
            (callbacks.print_prompt)();
            (callbacks.print_flush)();

            let input = match (callbacks.read_input)() {
                Some(line) if !line.trim().is_empty() => line.trim().to_string(),
                _ => continue,
            };

            if (callbacks.should_exit)(&input) { break; }

            let response = self.turn(&input, stream, stream.then(move || {
                Box::new(move |text: &str| {
                    print!("{}", text);
                    (callbacks.print_flush)();
                }) as StreamDeltaCallback
            })).await?;
            if stream {
                (callbacks.print_newline)();
            } else {
                (callbacks.print_info)(&format!("\n{}", response));
                (callbacks.print_flush)();
            }
        }

        Ok(())
    }
}

fn print_session_messages(messages: &[oben_models::Message], max_show: usize) {
    if messages.is_empty() {
        println!("(no messages)");
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
        let display = if text.len() > 120 {
            format!("{}...", &text[..117])
        } else {
            text.to_string()
        };
        println!("  {} {}", role, display);
    }

    if overflow > 0 {
        println!("  ... {} more messages", overflow);
    }
}
