//! ObenAgent TUI — a full terminal UI for chat, sessions, config, and setup.
//!
//! Replaces the CLI-based `oben chat`, `oben setup`, `oben config`, and
//! `oben sessions` with a ratatui-driven interface.

pub mod app;
pub mod clipboard;
pub mod commands;
pub mod event;
pub mod history;
pub mod panels;
pub mod turn;
pub mod widgets;

pub use app::App;

use anyhow::Result;
use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent,
    MouseEventKind,
};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use panels::PanelId;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Paragraph, Tabs};
use ratatui_toaster::ToastType;
use ratatui::{Frame, Terminal};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::unbounded_channel;
use tracing::info;
use tracing_subscriber::{
    fmt::layer,
    layer::SubscriberExt,
    util::SubscriberInitExt,
};

use crate::app::TurnCompletion;

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
            Constraint::Length(1),
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
    CompactSession,
    Mouse(MouseEvent),
    Resize(u16, u16),
}

pub async fn run_tui(session_name: Option<&str>) -> Result<()> {
    let mut app = App::new()?;
    app.init_agent().await?;
    app.init_active_panel(session_name).await?;
    
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
                    crossterm::event::Event::Resize(w, h) => {
                        let _ = event_tx.send(TuiEvent::Resize(w, h));
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
            // Timeout: always check toast expiry so toasts auto-hide even when idle.
            // Only redraw during streaming so live text remains visible.
            _ = tokio::time::sleep(Duration::from_millis(32)) => {
                // Check toast expiry even if not streaming
                let toast_expired = if let Some(expiry) = app.toast_expires_at {
                    if std::time::Instant::now() >= expiry {
                        app.toast_engine.hide_toast();
                        app.toast_expires_at = None;
                        true
                    } else {
                        false
                    }
                } else {
                    false
                };
                let is_streaming = app.get_chat().map(|cp| cp.streaming).unwrap_or(false);
                if is_streaming || toast_expired {
                    let _ = terminal.draw(|frame| draw_ui(frame, &mut app));
                }
            }

            // Check for completion signal from spawned turn task
            maybe_completion = done_rx.recv() => {
                tracing::info!("[done_rx] recv returned");
                if let Some(completion) = maybe_completion {
                    tracing::info!("[done_rx] success={}, session_name={:?}, messages.len={}", completion.success, completion.session_name, completion.messages.len());
                    app.turn_handle = None;
                    if completion.success {
                        // Read messages from the agent's current session instead of the
                        // stale snapshot carried in TurnCompletion. This prevents /clear
                        // (which resets the agent session and clears the chat) from being
                        // undone when a previously-spawned turn finally completes.
                        let messages = if let Some(agent) = &app.agent {
                            let guard = agent.lock().await;
                            let count = guard
                                .session_manager().lock().await
                                .active_session()
                                .map(|s| s.messages.len())
                                .unwrap_or(0);
                            tracing::info!("[done_rx] agent session has {} messages after lock", count);
                            guard
                                .session_manager().lock().await
                                .active_session()
                                .map(|s| s.messages.clone())
                                .unwrap_or_default()
                        } else {
                            Vec::new()
                        };
                        tracing::info!("[done_rx] messages to display: {}, calling update_from_messages", messages.len());
                        if let Some(chat) = app.get_chat_mut() {
                            tracing::info!("[done_rx] chat.message_count before update: {}", chat.message_count);
                            chat.update_from_messages(&messages, completion.session_name);
                            tracing::info!("[done_rx] chat.update_from_messages done, streaming={}", chat.streaming);
                        }
                    } else {
                        app.status = "Turn completed with errors".into();
                        let eb_state = Arc::clone(&app.event_bus.state());
                        if let Some(chat) = app.get_chat_mut() {
                            chat.streaming = false;
                            chat.update_from_turn_state(&eb_state.lock().unwrap());
                        }
                    }
                    redraw = true;
                }
            }

            // Event branch: handles key, mouse, and chat input events.
            event = event_rx.recv() => {
                match event {
                    Some(TuiEvent::Key(key)) => {
                        app.handle_key(key).await;
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
                        } else if let Some(panel) = app.panels.get_mut(&app.active_panel) {
                            panel.handle_mouse(&mouse_event);
                            redraw = true;
                        }
                    }
                    Some(TuiEvent::ChatInput(input)) => {
                        handle_chat_input(&mut app, input, &done_tx).await;
                        redraw = true;
                    }
                    Some(TuiEvent::Resize(_w, _h)) => {
                        redraw = true;
                    }
                    Some(TuiEvent::CompactSession) => {
                        // Execute compact directly in the main loop.
                        // This avoids an infinite self-loop that would occur
                        // if we re-sent the event via input_tx.
                        if let Some(agent_arc) = &app.agent {
                            let result = agent_arc.lock().await.compact_session().await;
                            match result {
                                Ok(()) => {
                                    // After compression, reload active session messages
                                    // into the ChatPanel display.
                                    let sid = app.session_id.clone();
                                    let messages = if let Some(agent_arc) = &app.agent {
                                        let guard = agent_arc.lock().await;
                                        guard
                                            .session_manager().lock().await
                                            .active_session()
                                            .map(|s| s.messages.clone())
                                            .unwrap_or_default()
                                    } else {
                                        Vec::new()
                                    };
                                    if let Some(chat) = app.get_chat_mut() {
                                        chat.update_from_messages(&messages, sid);
                                    }
                                    app.show_toast(
                                        "Session context compressed.",
                                        ToastType::Success,
                                    );
                                }
                                Err(e) => {
                                    app.show_toast(
                                        format!("Compact failed: {e}"),
                                        ToastType::Error,
                                    );
                                    tracing::error!("Context compression failed: {e}");
                                }
                            }
                        } else {
                            app.show_toast(
                                "Cannot compact: agent not initialized",
                                ToastType::Warning,
                            );
                        }
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
        let eb_state = Arc::clone(&app.event_bus.state());
        if let Some(chat) = app.get_chat_mut() {
            tracing::info!("handle_chat_input: setting ChatPanel.streaming=true");
            chat.streaming = true;
            chat.message_state.turn_state_ref = Some(eb_state);
            chat.append_user_message(&input);
            tracing::info!("handle_chat_input: appended user message to chat, msg_count=0");
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
                let sid = guard.active_session_name().await.map(|s| s.clone());
                let msgs = guard
                    .session_manager().lock().await
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
        if let Some(chat) = app.get_chat_mut() {
            chat.input.text.clear();
            chat.input.cursor = 0;
        }
    }
}

fn draw_ui(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let layout = Layouts::new(area);

    // Inject turn_state_ref into ChatPanel so draw() can read streaming text in real-time
    // Clone event_bus state Arc first to avoid borrowing app twice
    let eb_state = Arc::clone(&app.event_bus.state());
    if let Some(chat) = app.get_chat_mut() {
        chat.set_turn_state_ref(Arc::clone(&eb_state));
        chat.update_from_turn_state(&eb_state.lock().unwrap());
    }

    // Collect ChatPanel streaming state (after injecting ref)
    let chat_panel_info = app
        .get_chat()
        .map(|cp| format!("streaming={}", cp.streaming))
        .unwrap_or("no_chat_panel".to_string());
    tracing::debug!("[draw_ui] chat_panel={}", chat_panel_info);

    let is_streaming = app.get_chat().map(|cp| cp.streaming).unwrap_or(false);

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

    // Toast area: top-right of the body region, 2 lines tall.
    let toast_height = 2u16;
    let toast_width: u16 = 50;
    let toast_rect = Rect::new(
        layout.body.x + layout.body.width.saturating_sub(toast_width),
        layout.body.y,
        toast_width,
        toast_height,
    );
    app.toast_engine.set_area(toast_rect);

    // Render toast above the status bar if one is active.
    // Auto-expire after 3 seconds (ratatui-toaster has no built-in timeout).
    let has_active_toast = if let Some(expiry) = app.toast_expires_at {
        if std::time::Instant::now() >= expiry {
            app.toast_engine.hide_toast();
            app.toast_expires_at = None;
            false
        } else {
            app.toast_engine.has_toast()
        }
    } else {
        app.toast_engine.has_toast()
    };
    if has_active_toast {
        frame.render_widget(&app.toast_engine, toast_rect);
    }

    // Derive session info from stored ChatPanel fields — no Agent locking.
    let (session_name, msg_count) = match app.active_panel {
        PanelId::Sessions => {
            match app.get_sessions() {
                Some(sessions) => (sessions.get_session_name().unwrap_or_default(),
                                   sessions.get_message_count().unwrap_or(0)),
                None => (String::new(), 0),
            }
        }
        _ => {
            match app.get_chat() {
                Some(chat) => {
                    if let Some(ref sid) = chat.session_name {
                        (sid.clone(), chat.message_count)
                    } else {
                        (app.session_id.clone().unwrap_or_default(), chat.message_count)
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
