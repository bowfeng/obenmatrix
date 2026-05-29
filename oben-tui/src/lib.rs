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
use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind};
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen, enable_raw_mode, disable_raw_mode};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

use ratatui::Frame;
use ratatui::Terminal;
use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc::unbounded_channel;
use tracing::info;
use tracing_subscriber::{fmt::layer, EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use panels::chat::ChatPanel;
use panels::config::ConfigPanel;
use panels::setup::SetupPanel;
use panels::sessions::SessionsPanel;
use panels::{Panel, PanelId};

use turn::event::TurnState;

use oben_config::AppConfig;
use oben_agent::{Agent, AgentConfig};
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

pub(crate) enum TuiEvent {
    Key(KeyEvent),
    ChatInput(String),
    Mouse(MouseEvent),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusMode {
    Ready,
    Streaming,
    Error,
    ToolRunning,
}

pub struct App {
    pub running: bool,
    pub active_panel: PanelId,
    pub panels: HashMap<PanelId, Box<dyn Panel>>,
    pub status: String,
    pub config: AppConfig,
    pub chat: Option<Agent>,
    pub session_id: Option<String>,
    pub tools: std::sync::Arc<ToolRegistry>,
    pub tool_names: Vec<String>,
    pub input_tx: Option<tokio::sync::mpsc::UnboundedSender<TuiEvent>>,
    pub input_history: history::InputHistory,
    pub paste_mode: bool,
    pub turn_state: Arc<Mutex<turn::event::TurnState>>,
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
            session_id: None,
            tools: std::sync::Arc::new(tools),
            tool_names,
            input_tx: None,
            input_history: history::InputHistory::new(),
            paste_mode: false,
            turn_state: Arc::new(Mutex::new(turn::event::TurnState::new())),
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

        let turn_state = Arc::new(Mutex::new(turn::event::TurnState::new()));
        let turn_state_clone = Arc::clone(&turn_state);

        let callbacks = oben_agent::AgentCallbacks {
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
                Some(Box::new(move |tool_name: &str, args_json: &str, result: &str| {
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

        self.chat = Some(Agent::new(AgentConfig {
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
        })?);

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

    pub fn create_chat_panel(&mut self) {
        let session_id = self.chat.as_ref().and_then(|c| c.active_session_name().map(|s| s.clone()));
        let messages = self.chat.as_ref().and_then(|c| {
            c.session_manager().active_session().map(|s| s.messages.clone())
        });
        self.panels.insert(
            PanelId::Chat,
            Box::new(ChatPanel::new(session_id, messages)),
        );
    }

    pub fn create_sessions_panel(&mut self) {
        let sessions: Vec<oben_models::Session> = match &self.chat {
            Some(chat) => chat.session_manager().list_sessions_full(),
            None => vec![],
        };
        self.panels.insert(PanelId::Sessions, Box::new(SessionsPanel::new(sessions)));
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

    pub fn init_panels(&mut self) -> Result<()> {
        self.create_chat_panel();
        self.create_sessions_panel();
        self.create_config_panel();
        self.create_setup_panel();
        Ok(())
    }

    pub fn update_session_messages(&mut self, messages: Vec<Message>) -> Result<()> {
        let _ = messages;
        Ok(())
    }
}

pub async fn run_tui() -> Result<()> {
    let mut app = App::new()?;
    app.init_chat()?;
    app.init_panels()?;

    // Only set up file logging if no subscriber has been initialized yet
    // (e.g. when TUI is run directly without going through oben-cli)
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
    app.input_tx = Some(event_tx.clone());

    let running = Arc::new(AtomicBool::new(true));
    let running_clone = Arc::clone(&running);

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

    while app.running {
        // Draw first to avoid waiting for first event before showing UI
        terminal.draw(|frame| {
            draw_ui(frame, &mut app);
        })?;

        match event_rx.recv().await {
            Some(TuiEvent::Key(key)) => {
                handle_key(&mut app, key);
            }
            Some(TuiEvent::Mouse(mouse_event)) => {
                if let MouseEventKind::ScrollUp = mouse_event.kind {
                    if let Some(panel) = app.panels.get_mut(&PanelId::Chat) {
                        if let Some(chat) = panel.downcast_mut::<ChatPanel>() {
                            chat.scroll = chat.scroll.saturating_sub(1);
                        }
                    }
                } else if let MouseEventKind::ScrollDown = mouse_event.kind {
                    if let Some(panel) = app.panels.get_mut(&PanelId::Chat) {
                        if let Some(chat) = panel.downcast_mut::<ChatPanel>() {
                            chat.scroll = chat.scroll.saturating_add(1);
                        }
                    }
                }
            }
            Some(TuiEvent::ChatInput(input)) => {
                if let Some(ref mut chat) = app.chat {
                    // Begin turn tracking
                    app.turn_state.lock().unwrap().on_turn_start();

                    // Preserve Input state across ChatPanel rebuild.
                    let was_chat = app.active_panel == PanelId::Chat;
                    let saved_input = app.panels.get(&PanelId::Chat).and_then(|p| {
                        p.downcast_ref::<ChatPanel>().map(|cp| {
                            (cp.input.clone(), cp.cursor, cp.last_enter_time, cp.streaming)
                        })
                    });

                    match chat.turn(&input, false, None).await {
                        Ok(_) => {
                            // Finalize turn on success
                            app.turn_state.lock().unwrap().on_completed("completed");
                            
                            if was_chat {
                                let session_id = app.chat.as_ref().and_then(|c| c.active_session_name().map(|s| s.clone()));
                                let messages = app.chat.as_ref().and_then(|c| {
                                    c.session_manager().active_session().map(|s| s.messages.clone())
                                });
                                // Extract tool trail before messages are moved into ChatPanel
                                let trail_msgs = messages.clone();
                                let mut new_panel = ChatPanel::new(session_id, messages);
                                if let Some((inp, cursor, enter, streaming)) = saved_input {
                                    new_panel.input = inp;
                                    new_panel.cursor = cursor;
                                    new_panel.last_enter_time = enter;
                                    new_panel.streaming = streaming;
                                }
                                // Build tool trail from session messages
                                if let Some(ref msgs) = trail_msgs {
                                    new_panel.extract_tool_trail(msgs);
                                }
                                app.panels.insert(PanelId::Chat, Box::new(new_panel));
                            }
                        }
                        Err(e) => {
                            // Finalize turn on error
                            app.turn_state.lock().unwrap().on_error(&format!("{}", e));
                            app.status = format!("Error: {}", e);
                            info!("Agent turn error: {}", e);
                        }
                    }
                }
            }
            None => break,
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

fn handle_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.running = false;
            return;
        }
        KeyCode::F(1) => { app.active_panel = PanelId::Chat; return; }
        KeyCode::F(2) => { app.active_panel = PanelId::Sessions; return; }
        KeyCode::F(3) => { app.active_panel = PanelId::Config; return; }
        KeyCode::F(4) => { app.active_panel = PanelId::Setup; return; }
        _ => {}
    }
    if let Some(_panel) = app.panels.get_mut(&app.active_panel) {
        let panel_id = app.active_panel;
        if let Some(boxed_panel) = app.panels.remove(&panel_id) {
            let mut panel = boxed_panel;
            panel.handle_key(app, key);
            app.panels.insert(panel_id, panel);
        }
    }
}

fn draw_ui(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let layout = Layouts::new(area);

    // Collect ChatPanel streaming state
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

    let session_info = match app.chat.as_ref().and_then(|c| c.session_manager().active_session()) {
        Some(s) => format!(" Session: {} ({} msgs)", s.name, s.messages.len()),
        None => " No session".to_string(),
    };

    let session_text = format!(" Session: {}", session_info);
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
