//! ObenAgent TUI — a full terminal UI for chat, sessions, config, and setup.
//!
//! Replaces the CLI-based `oben chat`, `oben setup`, `oben config`, and
//! `oben sessions` with a ratatui-driven interface.

pub mod panels;
pub mod widgets;

use anyhow::Result;
use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
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
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::unbounded_channel;
use tracing::info;

use panels::chat::ChatPanel;
use panels::config::ConfigPanel;
use panels::setup::SetupPanel;
use panels::sessions::SessionsPanel;
use panels::{Panel, PanelId};

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
        self.chat = Some(Agent::new(AgentConfig {
            system_prompt: assembled.prompt,
            transport,
            tools: std::sync::Arc::clone(&self.tools),
            skills_dirs: vec![],
            max_iterations: self.config.max_iterations.unwrap_or(50),
            max_messages: self.config.context.max_messages.unwrap_or(100),
            context_config: oben_agent::CompactCofig::default(),
            fallback_models: vec![],
            callbacks: oben_agent::AgentCallbacks::default(),
            concurrent_dispatch_config: oben_agent::ConcurrentDispatchConfig::default(),
            nudge_config: None,
        })?);
        Ok(())
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
                if let crossterm::event::Event::Key(key) = event::read().unwrap() {
                    if key.kind == KeyEventKind::Press {
                        let _ = event_tx.send(TuiEvent::Key(key));
                    }
                }
            }
        }
    });

    while app.running {
        terminal.draw(|frame| {
            draw_ui(frame, &app);
        })?;

        match event_rx.recv().await {
            Some(TuiEvent::Key(key)) => {
                handle_key(&mut app, key);
            }
            Some(TuiEvent::ChatInput(input)) => {
                // Run turn directly in async context (no spawn needed)
                if let Some(ref mut chat) = app.chat {
                    match chat.turn(&input, false, None).await {
                        Ok(_) => {
                            if app.active_panel == PanelId::Chat {
                                let session_id = app.chat.as_ref().and_then(|c| c.active_session_name().map(|s| s.clone()));
                                let messages = app.chat.as_ref().and_then(|c| {
                                    c.session_manager().active_session().map(|s| s.messages.clone())
                                });
                                app.panels.insert(
                                    PanelId::Chat,
                                    Box::new(ChatPanel::new(session_id, messages)),
                                );
                            }
                        }
                        Err(e) => {
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

fn draw_ui(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let layout = Layouts::new(area);

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

    let status_text = format!(
        " F1:Chat  F2:Sessions  F3:Config  F4:Setup  q/Ctrl+C:Quit  {} ",
        session_info
    );
    let status_span = Span::styled(
        status_text,
        Style::default().fg(Color::White).bg(Color::DarkGray),
    );
    let status_para = Paragraph::new(Line::from(status_span));
    frame.render_widget(status_para, layout.statusbar);
}
