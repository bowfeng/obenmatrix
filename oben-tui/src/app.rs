//! Application state and core logic.
use crate::commands;
use crate::history;
use crate::panels::chat::ChatPanel;
use crate::panels::config::ConfigPanel;
use crate::panels::sessions::SessionsPanel;
use crate::panels::setup::SetupPanel;
use crate::panels::splash::SplashPanel;
use crate::panels::{KeyAction, Panel, PanelId};
use anyhow::Result;
use parking_lot::Mutex as PlMutex;
use ratatui::layout::Rect;
use ratatui_toaster::{ToastBuilder, ToastEngine, ToastEngineBuilder, ToastType};
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tracing::info;
use oben_config::AppConfig;
use super::shared::SharedAgentState;

/// Payload carried by TurnDone completion event from spawned task.
pub struct TurnCompletion {
    pub success: bool,
    pub status: String,
    pub session_name: Option<String>,
    pub message_count: usize,
}

pub struct App {
    pub running: bool,
    pub active_panel: PanelId,
    pub panels: HashMap<PanelId, Box<dyn Panel>>,
    pub status: String,
    pub config: AppConfig,
    /// Timestamp when splash was first created. Used to enforce minimum 5s display.
    pub splash_started: std::time::Instant,
    /// Pending session name to load on startup (from CLI `-s` argument).
    pub pending_session: Option<String>,
    /// Whether step-by-step reasoning mode is enabled.
    pub reasoning_enabled: bool,
    /// Temporary toast notification engine.
    /// Commands (clear, new, reasoning, etc.) show toasts instead of
    /// persistent status-bar text. Status bar keeps using `status`
    /// for persistent state (Streaming, Ready, Error).
    pub toast_engine: ToastEngine<()>,
    /// Timestamp after which the current toast should expire.
    /// Used because ratatui-toaster has no built-in auto-dismiss.
    pub toast_expires_at: Option<std::time::Instant>,
    /// Set to true by event handlers and toast expiry check.
    /// Cleared by draw_ui after rendering. When false, the 32ms timer
    /// skips the terminal.draw() call entirely during idle.
    pub needs_redraw: bool,
    /// Shared agent state (agent, tools, session data).
    pub shared_state: Arc<PlMutex<super::shared::SharedAgentState>>,
    /// Sender for TuiEvents to the event loop.
    pub input_tx: Option<tokio::sync::mpsc::UnboundedSender<crate::TuiEvent>>,
    /// Input history for arrow key navigation.
    pub input_history: history::InputHistory,
    /// Whether paste mode is enabled.
    pub paste_mode: bool,
}

impl App {
    /// Downcast the stored ChatPanel — reads.
    pub(super) fn get_chat(&self) -> Option<&ChatPanel> {
        self.panels
            .get(&PanelId::Chat)
            .and_then(|p| p.downcast_ref::<ChatPanel>())
    }

    /// Downcast the stored ChatPanel — mutates.
    pub(super) fn get_chat_mut(&mut self) -> Option<&mut ChatPanel> {
        self.panels
            .get_mut(&PanelId::Chat)
            .and_then(|p| p.downcast_mut::<ChatPanel>())
    }

    /// Downcast the stored SessionsPanel — reads.
    pub(super) fn get_sessions(&self) -> Option<&SessionsPanel> {
        self.panels
            .get(&PanelId::Sessions)
            .and_then(|p| p.downcast_ref::<SessionsPanel>())
    }
    /// Execute a TUI command by name. Matches the command name directly
    /// to avoid borrow conflicts between registry access and app mutation.
    pub async fn execute_command(&mut self, name: &str) {
        match name {
            "clear" => {
                let has_turn = {
                    let ss = self.shared_state.lock();
                    ss.turn_handle.is_some()
                };
                if has_turn {
                    self.show_toast(
                        "Cannot clear: turn in progress",
                        ratatui_toaster::ToastType::Error,
                    );
                    return;
                }
                tracing::info!("[clear] START");
                {
                    let ss_arc = Arc::clone(&self.shared_state);
                    let ss = ss_arc.lock();
                    if let Some(ref agent) = ss.agent {
                        let result = agent.lock().await.reset().await;
                        if let Err(e) = result {
                            self.show_toast(
                                format!("Clear failed: {e}"),
                                ratatui_toaster::ToastType::Error,
                            );
                            return;
                        }
                    }
                }
                let chat_panel_present = self.panels.contains_key(&PanelId::Chat);
                tracing::info!("[clear] panels contain Chat key: {}", chat_panel_present);
                tracing::info!(
                    "[clear] all panel keys: {:?}",
                    self.panels.keys().collect::<Vec<_>>()
                );
                if chat_panel_present {
                    let panel_type = self.panels.get(&PanelId::Chat).map(|p| {
                        if p.downcast_ref::<ChatPanel>().is_some() {
                            "ChatPanel"
                        } else if p.downcast_ref::<SessionsPanel>().is_some() {
                            "SessionsPanel"
                        } else if p.downcast_ref::<ConfigPanel>().is_some() {
                            "ConfigPanel"
                        } else if p.downcast_ref::<SetupPanel>().is_some() {
                            "SetupPanel"
                        } else {
                            "unknown"
                        }
                    });
                    tracing::info!("[clear] Chat panel type: {:?}", panel_type);
                }
                if let Some(chat) = self.get_chat_mut() {
                    let streaming_before = chat.streaming;
                    let count_before = chat.message_count;
                    tracing::info!(
                        "[clear] chat: streaming={}, message_count={}",
                        streaming_before,
                        count_before
                    );
                    chat.clear_display();
                    tracing::info!("[clear] chat clear_display done");
                    // Also reset streaming flag — prevents 32ms redraw loop from repopulating
                    chat.streaming = false;
                    tracing::info!("[clear] chat streaming set to false");
                }
                // Clear turn state to prevent stale streaming text from being redrawn
                {
                    let ss = self.shared_state.lock();
                    let mut ts = ss.turn_state.lock();
                    ts.reset();
                }
                tracing::info!("[clear] turn state cleared");
                {
                    let mut ss = self.shared_state.lock();
                    ss.session_id = None;
                }
                // Refresh SessionsPanel so cleared session list stays in sync.
                if let Some(sp) = self
                    .panels
                    .get_mut(&PanelId::Sessions)
                    .and_then(|p| p.downcast_mut::<crate::panels::sessions::SessionsPanel>())
                {
                    sp.refresh_list(None).await;
                }
                // Display info message in conversation so user sees "Session cleared"
                if let Some(chat) = self.get_chat_mut() {
                    chat.append_info_message(
                        "Session cleared. A new session will be created on the next turn.",
                    );
                }
                self.show_toast("Session cleared.", ratatui_toaster::ToastType::Success);
                tracing::info!("[clear] DONE");
            }
            "compact" => {
                let has_agent = self.shared_state.lock().agent.is_some();
                let has_turn = self.shared_state.lock().turn_handle.is_some();
                if !has_agent {
                    self.show_toast(
                        "Cannot compact: agent not initialized",
                        ratatui_toaster::ToastType::Error,
                    );
                    return;
                }
                if has_turn {
                    self.show_toast(
                        "Cannot compact: turn in progress",
                        ratatui_toaster::ToastType::Warning,
                    );
                    return;
                }
                if let Some(chat) = self.get_chat_mut() {
                    chat.streaming = true;
                }
                self.show_toast(
                    "Compacting session context...",
                    ratatui_toaster::ToastType::Info,
                );
                if let Some(tx) = &self.input_tx {
                    let _ = tx.send(crate::TuiEvent::CompactSession);
                }
            }
            "new" => {
                let has_turn = self.shared_state.lock().turn_handle.is_some();
                if has_turn {
                    self.show_toast(
                        "Cannot create new session: turn in progress",
                        ratatui_toaster::ToastType::Error,
                    );
                    return;
                }
                {
                    let ss_arc = Arc::clone(&self.shared_state);
                    let ss = ss_arc.lock();
                    if let Some(agent) = &ss.agent {
                        let result = {
                            let mut guard = agent.lock().await;
                            guard.new_session().await
                        };
                        match result {
                            Ok(new_id) => {
                                tracing::info!("[new] created session: {}", new_id);
                                if let Some(chat) = self.get_chat_mut() {
                                    // Set the new session (empty messages)
                                    chat.session_name = Some(new_id.clone());
                                    chat.message_count = 0;
                                    chat.clear_display();
                                }
                                // Refresh SessionsPanel so the new session appears in the list.
                                if let Some(sp) =
                                    self.panels.get_mut(&PanelId::Sessions).and_then(|p| {
                                        p.downcast_mut::<crate::panels::sessions::SessionsPanel>()
                                    })
                                {
                                    sp.refresh_list(None).await;
                                }
                            }
                            Err(e) => {
                                self.show_toast(
                                    format!("New session failed: {e}"),
                                    ratatui_toaster::ToastType::Error,
                                );
                                return;
                            }
                        }
                    }
                }
                self.show_toast("New session started.", ratatui_toaster::ToastType::Success);
            }
            "quit" => {
                self.running = false;
            }
            "reasoning" => {
                self.reasoning_enabled = !self.reasoning_enabled;
                let msg = if self.reasoning_enabled {
                    "Reasoning mode: ON"
                } else {
                    "Reasoning mode: OFF"
                };
                self.show_toast(msg, ratatui_toaster::ToastType::Info);
            }
            "rename" => {
                self.show_toast(
                    "Usage: /rename [new_name]",
                    ratatui_toaster::ToastType::Error,
                );
            }
            "theme" => {
                self.show_toast(
                    "Press Ctrl+T to cycle themes",
                    ratatui_toaster::ToastType::Info,
                );
            }
            "help" => {
                if let Some(chat) = self.get_chat_mut() {
                    chat.append_info_message(
                        "Slash commands: /clear /compact /new /quit /reasoning /session [name] /rename [name]\n\
                         Keyboard: Up/Down=history, Ctrl+A/E=home/end, Ctrl+W=delete word, Ctrl+K=kill line",
                    );
                }
                self.show_toast(
                    "Help displayed in conversation.",
                    ratatui_toaster::ToastType::Info,
                );
            }
            "todo" => {
                self.show_toast("TODO: No pending tasks.", ratatui_toaster::ToastType::Info);
            }
            _ => {}
        }
    }

    /// Activate a panel and call its on_activate/on_deactivate hooks.
    pub async fn activate_panel(&mut self, panel: PanelId) {
        let old = self.active_panel;
        tracing::info!("[panel] Activating {:?} (was {:?})", panel, old);

        // Deactivate old panel
        if old != panel {
            if let Some(mut boxed_panel) = self.panels.remove(&old) {
                boxed_panel.on_deactivate(self).await;
                self.panels.insert(old, boxed_panel);
            }
        }

        self.active_panel = panel;

        // Activate new panel
        if let Some(mut boxed_panel) = self.panels.remove(&panel) {
            if panel == PanelId::Chat {
                if let Some(s) = boxed_panel.downcast_mut::<ChatPanel>() {
                    let ss_arc = Arc::clone(&self.shared_state);
                    let ss = ss_arc.lock();
                    let agent_initialized = ss.agent.is_some();
                    let session_name = ss.active_session_name();
                    drop(ss);
                    if agent_initialized {
                        let messages = self.loaded_session_messages_safe().await;
                        let msg_roles: Vec<String> = messages.iter().map(|m| format!("{:?}", m.role)).collect();
                        tracing::info!(
                            "[activate_panel/Chat] session_name={:?} messages={:?} message_count={} roles={:?}",
                            session_name,
                            messages.iter().filter_map(|m| match &m.content {
                                oben_models::MessageContent::Text(t) => Some(t.chars().take(30).collect::<String>()),
                                _ => None,
                            }).collect::<Vec<_>>(),
                            messages.len(),
                            msg_roles,
                        );
                        s.set_session_data(session_name, messages);
                    }
                }
            }
            boxed_panel.on_activate(self).await;
            self.panels.insert(panel, boxed_panel);
        }
    }

    pub fn new() -> Result<Self> {
        let config = AppConfig::load(None)?;
        // Placeholder toast engine — area is set dynamically in draw_ui.
        let toast_engine = ToastEngine::new(ToastEngine::<()>::from_builder(
            ToastEngineBuilder::<()>::new(Rect::new(0, 0, 0, 0))
                .default_duration(Duration::from_secs(3)),
        ));
        // Insert Splash panel — shown during agent initialization
        let mut panels: HashMap<PanelId, Box<dyn Panel>> = HashMap::new();
        panels.insert(PanelId::Splash, Box::new(SplashPanel::new()));

        Ok(Self {
            running: true,
            active_panel: PanelId::Splash,
            panels,
            splash_started: std::time::Instant::now(),
            status: String::new(),
            config,
            input_tx: None,
            shared_state: Arc::new(PlMutex::new(
                crate::shared::SharedAgentState::new_empty(),
            )),
            input_history: history::InputHistory::new(),
            paste_mode: false,
            pending_session: None,
            reasoning_enabled: false,
            toast_engine,
            toast_expires_at: None,
            needs_redraw: true,
        })
    }

    /// Show a temporary toast notification.
    pub fn show_toast<S: Into<Cow<'static, str>>>(&mut self, msg: S, toast_type: ToastType) {
        let builder = ToastBuilder::new(msg.into()).toast_type(toast_type);
        self.toast_engine.show_toast(builder);
        self.toast_expires_at = Some(std::time::Instant::now() + Duration::from_secs(2));
    }

    pub async fn init_agent(&mut self) -> Result<()> {
        let shared = SharedAgentState::init(&self.config).await?;
        let mut ss = self.shared_state.lock();
        ss.agent = shared.agent;
        ss.turn_message_count = shared.turn_message_count;
        ss.turn_handle = shared.turn_handle;
        ss.session_id = shared.session_id;
        ss.tools = shared.tools;
        ss.tool_names = shared.tool_names;
        ss.turn_state = shared.turn_state;
        Ok(())
    }

    pub fn begin_turn(&self) {
        let ss = self.shared_state.lock();
        let mut ts = ss.turn_state.lock();
        ts.on_turn_start();
    }

    pub fn finalize_turn(&self, outcome: &str) {
        let ss = self.shared_state.lock();
        let mut ts = ss.turn_state.lock();
        ts.on_completed(outcome);
    }

    pub async fn create_chat_panel(&mut self) -> Result<()> {
        tracing::info!("[panel] Creating ChatPanel");
        let theme = self.config.display.theme.clone();
        // CRITICAL: use shared_state.turn_state (written by adapters) not
        // AppState::turn_state (a stale empty TurnState from App::new).
        let state_ref = Arc::clone(&self.shared_state.lock().turn_state);
        tracing::info!("[panel] Using theme: {}", theme);
        let mut panel = ChatPanel::new_with_theme(None, &theme, state_ref);
        // Wire the shared state ref for subagent rendering.
        panel.set_shared_state_ref(Arc::clone(&self.shared_state));
        self.panels.insert(
            PanelId::Chat,
            Box::new(panel),
        );
        tracing::info!(
            "[panel] ChatPanel inserted, panels={:?}",
            self.panels.keys().collect::<Vec<_>>()
        );
        Ok(())
    }

    pub async fn create_sessions_panel(&mut self) -> Result<()> {
        tracing::info!("[panel] Creating SessionsPanel");
        if let Some(agent) = &self.shared_state.lock().agent {
            self.panels.insert(
                PanelId::Sessions,
                Box::new(SessionsPanel::new_shared(Arc::clone(agent))),
            );
        } else {
            tracing::warn!("[panel] No agent, using fallback");
            self.panels
                .insert(PanelId::Sessions, Box::new(SessionsPanel::new_empty()));
        }
        tracing::info!(
            "[panel] SessionsPanel inserted, panels={:?}",
            self.panels.keys().collect::<Vec<_>>()
        );
        Ok(())
    }

    pub async fn handle_key(&mut self, key: crossterm::event::KeyEvent) -> KeyAction {
        tracing::info!("[app] handle_key: code={:?} active_panel={:?}", key.code, self.active_panel);
        match key.code {
            crossterm::event::KeyCode::Char('w')
                if key
                    .modifiers
                    .contains(crossterm::event::KeyModifiers::CONTROL) =>
            {
                if self.active_panel == PanelId::Chat
                    && self
                        .get_chat()
                        .map(|cp| cp.message_state.selection_start.is_some())
                        .unwrap_or(false)
                {
                    if let Some(chat) = self.get_chat_mut() {
                        chat.copy_selection_to_clipboard();
                    }
                    return KeyAction::None;
                }
            }
            crossterm::event::KeyCode::Char('c')
                if key
                    .modifiers
                    .contains(crossterm::event::KeyModifiers::CONTROL) =>
            {
                self.running = false;
                return KeyAction::Quit;
            }
            crossterm::event::KeyCode::Char('1')
                if key
                    .modifiers
                    .contains(crossterm::event::KeyModifiers::CONTROL) =>
            {
                self.activate_panel(PanelId::Chat).await;
                return KeyAction::None;
            }
            crossterm::event::KeyCode::Char('2')
                if key
                    .modifiers
                    .contains(crossterm::event::KeyModifiers::CONTROL) =>
            {
                self.activate_panel(PanelId::Sessions).await;
                return KeyAction::None;
            }

            crossterm::event::KeyCode::F(1) => {
                self.activate_panel(PanelId::Chat).await;
                return KeyAction::None;
            }
            crossterm::event::KeyCode::F(2) => {
                self.activate_panel(PanelId::Sessions).await;
                return KeyAction::None;
            }
            crossterm::event::KeyCode::F(3) => {
                if self.active_panel == PanelId::Chat {
                    let subagents = {
                        let ss = self.shared_state.lock();
                        ss.get_subagents()
                    };
                    if !subagents.is_empty() {
                        if let Some(chat) = self.get_chat_mut() {
                            chat.toggle_first_subagent(&subagents);
                            self.needs_redraw = true;
                            return KeyAction::None;
                        }
                    }
                }
            }

            crossterm::event::KeyCode::Tab => {
                let n = 2usize;
                let next_idx = match self.active_panel {
                    PanelId::Chat => 0,
                    PanelId::Sessions => 1,
                    _ => 0,
                };
                let next = match (next_idx + 1) % n {
                    0 => PanelId::Chat,
                    1 => PanelId::Sessions,
                    _ => unreachable!(),
                };
                self.activate_panel(next).await;
                return KeyAction::None;
            }
            crossterm::event::KeyCode::Char('t')
                if key
                    .modifiers
                    .contains(crossterm::event::KeyModifiers::CONTROL) =>
            {
                if let Some(chat) = self.get_chat_mut() {
                    let new_theme = chat.cycle_theme();
                    self.config.display.theme = new_theme.clone();
                    // Persist to disk (best-effort, don't block UI)
                    if let Err(e) = self.config.save() {
                        tracing::warn!("[theme] Failed to save theme to config: {}", e);
                    }
                    let display_name: String = new_theme
                        .parse::<ratatui_themes::ThemeName>()
                        .map(|t| t.display_name().to_string())
                        .unwrap_or(new_theme);
                    self.show_toast(
                        format!("Theme: {}", display_name),
                        ratatui_toaster::ToastType::Info,
                    );
                }
            }
            _ => {}
        }

        // Delegate to active panel; return the action for the event loop to route.
        // ChatInput / Steer / Quit are dispatched by the event loop in lib.rs
        // so channel ownership is unambiguous (app has input_tx but event loop
        // owns chat_tx and the shutdown flag).
        if let Some(panel) = self.panels.get_mut(&self.active_panel) {
            let action = panel.handle_key(key).await;
            match action {
                KeyAction::Clear => self.execute_command("clear").await,
                KeyAction::New => self.execute_command("new").await,
                KeyAction::Compact => self.execute_command("compact").await,
                KeyAction::Reasoning => self.execute_command("reasoning").await,
                KeyAction::Theme => self.execute_command("theme").await,
                KeyAction::Command { ref cmd_name, ref extra } => match cmd_name.as_str() {
                    "rename" => {
                        if extra.is_empty() {
                            self.show_toast(
                                "Usage: /rename [new_name]",
                                ratatui_toaster::ToastType::Error,
                            );
                        } else {
                            commands::execute_session_rename(self, extra).await;
                        }
                    }
                    _ => self.execute_command(cmd_name).await,
                },
                KeyAction::SwitchPanel(panel_id) => self.active_panel = panel_id,
                KeyAction::SessionChanged => {
                    self.activate_panel(PanelId::Chat).await;
                }
                // Pass-through: ChatInput, Steer, Quit, Interrupt, None go to event loop
                _ => {}
            }
            action
        } else {
            KeyAction::None
        }
    }

    pub fn create_config_panel(&mut self) {
        let yaml = serde_yaml::to_string(&self.config).unwrap_or_default();
        self.panels
            .insert(PanelId::Config, Box::new(ConfigPanel::new(yaml)));
    }

    pub fn create_setup_panel(&mut self) {
        let mut panel = SetupPanel::new();
        panel.set_config(AppConfig {
            model: self.config.model.clone(),
            temperature: self.config.temperature,
            max_tokens: self.config.max_tokens,
            max_iterations: self.config.max_iterations,
            max_spawn_depth: self.config.max_spawn_depth,
            max_concurrent_tasks: self.config.max_concurrent_tasks,
            tools: self.config.tools.clone(),
            skills: self.config.skills.clone(),
            gateway: self.config.gateway.clone(),
            display: self.config.display.clone(),
            context: self.config.context.clone(),
            voice: self.config.voice.clone(),
            providers: self.config.providers.clone(),
            custom_providers: self.config.custom_providers.clone(),
            vision: self.config.vision.clone(),
            session_store: self.config.session_store.clone(),
            retry: self.config.retry.clone(),
            concurrency: self.config.concurrency.clone(),
            hooks: self.config.hooks.clone(),
            fallback_models: self.config.fallback_models.clone(),
            agent: self.config.agent.clone(),
            events: self.config.events.clone(),
        });
        self.panels.insert(PanelId::Setup, Box::new(panel));
    }

    pub async fn init_panels(&mut self) -> Result<()> {
        self.create_chat_panel().await?;
        self.activate_panel(PanelId::Chat).await;
        self.create_sessions_panel().await?;
        self.create_config_panel();
        self.create_setup_panel();
        Ok(())
    }

    /// Initialize the active panel based on config presence and CLI session argument.
    ///
    /// - If no `config.yaml` exists → activate Setup panel (first-time guide).
    /// - If `config.yaml` exists → activate Chat panel and load the specified session
    ///   messages (if `session_name` was provided via CLI `-s`).
    pub async fn init_active_panel(&mut self, session_name: Option<&str>) -> Result<()> {
        // Set pending session from CLI argument
        if let Some(name) = session_name {
            self.pending_session = Some(name.to_string());
        }

        // Check if config.yaml exists
        let config_dir = AppConfig::config_dir_legacy();
        let config_path = config_dir.join("config.yaml");
        let has_config = config_path.exists();

        // Create all panels first
        self.create_chat_panel().await?;
        self.create_sessions_panel().await?;
        self.create_config_panel();
        self.create_setup_panel();

        if !has_config {
            // No config — activate Setup panel for first-time configuration
            info!(
                "No config.yaml found at {:?}, activating Setup panel",
                config_path
            );
            self.activate_panel(PanelId::Setup).await;
        } else {
            // Config exists — activate Chat panel
            info!("Config found at {:?}, activating Chat panel", config_path);
            self.activate_panel(PanelId::Chat).await;

            // Load session messages if session_name was provided via CLI
            let pending_name = self.pending_session.clone();
            if let Some(ref session_name) = pending_name {
                // Extract session data in a scoped block so guard is dropped
                // before we mutably borrow self for get_chat_mut().
                let ss_arc = Arc::clone(&self.shared_state);
                let (load_id, load_messages): (Option<String>, Vec<oben_models::Message>) = {
                    if ss_arc.lock().agent.is_some() {
                        // Ensure session manager is initialized so find_session_key
                        // can look up sessions by name in the in-memory cache.
                        ss_arc.lock().init_session_manager().await.ok();
                        let id = ss_arc.lock().find_session_key(session_name).await;
                        let msgs = if let Some(ref id) = id {
                            ss_arc.lock().get_session_messages(id).await
                        } else {
                            Vec::new()
                        };
                        (id, msgs)
                    } else {
                        (None, Vec::new())
                    }
                };

                if let Some(ref id) = load_id {
                    ss_arc.lock().switch_session_to(id).await.ok();
                    let chat = self.get_chat_mut();
                    if let Some(chat) = chat {
                        info!(
                            "Loading session '{}' ({} messages)",
                            session_name,
                            load_messages.len()
                        );
                        chat.update_from_messages(&load_messages, session_name.to_string().into());
                    }
                } else {
                    // Session not found — log and continue with Chat panel
                    info!(
                        "Session '{}' not found, using default Chat panel",
                        session_name
                    );
                }
            }
        }

        Ok(())
    }

    /// Load session messages without holding shared_state guard across await.
    /// This is needed when called from a spawned future where Send is required.
    pub async fn loaded_session_messages_safe(&self) -> Vec<oben_models::Message> {
        let ss = self.shared_state.lock();
        ss.loaded_session_messages().await
    }
}
