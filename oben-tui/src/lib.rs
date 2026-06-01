//! ObenAgent TUI — a full terminal UI for chat, sessions, config, and setup.
//!
//! Replaces the CLI-based `oben chat`, `oben setup`, `oben config`, and
//! `oben sessions` with a ratatui-driven interface.

pub mod clipboard;
pub mod event;
pub mod history;
pub mod panels;
pub mod turn;
pub mod widgets;

use anyhow::Result;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use panels::chat::ChatPanel;
use panels::config::ConfigPanel;
use panels::sessions::SessionsPanel;
use panels::setup::SetupPanel;
use panels::{Panel, PanelId};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use ratatui::Terminal;
use ratatui::widgets::{Tabs, Block};
use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::unbounded_channel;
use tokio::sync::Mutex as TokioMutex;
use tracing::info;
use tracing_subscriber::{fmt::layer, layer::SubscriberExt, util::SubscriberInitExt};

use crate::event::EventBus;
use oben_agent::{Agent, AgentCallbacks, AgentConfig};
use oben_config::AppConfig;

use oben_tools::ToolRegistry;

pub struct Layouts {
    pub header: Rect,
    pub body: Rect,
    pub statusbar: Rect,
}

impl Layouts {
    pub fn new(area: Rect) -> Self {
        let chunks = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(2),
        ])
        .split(area);
        Self {
            header: chunks[0],
            body: chunks[1],
            statusbar: chunks[2],
        }
    }
}

pub enum TuiEvent {
    Key(KeyEvent),
    ChatInput(String),
    Mouse(MouseEvent),
}

/// Payload carried by TurnDone completion event from spawned task.
struct TurnCompletion {
    success: bool,
    session_name: Option<String>,
    messages: Vec<oben_models::Message>,
}

pub struct App {
    pub running: bool,
    pub active_panel: PanelId,
    pub panels: HashMap<PanelId, Box<dyn Panel>>,
    pub status: String,
    pub config: AppConfig,
    /// Agent protected by TokioMutex — guard is Send, needed for spawn()
    /// where we hold the lock across .await in agent.turn().
    pub agent: Option<Arc<TokioMutex<Agent>>>,
    pub turn_handle: Option<tokio::task::JoinHandle<()>>,
    pub session_id: Option<String>,
    pub tools: std::sync::Arc<ToolRegistry>,
    pub tool_names: Vec<String>,
    pub input_tx: Option<tokio::sync::mpsc::UnboundedSender<TuiEvent>>,
    pub input_history: history::InputHistory,
    pub paste_mode: bool,
    /// Event bus — single event emission point, wraps TurnState internally.
    /// Agents and UI components emit events through here to keep UI logic
    /// testable independently from the Agent.
    pub event_bus: Arc<EventBus>,
}

impl App {
    /// Activate a panel and call its on_activate/on_deactivate hooks.
    pub fn activate_panel(&mut self, panel: PanelId) {
        let old = self.active_panel;
        if old != panel {
            if let Some(p) = self.panels.get_mut(&old) {
                p.on_deactivate();
            }
        }
        self.active_panel = panel;
        if let Some(p) = self.panels.get_mut(&panel) {
            p.on_activate();
        }
    }

    pub fn new() -> Result<Self> {
        let config = AppConfig::load()?;
        let mut tools = ToolRegistry::new();
        oben_tools::discover_builtin_tools(&mut tools);
        let tool_names: Vec<String> = tools.list_tools().iter().map(|t| t.name.clone()).collect();
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
        })
    }

    pub fn init_chat(&mut self) -> Result<()> {
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

        self.agent = Some(Arc::new(TokioMutex::new(Agent::new(AgentConfig {
            system_prompt: assembled.prompt,
            transport,
            tools: std::sync::Arc::clone(&self.tools),
            skills_dirs: vec![],
            max_iterations: self.config.max_iterations.unwrap_or(50),
            max_messages: self.config.context.max_messages.unwrap_or(100),
            context_config: oben_agent::CompactCofig::default(),
            fallback_models: vec![],
            callbacks,
            concurrent_dispatch_config: oben_agent::ConcurrentDispatchConfig::default(),
            nudge_config: None,
        })?)));

        Ok(())
    }

    pub fn begin_turn(&self) {
        self.event_bus.begin_turn();
    }

    pub fn finalize_turn(&self, outcome: &str) {
        self.event_bus.on_turn_completed(outcome);
    }

    pub async fn create_chat_panel(&mut self) -> Result<()> {
        let (session_id, messages) = match &self.agent {
            Some(agent) => {
                let guard = agent.lock().await;
                let sid = guard.active_session_name().map(|s| s.clone());
                let msgs = guard
                    .session_manager()
                    .active_session()
                    .map(|s| s.messages.clone());
                (sid, msgs)
            }
            None => (None, None),
        };
        self.panels.insert(
            PanelId::Chat,
            Box::new(ChatPanel::new(session_id, messages)),
        );
        Ok(())
    }

    pub async fn create_sessions_panel(&mut self) -> Result<()> {
        let sessions: Vec<oben_models::Session> = match &self.agent {
            Some(agent) => {
                let mut g = agent.lock().await;
                let _ = g.session_manager_mut().init();
                g.session_manager().list_sessions_full()
            }
            None => vec![],
        };
        self.panels
            .insert(PanelId::Sessions, Box::new(SessionsPanel::new(sessions)));
        Ok(())
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.active_panel == PanelId::Chat
                    && self
                        .panels
                        .get(&PanelId::Chat)
                        .and_then(|p| p.downcast_ref::<ChatPanel>())
                        .map(|cp| cp.message_state.selection_start.is_some())
                        .unwrap_or(false)
                {
                    if let Some(panel) = self.panels.get_mut(&PanelId::Chat) {
                        if let Some(chat) = panel.downcast_mut::<ChatPanel>() {
                            chat.copy_selection_to_clipboard();
                        }
                    }
                    return;
                }
                self.running = false;
                return;
            }
            KeyCode::Char('1') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.activate_panel(PanelId::Chat);
                return;
            }
            KeyCode::Char('2') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.activate_panel(PanelId::Sessions);
                return;
            }
            KeyCode::Char('3') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.activate_panel(PanelId::Config);
                return;
            }
            KeyCode::Char('4') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.activate_panel(PanelId::Setup);
                return;
            }
            KeyCode::F(1) => {
                self.activate_panel(PanelId::Chat);
                return;
            }
            KeyCode::F(2) => {
                self.activate_panel(PanelId::Sessions);
                return;
            }
            KeyCode::F(3) => {
                self.activate_panel(PanelId::Config);
                return;
            }
            KeyCode::F(4) => {
                self.activate_panel(PanelId::Setup);
                return;
            }
            KeyCode::Tab => {
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
                self.activate_panel(next);
                return;
            }
            KeyCode::Char('t') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let panel_id = PanelId::Chat;
                if let Some(boxed_panel) = self.panels.remove(&panel_id) {
                    let mut panel = boxed_panel;
                    if let Some(chat) = panel.downcast_mut::<ChatPanel>() {
                        chat.cycle_theme();
                    }
                    self.panels.insert(panel_id, panel);
                }
            }
            _ => {}
        }
        if let Some(_panel) = self.panels.get_mut(&self.active_panel) {
            let panel_id = self.active_panel;
            if let Some(boxed_panel) = self.panels.remove(&panel_id) {
                let mut panel = boxed_panel;
                panel.handle_key(self, key);
                self.panels.insert(panel_id, panel);
            }
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
        self.create_sessions_panel().await?;
        self.create_config_panel();
        self.create_setup_panel();
        Ok(())
    }
}

pub async fn run_tui() -> Result<()> {
    let mut app = App::new()?;
    app.init_chat()?;
    app.init_panels().await?;

    // Set up logging
    #[allow(unexpected_cfgs)]
    #[cfg(not(feature = "cli-wired"))]
    {
        let log_dir = dirs::home_dir()
            .map(|d| d.join(".obenalien/logs"))
            .unwrap_or_else(|| std::path::PathBuf::from("./logs"));
        let _ = std::fs::create_dir_all(&log_dir);
        let datetime = chrono::Local::now().format("%Y%m%dT%H%M%S");
        let log_path = log_dir.join(format!("oa-{datetime}.log"));
        let log_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)?;
        let subscriber = tracing_subscriber::registry().with(layer().with_writer(log_file));
        let _ = subscriber.try_init();
    }

    enable_raw_mode()?;
    io::stdout().execute(EnterAlternateScreen)?;
    io::stdout().execute(EnableMouseCapture)?;

    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let (event_tx, mut event_rx) = unbounded_channel();
    let event_tx_for_signal = event_tx.clone();
    app.input_tx = Some(event_tx.clone());

    let running = Arc::new(AtomicBool::new(true));
    let running_clone = Arc::clone(&running);

    // Read events from crossterm in a blocking task
    let reader_handle = tokio::task::spawn_blocking(move || {
        while running_clone.load(Ordering::SeqCst) {
            if crossterm::event::poll(Duration::from_millis(16)).unwrap() {
                match crossterm::event::read().unwrap() {
                    crossterm::event::Event::Key(key) => {
                        if key.kind == KeyEventKind::Press || key.kind == KeyEventKind::Repeat {
                            let _ = event_tx.send(TuiEvent::Key(key));
                        }
                    }
                    crossterm::event::Event::Mouse(mouse) => {
                        let _ = event_tx.send(TuiEvent::Mouse(mouse));
                    }
                    _ => {}
                }
            }
        }
    });

    // Ctrl+C signal handler — raw mode intercepts key events, so we must catch SIGINT directly
    let running_for_signal = running.clone();
    let quit_ev = TuiEvent::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
    tokio::spawn(async move {
        let mut signal =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt()).unwrap();
        let _ = signal.recv().await;
        running_for_signal.store(false, Ordering::SeqCst);
        let _ = event_tx_for_signal.send(quit_ev);
    });

    // Channel for signaling task completion back to the main event loop
    let (done_tx, mut done_rx) = tokio::sync::mpsc::unbounded_channel::<TurnCompletion>();

    // Main event loop — draw when something changes.
    // Only redraw periodically when streaming (to show live updates).
    // Draw once on startup so the UI is visible immediately.
    terminal.draw(|frame| draw_ui(frame, &mut app))?;
    loop {
        if !app.running {
            break;
        }
        let mut redraw = false;
        tokio::select! {
            // Timeout: only redraw periodically during streaming so live text
            // remains visible even when no user events arrive.
            _ = tokio::time::sleep(Duration::from_millis(32)) => {
                let is_streaming = app.panels.get(&PanelId::Chat)
                    .and_then(|p| p.downcast_ref::<ChatPanel>())
                    .map(|cp| cp.streaming)
                    .unwrap_or(false);
                if is_streaming {
                    let _ = terminal.draw(|frame| draw_ui(frame, &mut app));
                }
            }

            // Check for completion signal from spawned turn task
            maybe_completion = done_rx.recv() => {
                if let Some(completion) = maybe_completion {
                    app.turn_handle = None;
                    if completion.success {
                        if let Some(panel) = app.panels.get_mut(&PanelId::Chat) {
                            if let Some(chat) = panel.downcast_mut::<ChatPanel>() {
                                chat.update_from_messages(&completion.messages, completion.session_name);
                            }
                        }
                    } else {
                        app.status = "Turn completed with errors".into();
                        let eb_state = Arc::clone(&app.event_bus.state());
                        if let Some(panel) = app.panels.get_mut(&PanelId::Chat) {
                            if let Some(chat) = panel.downcast_mut::<ChatPanel>() {
                                chat.streaming = false;
                                chat.update_from_turn_state(&eb_state.lock().unwrap());
                            }
                        }
                    }
                    redraw = true;
                }
            }

            // Event branch: handles key, mouse, and chat input events.
            event = event_rx.recv() => {
                match event {
                    Some(TuiEvent::Key(key)) => {
                        app.handle_key(key);
                        redraw = true;
                    }
                    Some(TuiEvent::Mouse(mouse_event)) => {
                        let click_on_tabs = matches!(mouse_event.kind, MouseEventKind::Down(crossterm::event::MouseButton::Left))
                            && mouse_event.row == 0;
                        if click_on_tabs {
                            let tab_names = ["Chat", "Sessions", "Config", "Setup"];
                            let tab_widths: Vec<usize> = tab_names.iter().map(|n| n.len() + 2).collect();
                            let mut consumed = 0usize;
                            for (i, &pw) in tab_widths.iter().enumerate() {
                                if mouse_event.column >= consumed as u16 && mouse_event.column < (consumed + pw) as u16 {
                                    app.active_panel = match i {
                                        0 => PanelId::Chat,
                                        1 => PanelId::Sessions,
                                        2 => PanelId::Config,
                                        3 => PanelId::Setup,
                                        _ => break,
                                    };
                                    break;
                                }
                                consumed += pw + 1;
                            }
                            redraw = true;
                            continue;
                        }
                        let scroll_up = matches!(mouse_event.kind, MouseEventKind::ScrollUp)
                            || matches!(mouse_event.kind, MouseEventKind::ScrollLeft);
                        let scroll_down = matches!(mouse_event.kind, MouseEventKind::ScrollDown)
                            || matches!(mouse_event.kind, MouseEventKind::ScrollRight);
                        if scroll_up {
                            if let Some(panel) = app.panels.get_mut(&PanelId::Chat) {
                                if let Some(chat) = panel.downcast_mut::<ChatPanel>() {
                                    chat.message_state.scroll_to_bottom = false;
                                    chat.message_state.scroll_state.lock().unwrap().prev();
                                    redraw = true;
                                }
                            }
                        } else if scroll_down {
                            if let Some(panel) = app.panels.get_mut(&PanelId::Chat) {
                                if let Some(chat) = panel.downcast_mut::<ChatPanel>() {
                                    chat.message_state.scroll_to_bottom = true;
                                    chat.message_state.scroll_state.lock().unwrap().next();
                                    redraw = true;
                                }
                            }
                        }
                    }
                    Some(TuiEvent::ChatInput(input)) => {
                        handle_chat_input(&mut app, input, &done_tx).await;
                        redraw = true;
                    }
                    None => break,
                }
            }
        }

        // Redraw after events and during streaming
        if redraw {
            let _ = terminal.draw(|frame| draw_ui(frame, &mut app));
        }
    }

    running.store(false, Ordering::SeqCst);
    let _ = reader_handle.await;
    drop(terminal);
    io::stdout().execute(LeaveAlternateScreen)?;
    io::stdout().execute(DisableMouseCapture)?;
    disable_raw_mode()?;
    info!("TUI exited normally.");
    Ok(())
}

/// Handle a chat input: spawn a turn in a background task so the event loop
/// can keep drawing the UI during streaming.
async fn handle_chat_input(
    app: &mut App,
    input: String,
    done_tx: &tokio::sync::mpsc::UnboundedSender<TurnCompletion>,
) {
    info!("handle_chat_input: input.len()={}", input.len());

    let Some(agent) = app.agent.as_ref().map(|a| Arc::clone(a)) else {
        app.status = "Agent not initialized".into();
        return;
    };

    if app.turn_handle.is_some() {
        app.status = "Already processing a turn. Please wait...".into();
        return;
    }

    let was_chat = app.active_panel == PanelId::Chat;
    tracing::info!(
        "handle_chat_input: was_chat={}, has_agent={}, turn_handle_some={}",
        was_chat,
        app.agent.is_some(),
        app.turn_handle.is_some()
    );

    // Begin turn tracking
    let event_bus = Arc::clone(&app.event_bus);
    event_bus.begin_turn();
    tracing::info!("handle_chat_input: turn started via event bus");

    // Prepare ChatPanel for streaming
    if was_chat {
        if let Some(panel) = app.panels.get_mut(&PanelId::Chat) {
            if let Some(chat) = panel.downcast_mut::<ChatPanel>() {
                tracing::info!("handle_chat_input: setting ChatPanel.streaming=true");
                chat.streaming = true;
                chat.message_state.turn_state_ref = Some(Arc::clone(&app.event_bus.state()));
                chat.append_user_message(&input);
                tracing::info!("handle_chat_input: appended user message to chat, msg_count=0");
            }
        }
    }

    // Spawn turn in background so event loop is not blocked.
    // TokioMutex guard IS Send, so the spawned future can hold the lock
    // across .await in agent.turn().
    let agent_clone = agent;
    let eb = Arc::clone(&app.event_bus);
    let eb_for_finalize = Arc::clone(&app.event_bus);
    let done_tx_clone = done_tx.clone();
    let input_clone = input.clone();

    let handle = tokio::spawn({
        tracing::info!("handle_chat_input: tokio::spawn called");
        async move {
            info!("spawned_turn_task: calling agent.turn()");
            let (result, sid, messages) = {
                let mut guard = agent_clone.lock().await;
                // inline delta_callback now emits through EventBus
                let delta_callback = Box::new(move |text: &str| {
                    tracing::info!("[delta_callback] text.len={} text='{}'", text.len(), text);
                    eb.on_stream_delta(text);
                });
                let result = guard.turn(&input_clone, false, Some(delta_callback)).await;
                let sid = guard.active_session_name().map(|s| s.clone());
                let msgs = guard
                    .session_manager()
                    .active_session()
                    .map(|s| s.messages.clone())
                    .unwrap_or_default();
                (result, sid, msgs)
            };

            tracing::info!(
                "spawned_turn_task: turn completed, is_ok={}",
                result.is_ok()
            );

            // Finalize turn state
            match &result {
                Ok(_) => {
                    eb_for_finalize.on_turn_completed("completed");
                    let _ = done_tx_clone.send(TurnCompletion { success: true, session_name: sid, messages });
                    tracing::info!("spawned_turn_task: sent done_tx success");
                }
                Err(e) => {
                    eb_for_finalize.on_turn_error(&format!("{}", e));
                    tracing::info!("spawned_turn_task: finalized turn_state error: {}", e);
                    let _ = done_tx_clone.send(TurnCompletion { success: false, session_name: None, messages: Vec::new() });
                    tracing::info!("spawned_turn_task: sent done_tx error");
                }
            }

            tracing::info!("spawned_turn_task: done");
        }
    });

    tracing::info!(
        "handle_chat_input: spawn completed, handle={:?}, about to send done_tx",
        handle
    );
    app.turn_handle = Some(handle);
    app.input_history.append(&input);

    if was_chat {
        if let Some(panel) = app.panels.get_mut(&PanelId::Chat) {
            if let Some(chat) = panel.downcast_mut::<ChatPanel>() {
                chat.input.text.clear();
                chat.input.cursor = 0;
            }
        }
    }
}

fn draw_ui(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let layout = Layouts::new(area);

    // Inject turn_state_ref into ChatPanel so draw() can read streaming text in real-time
    // Clone event_bus state Arc first to avoid borrowing app twice
    let eb_state = Arc::clone(&app.event_bus.state());
    if let Some(panel) = app.panels.get_mut(&PanelId::Chat) {
        if let Some(chat) = panel.downcast_mut::<ChatPanel>() {
            chat.set_turn_state_ref(Arc::clone(&eb_state));
            chat.update_from_turn_state(&eb_state.lock().unwrap());
        }
    }

    // Collect ChatPanel streaming state (after injecting ref)
    let chat_panel_info = if let Some(panel) = app.panels.get(&PanelId::Chat) {
        if let Some(cp) = panel.downcast_ref::<ChatPanel>() {
            format!("streaming={}", cp.streaming)
        } else {
            "no_chat_panel".to_string()
        }
    } else {
        "no_chat_panel".to_string()
    };
    tracing::info!("[draw_ui] chat_panel={}", chat_panel_info);

    let is_streaming = app
        .panels
        .get(&PanelId::Chat)
        .and_then(|p| p.downcast_ref::<ChatPanel>())
        .map(|cp| cp.streaming)
        .unwrap_or(false);

    let panel_names: [&str; 4] = ["Chat", "Sessions", "Config", "Setup"];
    let panel_index = match app.active_panel {
        PanelId::Chat => 0,
        PanelId::Sessions => 1,
        PanelId::Config => 2,
        PanelId::Setup => 3,
    };
    let tabs = Tabs::new(panel_names)
        .style(Style::default().fg(Color::DarkGray))
        .highlight_style(Style::default().fg(Color::Cyan).bold())
        .divider(" ")
        .select(panel_index)
        .block(Block::default().style(Style::default().bg(Color::Gray)));
    frame.render_widget(tabs, layout.header);

    if let Some(panel) = app.panels.get(&app.active_panel) {
        panel.draw(frame, layout.body);
    }

    // Derive session info from stored ChatPanel fields — no Agent locking.
    let (session_name, msg_count) = match app.active_panel {
        PanelId::Sessions => {
            match app.panels.get(&PanelId::Sessions) {
                Some(panel) => {
                    if let Some(sessions) = panel.downcast_ref::<SessionsPanel>() {
                        (sessions.get_session_name().unwrap_or_default(),
                         sessions.get_message_count().unwrap_or(0))
                    } else {
                        (String::new(), 0)
                    }
                }
                None => (String::new(), 0),
            }
        }
        _ => {
            match app.panels.get(&PanelId::Chat) {
                Some(panel) => {
                    if let Some(chat) = panel.downcast_ref::<ChatPanel>() {
                        if let Some(ref sid) = chat.session_id {
                            (sid.clone(), chat.message_count)
                        } else {
                            (app.session_id.clone().unwrap_or_default(), chat.message_count)
                        }
                    } else {
                        (String::new(), 0)
                    }
                }
                None => (String::new(), 0),
            }
        }
    };

    let session_text = match &session_name {
        s if !s.is_empty() => format!(" Session: {} ({} msgs)", s, msg_count),
        _ => " No session".to_string(),
    };
    let mode_text = match (is_streaming, app.status.as_str()) {
        (true, _) => "⏳ Streaming",
        (_, s) if s.starts_with("Error") => "Error",
        (_, s) if !s.is_empty() && s != " No session" => "Info",
        _ => "Ready",
    };
    let status_lines: Vec<Line> = vec![
        Line::from(format!(" [{}]  {}", mode_text, session_text)),
    ];
    let status_para = Paragraph::new(status_lines);
    let status_area = Rect::new(
        layout.statusbar.x,
        layout.statusbar.y,
        layout.statusbar.width,
        layout.statusbar.height,
    );
    frame.render_widget(status_para, status_area);
}
