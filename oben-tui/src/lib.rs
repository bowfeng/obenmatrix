//! ObenAgent TUI — a full terminal UI for chat, sessions, config, and setup.
//!
//! Replaces the CLI-based `oben chat`, `oben setup`, `oben config`, and
//! `oben sessions` with a ratatui-driven interface.

pub mod clipboard;
pub mod history;
pub mod panels;
pub mod turn;
pub mod widgets;

use anyhow::Result;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, MouseEvent, MouseEventKind,
};
use crossterm::terminal::{
    enable_raw_mode, disable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use panels::chat::{ChatPanel, ChatViewMode};
use panels::config::ConfigPanel;
use panels::setup::SetupPanel;
use panels::sessions::SessionsPanel;
use panels::{Panel, PanelId};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use ratatui::Terminal;
use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use tokio::sync::mpsc::unbounded_channel;
use tokio::sync::Mutex as TokioMutex;
use tracing::info;
use tracing_subscriber::{fmt::layer, layer::SubscriberExt, util::SubscriberInitExt};

use oben_agent::{Agent, AgentCallbacks, AgentConfig};
use oben_config::AppConfig;
use oben_models::Message;
use oben_tools::ToolRegistry;

pub struct Layouts {
    pub header: Rect,
    pub body: Rect,
    pub statusbar: Rect,
}

impl Layouts {
    pub fn new(area: Rect) -> Self {
        let chunks = Layout::vertical([
            Constraint::Length(2),
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
}

pub struct App {
    pub running: bool,
    pub active_panel: PanelId,
    pub panels: HashMap<PanelId, Box<dyn Panel>>,
    pub status: String,
    pub config: AppConfig,
    /// Agent protected by TokioMutex — guard is Send, needed for spawn()
    /// where we hold the lock across .await in agent.turn().
    pub chat: Option<Arc<TokioMutex<Agent>>>,
    pub turn_handle: Option<tokio::task::JoinHandle<()>>,
    /// Cached session info updated on turn completion.
    /// Avoids blocking the tokio runtime during draw().
    pub cached_session_info: String,
    pub session_id: Option<String>,
    pub tools: std::sync::Arc<ToolRegistry>,
    pub tool_names: Vec<String>,
    pub input_tx: Option<tokio::sync::mpsc::UnboundedSender<TuiEvent>>,
    pub input_history: history::InputHistory,
    pub paste_mode: bool,
    /// TurnState protected by std Mutex — accessed only in sync context,
    /// never held across .await. std Mutex is lighter than TokioMutex.
    pub turn_state: Arc<StdMutex<turn::event::TurnState>>,
}

impl App {
    pub fn new() -> Result<Self> {
        let config = AppConfig::load()?;
        let mut tools = ToolRegistry::new();
        oben_tools::discover_builtin_tools(&mut tools);
        let tool_names: Vec<String> = tools.list_tools().iter()
            .map(|t| t.name.clone())
            .collect();
        Ok(Self {
            running: true,
            active_panel: PanelId::Chat,
            panels: HashMap::new(),
            status: String::new(),
            config,
            chat: None,
            turn_handle: None,
            session_id: None,
            cached_session_info: String::new(),
            tools: std::sync::Arc::new(tools),
            tool_names,
            input_tx: None,
            input_history: history::InputHistory::new(),
            paste_mode: false,
            turn_state: Arc::new(StdMutex::new(turn::event::TurnState::new())),
        })
    }

    pub fn init_chat(&mut self) -> Result<()> {
        let identity = oben_config::defaults::default_system_prompt();
        let skills_dirs: Vec<std::path::PathBuf> = vec![];
        let volatile = oben_agent::system_prompt::build_volatile_block(
            None, None, Some(&self.config.model.model),
        );
        let assembled = oben_agent::system_prompt::build_system_prompt(
            &identity, &self.tool_names, &skills_dirs, None, None, Some(&volatile),
        );
        let transport = oben_transport::Transport::from_config_with_tools_via_registry(
            &self.config.model, &assembled.prompt,
            &self.tools.list_tools().iter().map(|t| (*t).clone()).collect::<Vec<oben_models::Tool>>(),
        );

        let turn_state = Arc::clone(&self.turn_state);

        let callbacks = AgentCallbacks {
            step: Some(Box::new(move |msg: &str| {
                info!("STEP: {}", msg);
            })),
            status: Some(Box::new(move |level: &str, msg: &str| {
                info!("STATUS [{}]: {}", level, msg);
            })),
            tool_start: {
                let ts_clone = Arc::clone(&turn_state);
                Some(Box::new(move |tool_name: &str, args_json: &str| {
                    if let Ok(mut ts) = ts_clone.lock() {
                        ts.on_tool_start("tool-id", tool_name, args_json);
                    }
                }))
            },
            tool_complete: {
                let ts_clone = Arc::clone(&turn_state);
                Some(Box::new(move |tool_name: &str, _args_json: &str, result: &str| {
                    if let Ok(mut ts) = ts_clone.lock() {
                        ts.on_tool_complete("tool-id", tool_name, result);
                    }
                }))
            },
            stream_delta: {
                let ts_clone = Arc::clone(&turn_state);
                Some(Box::new(move |text: &str| {
                    if let Ok(mut ts) = ts_clone.lock() {
                        ts.on_stream_delta(text);
                    }
                }))
            },
            reasoning: {
                let ts_clone = Arc::clone(&turn_state);
                Some(Box::new(move |text: &str| {
                    if let Ok(mut ts) = ts_clone.lock() {
                        ts.on_reasoning(text);
                    }
                }))
            },
            ..Default::default()
        };

        self.chat = Some(Arc::new(TokioMutex::new(Agent::new(AgentConfig {
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
        if let Ok(mut ts) = self.turn_state.lock() {
            ts.on_turn_start();
        }
    }

    pub fn finalize_turn(&self, outcome: &str) {
        if let Ok(mut ts) = self.turn_state.lock() {
            ts.on_completed(outcome);
        }
    }

    pub async fn create_chat_panel(&mut self) -> Result<()> {
        let (session_id, messages) = match &self.chat {
            Some(agent) => {
                let guard = agent.lock().await;
                let sid = guard.active_session_name().map(|s| s.clone());
                let msgs = guard.session_manager().active_session()
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
        let sessions: Vec<oben_models::Session> = match &self.chat {
            Some(agent) => {
                agent.lock().await.session_manager().list_sessions_full()
            }
            None => vec![],
        };
        self.panels.insert(PanelId::Sessions, Box::new(SessionsPanel::new(sessions)));
        Ok(())
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.running = false;
                return;
            }
            KeyCode::F(1) => { self.active_panel = PanelId::Chat; return; }
            KeyCode::F(2) => { self.active_panel = PanelId::Sessions; return; }
            KeyCode::F(3) => { self.active_panel = PanelId::Config; return; }
            KeyCode::F(4) => { self.active_panel = PanelId::Setup; return; }
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
        self.panels.insert(PanelId::Config, Box::new(ConfigPanel::new(yaml)));
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

    pub fn update_session_messages(&mut self, _messages: Vec<Message>) -> Result<()> {
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
        let log_path = log_dir.join("oben-tui.log");
        let log_file = std::fs::OpenOptions::new().create(true).append(true).open(log_path)?;
        let subscriber = tracing_subscriber::registry()
            .with(layer().with_writer(log_file));
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
            if event::poll(Duration::from_millis(16)).unwrap() {
                match event::read().unwrap() {
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
        let mut signal = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt()).unwrap();
        let _ = signal.recv().await;
        running_for_signal.store(false, Ordering::SeqCst);
        let _ = event_tx_for_signal.send(quit_ev);
    });

    // Channel for signaling task completion back to the main event loop
    let (done_tx, mut done_rx) = tokio::sync::mpsc::unbounded_channel::<TurnCompletion>();

    // Main event loop — draw at the end of every iteration.
    // The timeout branch ensures we periodically return to draw() even when
    // no events arrive (critical for streaming feedback).
    loop {
        if !app.running {
            break;
        }
        info!(
            "🔄 loop iteration: turn_handle={:?}, streaming_panel={}",
            app.turn_handle.as_ref().map(|_| "some"),
            app.panels.get(&PanelId::Chat)
                .and_then(|p| p.downcast_ref::<ChatPanel>())
                .map_or("".to_string(), |p| format!("{}", p.streaming_text.len())),
        );
        tokio::select! {
            // Timeout: ensures periodic redraw (~30fps) so streaming/tool activity
            // remain visible even when no user events arrive.
            _ = tokio::time::sleep(Duration::from_millis(32)) => {
                if let Ok(ts) = app.turn_state.lock() {
                    info!(
                        "⏱️ timeout: phase={:?}, streaming_text.len={}, active_tools={}",
                        ts.phase,
                        ts.streaming_text.len(),
                        ts.active_tools.len(),
                    );
                    info!(
                        "⏱️ timeout: streaming_text preview='{}'",
                        if ts.streaming_text.len() > 200 {
                            ts.streaming_text.chars().take(200).collect::<String>()
                        } else {
                            ts.streaming_text.clone()
                        },
                    );
                }
            }

            // Check for completion signal from spawned turn task
            maybe_completion = done_rx.recv() => {
                if let Some(completion) = maybe_completion {
                    info!("main_loop: task completed success={}", completion.success);
                    app.turn_handle = None;
                    if completion.success {
                        // Rebuild ChatPanel with new messages
                        let (session_id, messages) = match &app.chat {
                            Some(agent) => {
                                let guard = agent.lock().await;
                                let sid = guard.active_session_name().map(|s| s.clone());
                                let msgs = guard
                                    .session_manager().active_session()
                                    .map(|s| s.messages.clone());
                                (sid, msgs)
                            }
                            None => (None, None),
                        };
                        app.panels.insert(
                            PanelId::Chat,
                            Box::new(ChatPanel::new(session_id, messages)),
                        );
                    } else {
                        app.status = "Turn completed with errors".into();
                    }
                }
            }

            // Event branch: handles key, mouse, and chat input events.
            event = event_rx.recv() => {
                match event {
                    Some(TuiEvent::Key(key)) => {
                        app.handle_key(key);
                    }
                    Some(TuiEvent::Mouse(mouse_event)) => {
                        match mouse_event.kind {
                            MouseEventKind::ScrollUp => {
                                if let Some(panel) = app.panels.get_mut(&PanelId::Chat) {
                                    if let Some(chat) = panel.downcast_mut::<ChatPanel>() {
                                        chat.scroll = chat.scroll.saturating_sub(1);
                                    }
                                }
                            }
                            MouseEventKind::ScrollDown => {
                                if let Some(panel) = app.panels.get_mut(&PanelId::Chat) {
                                    if let Some(chat) = panel.downcast_mut::<ChatPanel>() {
                                        chat.scroll = chat.scroll.saturating_add(1);
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    Some(TuiEvent::ChatInput(input)) => {
                        tracing::info!("📥 event loop: ChatInput received, input.len()={}", input.len());
                        handle_chat_input(&mut app, input, &done_tx).await;
                        tracing::info!("📥 event loop: handle_chat_input returned");
                    }
                    None => break,
                }
            }
        }

        // Single draw per loop iteration — after select! returns for any reason.
        if let Err(e) = terminal.draw(|frame| {
            draw_ui(frame, &mut app);
        }) {
            tracing::warn!("draw error: {}", e);
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

    let Some(agent) = app.chat.as_ref().map(|a| Arc::clone(a)) else {
        app.status = "Agent not initialized".into();
        return;
    };

    if app.turn_handle.is_some() {
        app.status = "Already processing a turn. Please wait...".into();
        return;
    }

    let was_chat = app.active_panel == PanelId::Chat;
    tracing::info!("handle_chat_input: was_chat={}, has_agent={}, turn_handle_some={}", 
        was_chat, app.chat.is_some(), app.turn_handle.is_some());

    // Begin turn tracking
    if let Ok(mut ts) = app.turn_state.lock() {
        ts.on_turn_start();
        tracing::info!("handle_chat_input: turn_state phase set to {:?}", ts.phase);
    }

    // Prepare ChatPanel for streaming
    if was_chat {
        if let Some(panel) = app.panels.get_mut(&PanelId::Chat) {
            if let Some(chat) = panel.downcast_mut::<ChatPanel>() {
                tracing::info!("handle_chat_input: setting ChatPanel.streaming=true");
                chat.streaming = true;
                chat.view_mode = ChatViewMode::Streaming;
                // Append user message immediately so it shows in messages panel right away
                chat.messages.push(crate::panels::chat::ChatMessage {
                    role: "User".to_string(),
                    text: input.clone(),
                    has_tool_calls: false,
                    tool_calls: Vec::new(),
                    tool_results: Vec::new(),
                });
                tracing::info!("handle_chat_input: appended user message to ChatPanel.messages, msg_count={}", chat.messages.len());
            }
        }
    }

    // Spawn turn in background so event loop is not blocked.
    // TokioMutex guard IS Send, so the spawned future can hold the lock
    // across .await in agent.turn().
    let agent_clone = agent;
    let ts_clone_for_callback = Arc::clone(&app.turn_state);
    let ts_clone_for_finalize = Arc::clone(&app.turn_state);
    let done_tx_clone = done_tx.clone();
    let input_clone = input.clone();

    let handle = tokio::spawn({
        tracing::info!("handle_chat_input: tokio::spawn called");
        async move {
            info!("spawned_turn_task: calling agent.turn()");
            let result = {
                let mut guard = agent_clone.lock().await;
                let delta_callback = Box::new(move |text: &str| {
                    tracing::info!("[delta_callback] text.len={} text='{}'", text.len(), text);
                    if let Ok(mut ts) = ts_clone_for_callback.lock() {
                        ts.on_stream_delta(text);
                    } else {
                        tracing::warn!("[delta_callback] FAILED to lock ts_clone_for_callback");
                    }
                });
                guard.turn(&input_clone, false, Some(delta_callback)).await
            };

            tracing::info!("spawned_turn_task: turn completed, is_ok={}", result.is_ok());

            // Finalize turn state
            match &result {
                Ok(_) => {
                    if let Ok(mut ts) = ts_clone_for_finalize.lock() {
                        ts.on_completed("completed");
                        tracing::info!("spawned_turn_task: finalized turn_state phase={:?}", ts.phase);
                    }
                    let _ = done_tx_clone.send(TurnCompletion { success: true });
                    tracing::info!("spawned_turn_task: sent done_tx success");
                }
                Err(e) => {
                    if let Ok(mut ts) = ts_clone_for_finalize.lock() {
                        ts.on_error(&format!("{}", e));
                        tracing::info!("spawned_turn_task: finalized turn_state error: {}", e);
                    }
                    let _ = done_tx_clone.send(TurnCompletion { success: false });
                    tracing::info!("spawned_turn_task: sent done_tx failure");
                }
            }

            tracing::info!("spawned_turn_task: done");
        }
    });

    tracing::info!("handle_chat_input: spawn completed, handle={:?}, about to send done_tx", handle);
    app.turn_handle = Some(handle);
    app.input_history.append(&input);

    if was_chat {
        if let Some(panel) = app.panels.get_mut(&PanelId::Chat) {
            if let Some(chat) = panel.downcast_mut::<ChatPanel>() {
                chat.input.clear();
                chat.cursor = 0;
            }
        }
    }
}

fn draw_ui(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let layout = Layouts::new(area);

    // Inject turn_state_ref into ChatPanel so draw() can read streaming text in real-time
    if let Some(panel) = app.panels.get_mut(&PanelId::Chat) {
        if let Some(chat) = panel.downcast_mut::<ChatPanel>() {
            chat.turn_state_ref = Some(Arc::clone(&app.turn_state));
        }
    }

    // Collect ChatPanel streaming state (after injecting ref)
    let chat_panel_info = app.panels.get(&PanelId::Chat)
        .and_then(|p| p.downcast_ref::<ChatPanel>())
        .map(|cp| format!("streaming={} msg_count={} turn_state_ref={}", cp.streaming, cp.messages.len(), if cp.turn_state_ref.is_some() { "some" } else { "none" }))
        .unwrap_or_else(|| "no_chat_panel".to_string());
    tracing::info!("[draw_ui] chat_panel={}", chat_panel_info);

    let is_streaming = app.panels.get(&PanelId::Chat)
        .and_then(|p| p.downcast_ref::<ChatPanel>())
        .map(|cp| cp.streaming)
        .unwrap_or(false);

    let panel_name = match app.active_panel {
        PanelId::Chat => "Chat",
        PanelId::Sessions => "Sessions",
        PanelId::Config => "Config",
        PanelId::Setup => "Setup",
    };
    let title = format!(" 🦀 ObenAgent TUI | {} ", panel_name);
    let title = format!("{:<width$}", title, width = layout.header.width as usize);
    let title_span = Span::styled(title, Style::default().fg(Color::Cyan).bg(Color::DarkGray));
    let title_para = Paragraph::new(Line::from(title_span));
    frame.render_widget(title_para, layout.header);

    if let Some(panel) = app.panels.get(&app.active_panel) {
        panel.draw(frame, layout.body);
    }

    // Derive session info from stored ChatPanel fields — no Agent locking.
    let (session_name, msg_count) = match app.panels.get(&PanelId::Chat) {
        Some(panel) => {
            if let Some(chat) = panel.downcast_ref::<ChatPanel>() {
                if let Some(ref sid) = chat.session_id {
                    (sid.clone(), chat.messages.len())
                } else {
                    (app.session_id.clone().unwrap_or_default(), 0)
                }
            } else {
                (String::new(), 0)
            }
        }
        None => (String::new(), 0),
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
    let status_text = format!(" F1:Chat  F2:Sessions  F3:Config  F4:Setup  q/Ctrl+C:Quit  ");
    let status_lines: Vec<Line> = vec![
        Line::from(format!(" [{}]  {}", mode_text, session_text)),
        Line::from(status_text),
    ];
    let status_para = Paragraph::new(status_lines);
    let status_area = Rect::new(layout.statusbar.x, layout.statusbar.y, layout.statusbar.width, layout.statusbar.height);
    frame.render_widget(status_para, status_area);
}
