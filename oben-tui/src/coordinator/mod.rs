//! TUI conversation coordinator.
//!
//! Drives the TUI turn lifecycle as a pure compute actor. It holds no reference
//! to `Arc<App>`, sending `TuiCommand` messages via `command_tx` instead, and
//! the event loop applies them to the exclusive copy of `Arc<App>`.
//! This eliminates the deadlock / render-storm where coordinator and event
//! loop contended for the same mutex.

use super::app::TurnCompletion;
use anyhow::{anyhow, Result};
use std::sync::Arc;
use tracing::info;
use oben_agent::coordinator::{ConversationCoordinator, ConversationResult};
use oben_models::{Message, MessageContent, MessagePart, SessionManager};

// ── TUI Commands ──────────────────────────────────────────────────────────

/// Commands from the TUI coordinator to the event loop.
/// The coordinator is a pure compute actor — it NEVER locks `Arc<App>`.
/// Instead it sends `TuiCommand` messages via `command_tx`, and the event
/// loop (which owns `Arc<App>`) applies them.
pub enum TuiCommand {
    /// Signal the event loop to prepare ChatPanel for streaming.
    StartTurn {
        input: String,
        session_name: Option<String>,
    },
    /// Append the user input to the session history.
    AppendInputHistory {
        input: String,
    },
}

// ── TUI Coordinator ──────────────────────────────────────────────────────

/// TUI state consumed once during `run()` into `drive()`.
/// Fields are wrapped in `Option` so the trait impl can drain them.
pub struct TuiCoordinator {
    command_tx: Option<tokio::sync::mpsc::UnboundedSender<TuiCommand>>,
    chat_rx: Option<tokio::sync::mpsc::UnboundedReceiver<String>>,
    done_tx: Option<tokio::sync::mpsc::UnboundedSender<TurnCompletion>>,
    interrupt_state: Option<Arc<oben_agent::interrupt::InterruptState>>,
    /// Arc to the agent — stored to allow locking for turns.
    agent: Option<Arc<tokio::sync::Mutex<oben_agent::Agent>>>,
    config: Option<oben_config::AppConfig>,
    _call_mode: Option<oben_models::CallMode>,
}

impl TuiCoordinator {
    /// Run the coordinator directly, without going through
    /// `Agent::run()`. This avoids the unnecessary
    /// `Arc<Mutex<Agent>>` lock that the `ConversationCoordinator`
    /// trait path requires (`agent.lock().await`).
    pub async fn run_tui(self) -> Result<ConversationResult> {
        self.drive().await
    }

    pub fn new(
        command_tx: tokio::sync::mpsc::UnboundedSender<TuiCommand>,
        chat_rx: tokio::sync::mpsc::UnboundedReceiver<String>,
        done_tx: tokio::sync::mpsc::UnboundedSender<TurnCompletion>,
        interrupt_state: Arc<oben_agent::interrupt::InterruptState>,
        agent: Arc<tokio::sync::Mutex<oben_agent::Agent>>,
        config: oben_config::AppConfig,
    ) -> Self {
        Self {
            command_tx: Some(command_tx),
            chat_rx: Some(chat_rx),
            done_tx: Some(done_tx),
            interrupt_state: Some(interrupt_state),
            agent: Some(agent),
            config: Some(config),
            _call_mode: None,
        }
    }

    async fn send(&self, cmd: TuiCommand) {
        let _ = self.command_tx.as_ref().unwrap().send(cmd);
    }

    pub async fn drive(mut self) -> Result<ConversationResult> {
        let hook_engine = self
            .agent
            .as_ref()
            .unwrap()
            .lock()
            .await
            .hooks()
            .clone();

        hook_engine.emit_loop_start();

        loop {
            hook_engine.emit_pre_turn();

            let input = match self.chat_rx.as_mut().unwrap().recv().await {
                Some(text) => text,
                None => {
                    tracing::info!("coordinator: chat channel closed, exiting");
                    hook_engine.emit_loop_end("chat_channel_closed");
                    return Ok(ConversationResult::Exit);
                }
            };

            // Notify event loop before agent.turn — UI must be ready for streaming callbacks.
            self.send(TuiCommand::StartTurn { input: input.clone(), session_name: None })
                .await;

            let input_msg = build_image_message(&input);
            let interrupt_clone = Arc::clone(self.interrupt_state.as_ref().unwrap());

            let response = {
                let mut agent = self.agent.as_ref().unwrap().lock().await;
                if matches!(input_msg.content, MessageContent::Image { .. } | MessageContent::Parts(_)) {
                    agent.turn_with_message(input_msg, Some(Arc::clone(&interrupt_clone)))
                        .await
                } else {
                    agent.turn(&input, true, Some(Arc::clone(&interrupt_clone)))
                        .await
                }
            };

            let _ = self.interrupt_state.as_ref().unwrap().drain_interrupt_message();

            let (turn_session_name, messages_from_agent) = {
                let agent = self.agent.as_ref().unwrap().lock().await;
                let sm_arc = agent.session_manager();
                let sm = sm_arc.lock().await;
                let sid = agent.context_window_manager().session_id();
                let name = sid.as_ref().and_then(|s| sm.session(s)).map(|s| s.name.clone());
                let msgs = sid.and_then(|sid| sm.session(&sid)).map(|s| s.messages.clone()).unwrap_or_default();
                (name, msgs)
            };

            if let Err(e) = response {
                let err_str = format!("Turn error: {}", e);
                hook_engine.emit_loop_end(&err_str);
                let completion = TurnCompletion {
                    success: false,
                    status: err_str.clone(),
                    session_name: turn_session_name.clone(),
                    messages: messages_from_agent,
                };
                let _ = self.done_tx.as_ref().unwrap().send(completion);
                return Err(anyhow!(err_str));
            }

            let msg_roles_coordinator: Vec<String> = messages_from_agent.iter()
                .map(|m| format!("{:?}", m.role)).collect();
            let msg_preview: Vec<String> = messages_from_agent.iter()
                .filter_map(|m| match &m.content {
                    MessageContent::Text(t) => Some(t.chars().take(40).collect::<String>()),
                    _ => None,
                }).collect();
            info!("[coordinator/turn_done] sending completion success=true session_name=? msgs={} roles={:?} previews={:?}",
                messages_from_agent.len(), msg_roles_coordinator, msg_preview);
            let final_text = messages_from_agent.last().and_then(|m| {
                if let MessageContent::Text(ref t) = m.content { Some(t.clone()) } else { None }
            }).unwrap_or_default();
            hook_engine.emit_turn_complete(&final_text, messages_from_agent.len());

            let completion = TurnCompletion {
                success: true,
                status: String::new(),
                session_name: turn_session_name.clone(),
                messages: messages_from_agent,
            };
            let _ = self.done_tx.as_ref().unwrap().send(completion);

            self.send(TuiCommand::AppendInputHistory { input }).await;
        }
    }
}

#[::async_trait::async_trait]
impl ConversationCoordinator for TuiCoordinator {
    async fn run(
        &mut self,
        context_window_manager: &mut dyn oben_agent::context::ContextWindowManager,
        transport: Arc<dyn oben_models::providers::TransportProvider + Send + Sync>,
        tools: Arc<oben_tools::ToolRegistry>,
        session_manager: &mut dyn oben_models::SessionManager,
    ) -> Result<ConversationResult> {
        // Drop unused params to avoid temporary lifetime issues.
        drop((context_window_manager, transport, tools, session_manager));

        // Drain all fields so drive() can own them.
        let this = TuiCoordinator {
            command_tx: Some(self.command_tx.take().unwrap()),
            chat_rx: Some(self.chat_rx.take().unwrap()),
            done_tx: Some(self.done_tx.take().unwrap()),
            interrupt_state: Some(self.interrupt_state.take().unwrap()),
            agent: Some(self.agent.take().unwrap()),
            config: None, // drive() doesn't use config
            _call_mode: None,
        };
        this.drive().await
    }

    fn send_message(
        &self,
        _text: String,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        Box::pin(async {})
    }

    fn request_interrupt(
        &self,
        _message: Option<String>,
    ) {
    }
}

// ── Image Helpers ─────────────────────────────────────────────────────────

fn build_image_message(input: &str) -> Message {
    let known_exts = [".jpg", ".jpeg", ".png", ".gif", ".webp", ".svg", ".bmp", ".tiff", ".tif", ".ico", ".avif"];
    let tokens: Vec<&str> = input.split_whitespace().collect();
    let mut image_tokens: Vec<String> = Vec::new();
    let mut text_tokens: Vec<&str> = Vec::new();

    for token in tokens {
        let is_image = known_exts.iter().any(|ext| token.to_lowercase().ends_with(ext));
        if is_image && token.starts_with('/') {
            if let Some((msg, _)) = crate::image::path_to_image_message(token, "") {
                match &msg.content {
                    MessageContent::Image { url, .. } => {
                        if !url.is_empty() { image_tokens.push(url.clone()); }
                    }
                    MessageContent::Parts(parts) => {
                        for p in parts {
                            if let MessagePart::Image { url, .. } = p {
                                if !url.is_empty() { image_tokens.push(url.clone()); }
                            }
                        }
                    }
                    _ => {}
                }
            }
        } else {
            text_tokens.push(token);
        }
    }

    if !image_tokens.is_empty() {
        let text: String = text_tokens.join(" ");
        let text_trimmed = text.trim();

        if text_trimmed.is_empty() && image_tokens.len() == 1 {
            return Message {
                role: oben_models::MessageRole::User,
                content: MessageContent::Image { url: image_tokens[0].clone(), detail: None },
                id: None, tool_call_ids: vec![], tool_calls: None, reasoning: None,
            };
        }

        let mut parts: Vec<MessagePart> = vec![];
        if !text_trimmed.is_empty() { parts.push(MessagePart::Text(text_trimmed.to_string())); }
        for url in &image_tokens { parts.push(MessagePart::Image { url: url.clone(), detail: None }); }

        return Message {
            role: oben_models::MessageRole::User,
            content: MessageContent::Parts(parts),
            id: None, tool_call_ids: vec![], tool_calls: None, reasoning: None,
        };
    }

    Message::user(input)
}
