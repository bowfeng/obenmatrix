//! Application state and core logic.

use crate::commands;
use crate::panels::chat::ChatPanel;
use crate::panels::config::ConfigPanel;
use crate::panels::sessions::SessionsPanel;
use crate::panels::setup::SetupPanel;
use crate::panels::{KeyAction, Panel, PanelId};
use anyhow::Result;
use ratatui::layout::Rect;
use ratatui_toaster::{ToastBuilder, ToastEngine, ToastEngineBuilder, ToastType};
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tracing::info;

use crate::event::EventBus;
use crate::history;
use oben_agent::{Agent, AgentCallbacks, AgentConfig};
use oben_config::AppConfig;
use oben_tools::ToolRegistry;

/// Payload carried by TurnDone completion event from spawned task.
pub(super) struct TurnCompletion {
    pub(super) success: bool,
    pub(super) session_name: Option<String>,
    pub(super) messages: Vec<oben_models::Message>,
}

pub struct App {
    pub running: bool,
    pub active_panel: PanelId,
    pub panels: HashMap<PanelId, Box<dyn Panel>>,
    pub status: String,
    pub config: AppConfig,
    /// Agent protected by TokioMutex — guard is Send, needed for spawn()
    /// where we hold the lock across .await in agent.turn().
    pub agent: Option<Arc<tokio::sync::Mutex<Agent>>>,
    pub turn_handle: Option<tokio::task::JoinHandle<()>>,
    pub session_id: Option<String>,
    pub tools: std::sync::Arc<ToolRegistry>,
    pub tool_names: Vec<String>,
    pub input_tx: Option<tokio::sync::mpsc::UnboundedSender<crate::TuiEvent>>,
    pub input_history: history::InputHistory,
    pub paste_mode: bool,
    /// Event bus — single event emission point, wraps TurnState internally.
    /// Agents and UI components emit events through here to keep UI logic
    /// testable independently from the Agent.
    pub event_bus: Arc<EventBus>,
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
                if self.turn_handle.is_some() {
                    self.show_toast(
                        "Cannot clear: turn in progress",
                        ratatui_toaster::ToastType::Error,
                    );
                    return;
                }
                tracing::info!("[clear] START");
                if let Some(agent) = &self.agent {
                    let result = {
                        let mut guard = agent.lock().await;
                        let has_session = guard.active_session_name().await.is_some();
                        if has_session {
                            tracing::info!("[clear] deleting active session");
                        }
                        guard.reset().await
                    };
                    if let Err(e) = result {
                        self.show_toast(
                            format!("Clear failed: {e}"),
                            ratatui_toaster::ToastType::Error,
                        );
                        return;
                    }
                    tracing::info!("[clear] session reset complete, agent has no active session");
                } else {
                    tracing::info!("[clear] no agent, skipping reset");
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
                // Clear event bus turn state to prevent stale streaming text from being redrawn
                self.event_bus.clear();
                tracing::info!("[clear] event_bus state cleared");
                self.session_id = None;
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
                if self.agent.is_none() {
                    self.show_toast(
                        "Cannot compact: agent not initialized",
                        ratatui_toaster::ToastType::Error,
                    );
                    return;
                }
                if self.turn_handle.is_some() {
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
                if self.turn_handle.is_some() {
                    self.show_toast(
                        "Cannot create new session: turn in progress",
                        ratatui_toaster::ToastType::Error,
                    );
                    return;
                }
                if let Some(agent) = &self.agent {
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
                    if let Some(agent) = &self.agent {
                        let guard = agent.lock().await;
                        let session_name = guard.active_session_name().await.map(|n| n.clone());
                        let messages = guard.loaded_session_messages().await.unwrap_or_default();
                        s.set_session_data(session_name, messages);
                    }
                }
            }
            boxed_panel.on_activate(self).await;
            self.panels.insert(panel, boxed_panel);
        }
    }

    pub fn new() -> Result<Self> {
        let config = AppConfig::load()?;
        let mut tools = ToolRegistry::new();
        oben_tools::discover_builtin_tools(&mut tools);
        let tool_names: Vec<String> = tools.list_tools().iter().map(|t| t.name.clone()).collect();
        // Placeholder toast engine — area is set dynamically in draw_ui.
        let toast_engine = ToastEngine::new(ToastEngine::<()>::from_builder(
            ToastEngineBuilder::<()>::new(Rect::new(0, 0, 0, 0))
                .default_duration(Duration::from_secs(3)),
        ));
        Ok(Self {
            running: true,
            active_panel: PanelId::Chat,
            panels: HashMap::new(),
            status: String::new(),
            config,
            agent: None,
            turn_handle: None,
            session_id: None,
            tools: std::sync::Arc::new(tools),
            tool_names,
            input_tx: None,
            input_history: history::InputHistory::new(),
            paste_mode: false,
            event_bus: Arc::new(EventBus::new()),
            pending_session: None,
            reasoning_enabled: false,
            toast_engine,
            toast_expires_at: None,
        })
    }

    /// Show a temporary toast notification.
    pub fn show_toast<S: Into<Cow<'static, str>>>(&mut self, msg: S, toast_type: ToastType) {
        let builder = ToastBuilder::new(msg.into()).toast_type(toast_type);
        self.toast_engine.show_toast(builder);
        self.toast_expires_at = Some(std::time::Instant::now() + Duration::from_secs(2));
    }

    pub async fn init_agent(&mut self) -> Result<()> {
        let identity = oben_config::defaults::default_system_prompt();
        let skills_dirs: Vec<std::path::PathBuf> = vec![];
        let volatile = oben_agent::system_prompt::build_volatile_block(
            None,
            None,
            Some(&self.config.model.model),
        );
        let assembled = oben_agent::system_prompt::build_system_prompt(
            &identity,
            &self.tool_names,
            &skills_dirs,
            None,
            None,
            Some(&volatile),
        );
        let transport = oben_transport::Transport::from_config_with_tools_via_registry(
            &self.config.model,
            &assembled.prompt,
            &self
                .tools
                .list_tools()
                .iter()
                .map(|t| (*t).clone())
                .collect::<Vec<oben_models::Tool>>(),
        );

        let event_bus = Arc::clone(&self.event_bus);

        let callbacks = AgentCallbacks {
            step: Some(Box::new(move |msg: &str| {
                info!("STEP: {}", msg);
            })),
            status: Some(Box::new(move |level: &str, msg: &str| {
                info!("STATUS [{}]: {}", level, msg);
            })),
            tool_start: {
                let eb = Arc::clone(&event_bus);
                Some(Box::new(move |tool_name: &str, args_json: &str| {
                    eb.on_tool_start("tool-id", tool_name, args_json);
                }))
            },
            tool_complete: {
                let eb = Arc::clone(&event_bus);
                Some(Box::new(
                    move |tool_name: &str, _args_json: &str, result: &str| {
                        eb.on_tool_complete("tool-id", tool_name, result);
                    },
                ))
            },
            stream_delta: {
                let eb = Arc::clone(&event_bus);
                Some(Box::new(move |text: &str| {
                    eb.on_stream_delta(text);
                }))
            },
            reasoning: {
                let eb = Arc::clone(&event_bus);
                Some(Box::new(move |text: &str| {
                    eb.on_reasoning(text);
                }))
            },
            ..Default::default()
        };

        self.agent = Some(Arc::new(tokio::sync::Mutex::new(
            Agent::new(AgentConfig {
                system_prompt: assembled.prompt,
                transport,
                tools: std::sync::Arc::clone(&self.tools),
                skills_dirs: vec![],
                max_iterations: self.config.max_iterations.unwrap_or(50),
                max_messages: self.config.context.max_messages.unwrap_or(100),
                context_config: oben_agent::compact::CompactCofig {
                    context_length: self.config.context.context_length,
                    threshold_percent: self.config.context.threshold_percent,
                    ..oben_agent::compact::CompactCofig::default()
                },
                fallback_models: vec![],
                callbacks,
                concurrent_dispatch_config: oben_agent::ConcurrentDispatchConfig::default(),
                nudge_config: None,
            })
            .await?,
        )));

        Ok(())
    }

    pub fn begin_turn(&self) {
        self.event_bus.begin_turn();
    }

    pub fn finalize_turn(&self, outcome: &str) {
        self.event_bus.on_turn_completed(outcome);
    }

    pub async fn create_chat_panel(&mut self) -> Result<()> {
        tracing::info!("[panel] Creating ChatPanel");
        let theme = self.config.display.theme.clone();
        tracing::info!("[panel] Using theme: {}", theme);
        self.panels.insert(
            PanelId::Chat,
            Box::new(ChatPanel::new_with_theme(None, None, &theme)),
        );
        tracing::info!(
            "[panel] ChatPanel inserted, panels={:?}",
            self.panels.keys().collect::<Vec<_>>()
        );
        Ok(())
    }

    pub async fn create_sessions_panel(&mut self) -> Result<()> {
        tracing::info!("[panel] Creating SessionsPanel");
        if let Some(agent) = &self.agent {
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

    pub async fn handle_key(&mut self, key: crossterm::event::KeyEvent) {
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
                    return;
                }
            }
            crossterm::event::KeyCode::Char('c')
                if key
                    .modifiers
                    .contains(crossterm::event::KeyModifiers::CONTROL) =>
            {
                self.running = false;
                return;
            }
            crossterm::event::KeyCode::Char('1')
                if key
                    .modifiers
                    .contains(crossterm::event::KeyModifiers::CONTROL) =>
            {
                self.activate_panel(PanelId::Chat).await;
                return;
            }
            crossterm::event::KeyCode::Char('2')
                if key
                    .modifiers
                    .contains(crossterm::event::KeyModifiers::CONTROL) =>
            {
                self.activate_panel(PanelId::Sessions).await;
                return;
            }
            crossterm::event::KeyCode::Char('3')
                if key
                    .modifiers
                    .contains(crossterm::event::KeyModifiers::CONTROL) =>
            {
                self.activate_panel(PanelId::Config).await;
                return;
            }
            crossterm::event::KeyCode::Char('4')
                if key
                    .modifiers
                    .contains(crossterm::event::KeyModifiers::CONTROL) =>
            {
                self.activate_panel(PanelId::Setup).await;
                return;
            }
            crossterm::event::KeyCode::F(1) => {
                self.activate_panel(PanelId::Chat).await;
                return;
            }
            crossterm::event::KeyCode::F(2) => {
                self.activate_panel(PanelId::Sessions).await;
                return;
            }
            crossterm::event::KeyCode::F(3) => {
                self.activate_panel(PanelId::Config).await;
                return;
            }
            crossterm::event::KeyCode::F(4) => {
                self.activate_panel(PanelId::Setup).await;
                return;
            }
            crossterm::event::KeyCode::Tab => {
                let n = 4usize;
                let next_idx = match self.active_panel {
                    PanelId::Chat => 0,
                    PanelId::Sessions => 1,
                    PanelId::Config => 2,
                    PanelId::Setup => 3,
                };
                let next = match (next_idx + 1) % n {
                    0 => PanelId::Chat,
                    1 => PanelId::Sessions,
                    2 => PanelId::Config,
                    3 => PanelId::Setup,
                    _ => unreachable!(),
                };
                self.activate_panel(next).await;
                return;
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

        // Call panel handle_key and process returned action
        let action = if let Some(panel) = self.panels.get_mut(&self.active_panel) {
            panel.handle_key(key).await
        } else {
            KeyAction::None
        };

        // Process the action
        match action {
            KeyAction::Clear => {
                self.execute_command("clear").await;
            }
            KeyAction::New => {
                self.execute_command("new").await;
            }
            KeyAction::Compact => {
                self.execute_command("compact").await;
            }
            KeyAction::Quit => {
                self.execute_command("quit").await;
            }
            KeyAction::Reasoning => {
                self.execute_command("reasoning").await;
            }
            KeyAction::Theme => {
                self.execute_command("theme").await;
            }
            KeyAction::Command { cmd_name, extra } => match cmd_name.as_str() {
                "rename" => {
                    if extra.is_empty() {
                        self.show_toast(
                            "Usage: /rename [new_name]",
                            ratatui_toaster::ToastType::Error,
                        );
                    } else {
                        commands::execute_session_rename(self, &extra).await;
                    }
                }
                _ => {
                    self.execute_command(&cmd_name).await;
                }
            },
            KeyAction::ChatInput(text) => {
                if let Some(tx) = &self.input_tx {
                    let _ = tx.send(crate::TuiEvent::ChatInput(text));
                }
            }
            KeyAction::SwitchPanel(panel_id) => {
                self.active_panel = panel_id;
            }
            KeyAction::SessionChanged => {
                // Reload session messages into ChatPanel after switching sessions.
                // Re-use activate_panel which already knows how to fetch messages.
                self.activate_panel(PanelId::Chat).await;
            }
            KeyAction::None => {}
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
            tools: self.config.tools.clone(),
            skills: self.config.skills.clone(),
            gateway: self.config.gateway.clone(),
            display: self.config.display.clone(),
            context: self.config.context.clone(),
            providers: self.config.providers.clone(),
            custom_providers: self.config.custom_providers.clone(),
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
                let (load_id, load_messages) = {
                    if let Some(agent) = &self.agent {
                        let mut agent = agent.lock().await;
                        // Ensure session manager is initialized so find_key()
                        // can look up sessions by name in the in-memory cache.
                        let _ = agent.init_session_manager().await;
                        let id = agent.find_session_key(session_name).await;
                        let msgs = if let Some(ref id) = id {
                            agent.get_session_messages(id).await.unwrap_or_default()
                        } else {
                            Vec::new()
                        };
                        (id, msgs)
                    } else {
                        (None, Vec::new())
                    }
                };

                if let Some(ref id) = load_id {
                    if let Some(agent) = &self.agent {
                        let mut g = agent.lock().await;
                        if let Err(e) = g.switch_session_to(id).await {
                            tracing::error!("Failed to switch session '{id}': {e}");
                        }
                    }
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
}
