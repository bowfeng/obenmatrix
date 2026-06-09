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
    DisableMouseCapture, EnableMouseCapture, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
    MouseEvent, MouseEventKind,
};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen, size,
};
use crossterm::ExecutableCommand;
use panels::PanelId;
use panels::splash::SplashPanel;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Paragraph, Tabs};
use ratatui::{Frame, Terminal};
use ratatui_toaster::ToastType;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::unbounded_channel;
use tracing::info;

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
    Interrupt,
}

pub async fn run_tui(session_name: Option<&str>) -> Result<()> {
    let app = App::new()?;

    // Raw mode + terminal — needed for splash loop to draw
    enable_raw_mode()?;
    io::stdout().execute(EnterAlternateScreen)?;
    io::stdout().execute(EnableMouseCapture)?;

    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    // Wrap app in Arc<TokioMutex<>> — needed because init task and splash loop
    // both need mutable access to app.panels during splash phase.
    let arc_app = Arc::new(tokio::sync::Mutex::new(app));

    // Configure splash minimum duration from terminal height — ensures at least
    // one full rain drop falls from top to bottom.
    {
        let term_h = terminal.size().unwrap().height;
        let mut a = arc_app.lock().await;
        if let Some(splash) = a.panels.get_mut(&PanelId::Splash)
            .and_then(|p| p.downcast_mut::<SplashPanel>())
        {
            splash.set_min_duration(term_h);
        }
    }

    // --- SPLASH LOOP ---
    // Use oneshot channel so init and draw don't contend for the app mutex.
    let (init_done_tx, mut init_done_rx) = tokio::sync::oneshot::channel::<Result<(), anyhow::Error>>();

    {
        let init_arc_app = Arc::clone(&arc_app);
        let tx = init_done_tx;
        tokio::spawn(async move {
            let mut a = init_arc_app.lock().await;
            let result = a.init_agent().await;
            let _ = tx.send(result);
        });
    }

    let mut interval = tokio::time::interval(Duration::from_millis(32));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        // Draw splash — single lock, ~32ms cadence
        {
            let a = arc_app.lock().await;
            if let Some(splash) = a.panels.get(&PanelId::Splash) {
                terminal.draw(|frame| {
                    splash.draw(frame, frame.area());
                }).ok();
            }
        }

        // Check init result via oneshot (no app lock needed)
        if let Ok(result) = init_done_rx.try_recv() {
            if let Err(e) = result {
                if let Some(splash) = arc_app.lock().await
                    .panels.get_mut(&PanelId::Splash)
                    .and_then(|p| p.downcast_mut::<SplashPanel>())
                {
                    splash.set_error(e.to_string());
                }
            } else {
                arc_app.lock().await.init_active_panel(session_name).await.ok();
            }
        }

        // Break once full fall cycle elapsed
        {
            let a = arc_app.lock().await;
            if let Some(splash) = a.panels.get(&PanelId::Splash)
                .and_then(|p| p.downcast_ref::<SplashPanel>())
            {
                if splash.remaining_min_display() == Duration::ZERO {
                    break;
                }
            }
        }

        interval.tick().await;
    }

    // Post-loop: check if init failed and enter error splash
    if arc_app.lock().await
        .panels.get(&PanelId::Splash)
        .and_then(|p| p.downcast_ref::<SplashPanel>())
        .map(|s| s.error.is_some())
        .unwrap_or(false)
    {
        enter_error_splash(arc_app).await;
    }

    // --- UNWRAP arc_app for main event loop ---
    let mut app = Arc::try_unwrap(arc_app)
        .map_err(|_| anyhow::anyhow!("Arc has extra references after splash phase"))?
        .into_inner();

    // --- MAIN EVENT LOOP SETUP ---
    #[allow(unexpected_cfgs)]
    #[cfg(not(feature = "cli-wired"))]
    {
        // Tracing already initialized
    }

    let (event_tx, mut event_rx) = unbounded_channel();
    let event_tx_for_signal = event_tx.clone();
    app.input_tx = Some(event_tx.clone());

    let running = Arc::new(AtomicBool::new(true));
    let running_clone = Arc::clone(&running);

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

    let running_for_signal = running.clone();
    let quit_ev = TuiEvent::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
    tokio::spawn(async move {
        let mut signal =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt()).unwrap();
        let _ = signal.recv().await;
        running_for_signal.store(false, Ordering::SeqCst);
        let _ = event_tx_for_signal.send(quit_ev);
    });

    let (done_tx, mut done_rx) = tokio::sync::mpsc::unbounded_channel::<TurnCompletion>();

    terminal.draw(|frame| draw_ui(frame, &mut app))?;
    loop {
        if !app.running {
            break;
        }
        let mut redraw = false;
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(32)) => {
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
                let is_splash = app.active_panel == PanelId::Splash;
                if is_streaming || toast_expired || is_splash {
                    let _ = terminal.draw(|frame| draw_ui(frame, &mut app));
                }
            }
            maybe_completion = done_rx.recv() => {
                tracing::info!("[done_rx] recv returned");
                if let Some(completion) = maybe_completion {
                    tracing::info!("[done_rx] success={}, session_name={:?}, messages.len={}", completion.success, completion.session_name, completion.messages.len());
                    app.turn_handle = None;
                    if completion.success {
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
                            chat.streaming = false;
                            chat.input.streaming = false;
                            chat.update_from_messages(&messages, completion.session_name);
                            tracing::info!("[done_rx] chat.update_from_messages done, streaming={}", chat.streaming);
                        }
                    } else {
                        app.status = "Turn completed with errors".into();
                        let eb_state = Arc::clone(&app.event_bus.state());
                        if let Some(chat) = app.get_chat_mut() {
                            chat.streaming = false;
                            chat.input.streaming = false;
                            chat.update_from_turn_state(&eb_state.lock().unwrap());
                        }
                    }
                    redraw = true;
                }
            }
            event = event_rx.recv() => {
                match event {
                    Some(TuiEvent::Key(key)) => {
                        app.handle_key(key).await;
                        redraw = true;
                    }
                    Some(TuiEvent::Mouse(mouse_event)) => {
                        let (term_w, term_h) = size().unwrap_or((80, 24));
                        let body_area = Rect::new(0, 1, term_w, term_h.saturating_sub(2));
                        match mouse_event.kind {
                            MouseEventKind::ScrollUp => {
                                tracing::debug!(
                                    "[mouse] scroll_up: row={}, col={}",
                                    mouse_event.row,
                                    mouse_event.column
                                );
                            }
                            MouseEventKind::ScrollDown => {
                                tracing::debug!(
                                    "[mouse] scroll_down: row={}, col={}",
                                    mouse_event.row,
                                    mouse_event.column
                                );
                            }
                            _ => {}
                        }
                        let click_on_tabs = matches!(mouse_event.kind, MouseEventKind::Down(crossterm::event::MouseButton::Left))
                            && mouse_event.row == 0;
                        if click_on_tabs {
                            let tab_names = ["Chat", "Sessions"];
                            let tab_widths: Vec<usize> = tab_names.iter().map(|n| n.len() + 2).collect();
                            let mut consumed = 0usize;
                            for (i, &pw) in tab_widths.iter().enumerate() {
                                if mouse_event.column >= consumed as u16 && mouse_event.column < (consumed + pw) as u16 {
                                    let target = match i {
                                        0 => PanelId::Chat,
                                        1 => PanelId::Sessions,
                                        _ => break,
                                    };
                                    if app.active_panel != target {
                                        app.activate_panel(target).await;
                                    }
                                    app.active_panel = target;
                                    break;
                                }
                                consumed += pw + 1;
                            }
                            continue;
                        }
                        let current_panel = app.active_panel;
                        if let Some(panel) = app.panels.get_mut(&current_panel) {
                            if let Some(text) = panel.handle_mouse(body_area, &mouse_event) {
                                tracing::debug!("[lib] handle_mouse returned text, about to show toast");
                                let lines = text.lines().count();
                                let msg = if lines == 1 {
                                    "Copied selection.".to_string()
                                } else {
                                    format!("Copied {} lines.", lines)
                                };
                                app.show_toast(msg, ratatui_toaster::ToastType::Success);
                            } else {
                                tracing::debug!("[lib] handle_mouse returned None for event={:?}", mouse_event.kind);
                            }
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
                        if let Some(agent_arc) = &app.agent {
                            let outcome = agent_arc.lock().await.compact_session().await;
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
                            match outcome {
                                oben_agent::compact::CompactOutcome::Compressed { .. } => {
                                    app.show_toast(
                                        "Session context compressed successfully.",
                                        ToastType::Success,
                                    );
                                }
                                oben_agent::compact::CompactOutcome::Ineffective { .. } => {
                                    app.show_toast(
                                        "Compaction skipped: no savings (all messages protected in head/tail zones).",
                                        ToastType::Info,
                                    );
                                }
                                oben_agent::compact::CompactOutcome::AlreadyCompact => {
                                    app.show_toast(
                                        "Session context already within budget.",
                                        ToastType::Info,
                                    );
                                }
                                oben_agent::compact::CompactOutcome::NoMiddleMessages { .. } => {
                                    app.show_toast(
                                        "Compaction skipped: all messages in protected head/tail zones.",
                                        ToastType::Info,
                                    );
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
                    Some(TuiEvent::Interrupt) => {
                        tracing::info!("[interrupt] received, turn_handle={}", app.turn_handle.is_some());
                        // Use Arc<InterruptState> directly — no tokio::sync::Mutex needed,
                        // avoids the deadlock where spawn task holds the lock across turn().
                        app.interrupt_state.request_interrupt(Some("interrupted".to_string()));
                        app.interrupt_state.reset_for_turn();
                        tracing::info!("[interrupt] signal sent via InterruptState");
                        if let Some(handle) = app.turn_handle.take() {
                            handle.abort();
                            tracing::info!("[interrupt] aborted turn handle");
                        }
                        // The user message was inserted into the in-memory session buffer
                        // by handle_chat_input before spawning. Truncate to remove orphaned messages.
                        if let Some(agent) = &app.agent {
                            let message_count = app.turn_message_count;
                            let g = agent.lock().await;
                            if let Some(session) = g.session_manager().lock().await.active_session_mut() {
                                let current_count = session.messages.len();
                                if current_count > message_count {
                                    let removed = current_count - message_count;
                                    session.messages.drain(message_count..);
                                    tracing::info!(
                                        "[interrupt] truncated {} orphaned messages ({} → {})",
                                        removed,
                                        current_count,
                                        message_count
                                    );
                                }
                            }
                        }
                        app.status = "Interrupted".into();
                        if let Some(chat) = app.get_chat_mut() {
                            chat.input.text.clear();
                            chat.input.cursor = 0;
                            chat.streaming = false;
                            chat.input.streaming = false;
                        }
                        app.event_bus.on_turn_completed("interrupted");
                        redraw = true;
                    }
                    None => break,
                }
            }
        }

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

/// Enters error splash mode: draws rain with centered error message forever.
/// Only exits when the user presses Ctrl+C.
async fn enter_error_splash(arc_app: Arc<tokio::sync::Mutex<App>>) -> ! {
    // Create a dedicated terminal for error splash, separate from the one used
    // during the splash loop. Error splash never exits normally — only Ctrl+C.
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.clear().unwrap();

    loop {
        {
            let a = arc_app.lock().await;
            if let Some(splash) = a.panels.get(&PanelId::Splash) {
                terminal.draw(|frame| {
                    splash.draw(frame, frame.area());
                }).ok();
            }
        }

        // Wait + poll for Ctrl+C
        tokio::time::sleep(Duration::from_millis(32)).await;
        if crossterm::event::poll(Duration::from_millis(32)).unwrap_or(false) {
            if let Ok(event) = crossterm::event::read() {
                if let crossterm::event::Event::Key(key) = event {
                    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                        drop(terminal);
                        io::stdout().execute(LeaveAlternateScreen).ok();
                        io::stdout().execute(DisableMouseCapture).ok();
                        disable_raw_mode().ok();
                        std::process::exit(0);
                    }
                }
            }
        }
    }
}
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
            chat.input.streaming = true;
            chat.message_state.turn_state_ref = Some(eb_state);
            chat.append_user_message(&input);
            tracing::info!(
                "handle_chat_input: appended user message to chat, msg_count={}",
                chat.message_count
            );
        }
    }

    // Spawn turn in background so event loop is not blocked.
    // TokioMutex guard IS Send, so the spawned future can hold the lock
    // across .await in agent.turn().
    // Clone InterruptState for spawn closure — avoids deadlocking the
    // interrupt handler which also needs to request_interrupt()
    // without acquiring the tokio::sync::Mutex.
    let agent_clone = agent;
    let eb = Arc::clone(&app.event_bus);
    let eb_for_finalize = Arc::clone(&app.event_bus);
    let done_tx_clone = done_tx.clone();
    let input_clone = input.clone();
    let interrupt_clone = Arc::clone(&app.interrupt_state);

    // Record session message count from the session manager BEFORE the
    // spawned task inserts the user message via execute_turn_with_options.
    // If the task is aborted, those orphaned messages must be truncated.
    // Lock dropped before spawn to avoid deadlock.
    if was_chat {
        if let Some(agent) = &app.agent {
            let g = agent.lock().await;
            app.turn_message_count = g
                .session_manager()
                .lock()
                .await
                .active_session()
                .map(|s| s.messages.len())
                .unwrap_or(0);
        }
    }

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
                let result = guard.turn(&input_clone, false, Some(delta_callback), Some(Arc::clone(&interrupt_clone))).await;
                let sid = guard.active_session_name().await.map(|s| s.clone());
                let msgs = guard
                    .session_manager()
                    .lock()
                    .await
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
                    let _ = done_tx_clone.send(TurnCompletion {
                        success: true,
                        session_name: sid,
                        messages,
                    });
                    tracing::info!("spawned_turn_task: sent done_tx success");
                }
                Err(e) => {
                    eb_for_finalize.on_turn_error(&format!("{}", e));
                    tracing::info!("spawned_turn_task: finalized turn_state error: {}", e);
                    let _ = done_tx_clone.send(TurnCompletion {
                        success: false,
                        session_name: None,
                        messages: Vec::new(),
                    });
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

    let panel_names: [&str; 2] = ["Chat", "Sessions"];
    let panel_index = match app.active_panel {
        PanelId::Chat => 0,
        PanelId::Sessions => 1,
        _ => 0,
    };
    let tabs = Tabs::new(panel_names)
        .style(Style::default().fg(Color::DarkGray))
        .highlight_style(Style::default().fg(Color::Cyan).bold())
        .divider(" ")
        .select(panel_index)
        .block(Block::default().style(Style::default().bg(Color::Blue)));
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
        PanelId::Sessions => match app.get_sessions() {
            Some(sessions) => (
                sessions.get_session_name().unwrap_or_default(),
                sessions.get_message_count().unwrap_or(0),
            ),
            None => (String::new(), 0),
        },
        _ => match app.get_chat() {
            Some(chat) => {
                if let Some(ref sid) = chat.session_name {
                    (sid.clone(), chat.message_count)
                } else {
                    (
                        app.session_id.clone().unwrap_or_default(),
                        chat.message_count,
                    )
                }
            }
            None => (String::new(), 0),
        },
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
    let status_lines: Vec<Line> = vec![Line::from(format!(" [{}]  {}", mode_text, session_text))];
    let status_para = Paragraph::new(status_lines);
    let status_area = Rect::new(
        layout.statusbar.x,
        layout.statusbar.y,
        layout.statusbar.width,
        layout.statusbar.height,
    );
    frame.render_widget(status_para, status_area);
}
