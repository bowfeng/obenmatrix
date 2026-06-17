//! ObenAgent TUI — a full terminal UI for chat, sessions, config, and setup.
//!
//! Replaces the CLI-based `oben chat`, `oben setup`, `oben config`, and
//! `oben sessions` with a ratatui-driven interface.

pub mod app;
pub mod clipboard;
pub mod commands;
pub mod coordinator;
pub mod history;
pub mod image;
pub mod panels;
pub mod widgets;

pub use app::App;

use anyhow::Result;
use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
    MouseEvent, MouseEventKind,
};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, size, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use oben_models::{Message, MessageContent, MessagePart};
use panels::splash::SplashPanel;
use panels::PanelId;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Paragraph, Tabs};
use ratatui::{Frame, Terminal};
use ratatui_toaster::ToastType;
use regex::Regex;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::app::TurnCompletion;
use tokio::sync::mpsc::unbounded_channel;
use tracing::info;
use oben_sessions::SessionManager;

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
    /// Inject a message into the next tool call without interrupting.
    /// (Mirrors `/steer` from the Hermes CLI.)
    Steer(String),
    /// Signal the event loop to dequeue messages from ChatPanel's input queue
    /// and submit them as ChatInput events. Replaces the old direct TuiEvent::ChatInput
    /// sends from ChatPanel for auto-drain.
    QueueDrain,
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
        if let Some(splash) = a
            .panels
            .get_mut(&PanelId::Splash)
            .and_then(|p| p.downcast_mut::<SplashPanel>())
        {
            splash.set_min_duration(term_h);
        }
    }

    // --- SPLASH LOOP ---
    // Use oneshot channel so init and draw don't contend for the app mutex.
    let (init_done_tx, mut init_done_rx) =
        tokio::sync::oneshot::channel::<Result<(), anyhow::Error>>();

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
                terminal
                    .draw(|frame| {
                        splash.draw(frame, frame.area());
                    })
                    .ok();
            }
        }

        // Check init result via oneshot (no app lock needed)
        if let Ok(result) = init_done_rx.try_recv() {
            if let Err(e) = result {
                if let Some(splash) = arc_app
                    .lock()
                    .await
                    .panels
                    .get_mut(&PanelId::Splash)
                    .and_then(|p| p.downcast_mut::<SplashPanel>())
                {
                    splash.set_error(e.to_string());
                }
            } else {
                arc_app
                    .lock()
                    .await
                    .init_active_panel(session_name)
                    .await
                    .ok();
            }
        }

        // Break once full fall cycle elapsed
        {
            let a = arc_app.lock().await;
            if let Some(splash) = a
                .panels
                .get(&PanelId::Splash)
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
    if arc_app
        .lock()
        .await
        .panels
        .get(&PanelId::Splash)
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
    // Wire input_sender into ChatPanel for queue auto-drain.
    if let Some(chat) = app.get_chat_mut() {
        chat.set_input_sender(event_tx.clone());
    }

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

    // Initial draw — needs_redraw starts as true from App::new()
    if app.needs_redraw {
        terminal.draw(|frame| draw_ui(frame, &mut app))?;
    }
    loop {
        if !app.running {
            break;
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(32)) => {
                // Check and hide expired toasts.
                if let Some(expiry) = app.toast_expires_at {
                    if std::time::Instant::now() >= expiry {
                        app.toast_engine.hide_toast();
                        app.toast_expires_at = None;
                        app.needs_redraw = true;
                    }
                }
                // During streaming, set needs_redraw so the 32ms timer paints
                // live updates without relying on user events.  During idle
                // (no turn active, nothing marked dirty) skip the draw entirely
                // — this eliminates the ~300 draws/sec from mouse Moved events.
                let is_streaming = app.get_chat().map(|c| c.streaming).unwrap_or(false);
                if is_streaming {
                    app.needs_redraw = true;
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
                            let sm_arc = guard.session_manager();
                            let sm_guard = sm_arc.lock().await;
                            let count = guard
                                .context_window_manager()
                                .session_id()
                                .and_then(|sid| sm_guard.session(&sid))
                                .map(|s| s.messages.len())
                                .unwrap_or(0);
                            tracing::info!("[done_rx] agent session has {} messages after lock", count);
                            guard
                                .context_window_manager()
                                .session_id()
                                .and_then(|sid| sm_guard.session(&sid))
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
                            app.needs_redraw = true;
                        }
                    } else {
                        app.status = "Turn completed with errors".into();
                        if let Some(chat) = app.get_chat_mut() {
                            chat.streaming = false;
                            chat.input.streaming = false;
                            chat.update_from_turn_state();
                            app.needs_redraw = true;
                        }
                    }
                }
            }
            event = event_rx.recv() => {
                match event {
                    Some(TuiEvent::Key(key)) => {
                tracing::info!("[event_loop] Key event: code={:?} modifiers={:?}", key.code, key.modifiers);
                app.handle_key(key).await;
                app.needs_redraw = true;
            }
            Some(TuiEvent::Mouse(mouse_event)) => {
                        // Check for expired toasts on mouse move so UI updates
                        // promptly when a toast expires during cursor hover.
                        if let Some(expiry) = app.toast_expires_at {
                            if std::time::Instant::now() >= expiry {
                                app.toast_engine.hide_toast();
                                app.toast_expires_at = None;
                                app.needs_redraw = true;
                            }
                        }
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
                            app.needs_redraw = true;
                            continue;
                        }
                        let current_panel = app.active_panel;
                        if let Some(panel) = app.panels.get_mut(&current_panel) {
                            match mouse_event.kind {
                                MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                                    // Forward scroll events to the panel so it can update
                                    // its scroll state (chat.rs::handle_mouse toggles
                                    // scroll_to_bottom and adjusts user_scroll_offset).
                                    let _ = panel.handle_mouse(body_area, &mouse_event);
                                    app.needs_redraw = true;
                                }
                                _ => {
                                    if let Some(text) = panel.handle_mouse(body_area, &mouse_event) {
                                        // Copy to clipboard
                                        tracing::debug!("[lib] handle_mouse returned text, about to show toast");
                                        let lines = text.lines().count();
                                        let msg = if lines == 1 {
                                            "Copied selection.".to_string()
                                        } else {
                                            format!("Copied {} lines.", lines)
                                        };
                                        app.show_toast(msg, ratatui_toaster::ToastType::Success);
                                        app.needs_redraw = true;
                                    } else {
                                        tracing::debug!("[lib] handle_mouse returned None for event={:?}", mouse_event.kind);
                                    }
                                    // Selection drag needs redraw so visible_body_ranges gets updated
                                    // and render_selection can draw the highlight correctly.
                                    if let Some(chat) = app.get_chat() {
                                        let selecting = chat.message_state.selection_start.is_some()
                                            && chat.message_state.selection_end.is_some();
                                        if selecting {
                                            app.needs_redraw = true;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Some(TuiEvent::ChatInput(input)) => {
                        tracing::debug!("[event_loop] ChatInput event received, input.len()={}", input.len());
                        handle_chat_input(&mut app, input, &done_tx).await;
                        tracing::debug!("[event_loop] ChatInput processed");
                        app.needs_redraw = true;
                    }
                    Some(TuiEvent::QueueDrain) => {
                        // Auto-drain from ChatPanel triggered this event.
                        // The event loop owns app state, so it dequeues and
                        // handles it directly.  Collect messages first to
                        // avoid holding a mutable borrow across .await.
                        let messages: Vec<String> = std::mem::take(&mut app.panels
                            .get_mut(&crate::panels::PanelId::Chat)
                            .unwrap()
                            .downcast_mut::<crate::panels::chat::ChatPanel>()
                            .unwrap()
                            .input
                            .input_queue);
                        for msg in messages {
                            tracing::info!(
                                "[event_loop] QueueDrain: draining msg: {}",
                                msg
                            );
                            handle_chat_input(&mut app, msg, &done_tx)
                                .await;
                        }
                        app.needs_redraw = true;
                    }
                    Some(TuiEvent::Resize(_w, _h)) => {
                        app.needs_redraw = true;
                    }
                    Some(TuiEvent::CompactSession) => {
                        if let Some(agent_arc) = &app.agent {
                            let outcome = agent_arc.lock().await.compact_session().await;
                            let sid = app.session_id.clone();
                            let messages = if let Some(agent_arc) = &app.agent {
                                let guard = agent_arc.lock().await;
                                let sm_arc = guard.session_manager();
                                let sm_guard = sm_arc.lock().await;
                                guard
                                    .context_window_manager()
                                    .session_id()
                                    .and_then(|s| sm_guard.session(&s))
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
                        app.needs_redraw = true;
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
                            let sm_arc = g.session_manager();
                            let mut sm = sm_arc.lock().await;
                            if let Some(session) = app
                                .session_id
                                .as_ref()
                                .and_then(|sid| sm.session_mut(sid))
                            {
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
                        let mut ts = app.turn_state.lock();
                        ts.on_interrupted();
                        ts.on_completed("interrupted");
                        app.needs_redraw = true;
                    }
                    Some(TuiEvent::Steer(text)) => {
                        // Inject a mid-run message into the next tool result.
                        // (Mirrors `/steer` from the Hermes CLI.)
                        if let Some(agent) = &app.agent {
                            let accepted = agent.lock().await.steer(&text);
                            if accepted {
                                tracing::info!(
                                    "[steer] queued: {text}",
                                );
                                app.show_toast(
                                    format!("Steer queued — arrives after next tool call"),
                                    ToastType::Info,
                                );
                            } else {
                                app.show_toast(
                                    "Steer rejected (empty payload).",
                                    ToastType::Warning,
                                );
                            }
                        } else {
                            // No active run — fall back to queue semantics
                            if let Some(chat) = app.get_chat_mut() {
                                chat.input.enqueue_msg(text.clone());
                                tracing::info!(
                                    "[steer] no agent — queued message instead: {text}",
                                );
                                app.show_toast(
                                    "Agent not running — queued for next turn",
                                    ToastType::Info,
                                );
                            }
                        }
                        // Only display steer text as a chat message on the first Ctrl+Enter
                        // of a batch. Subsequent steers in the same turn are suppressed.
                        if let Some(chat) = app.get_chat_mut() {
                            if chat.input.pending_steer_count == 0 {
                                chat.append_user_message(&text);
                                chat.input.pending_steer_count += 1;
                            }
                        }
                        app.needs_redraw = true;
                    }
                    None => break,
                }
            }
        }

        if app.needs_redraw {
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
                terminal
                    .draw(|frame| {
                        splash.draw(frame, frame.area());
                    })
                    .ok();
            }
        }

        // Wait + poll for Ctrl+C
        tokio::time::sleep(Duration::from_millis(32)).await;
        if crossterm::event::poll(Duration::from_millis(32)).unwrap_or(false) {
            if let Ok(event) = crossterm::event::read() {
                if let crossterm::event::Event::Key(key) = event {
                    if key.code == KeyCode::Char('c')
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                    {
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

/// Detect image URLs and local image file paths in the input text, then build
/// the appropriate [`Message`].
fn build_image_message(input: &str) -> oben_models::Message {
    // 1. Collect local image paths (tokens that start with `/` and end with an
    // image extension, e.g. `/Users/foo/photo.png`).
    let known_exts = [
        ".jpg", ".jpeg", ".png", ".gif", ".webp", ".svg", ".bmp", ".tiff", ".tif", ".ico", ".avif",
    ];

    let tokens: Vec<&str> = input.split_whitespace().collect();
    let mut image_tokens: Vec<String> = Vec::new();
    let mut text_tokens: Vec<&str> = Vec::new();

    for token in tokens {
        let is_image = known_exts
            .iter()
            .any(|ext| token.to_lowercase().ends_with(ext));
        if is_image && token.starts_with('/') {
            // Try to read and encode the image
            if let Some((msg, _)) = image::path_to_image_message(token, "") {
                let url = match &msg.content {
                    MessageContent::Image { url, .. } => url.clone(),
                    MessageContent::Parts(parts) => parts
                        .iter()
                        .find_map(|p| match p {
                            MessagePart::Image { url, .. } => Some(url.clone()),
                            _ => None,
                        })
                        .unwrap_or_else(|| String::new()),
                    _ => String::new(),
                };
                if !url.is_empty() {
                    image_tokens.push(url);
                }
            }
        } else {
            text_tokens.push(token);
        }
    }

    if !image_tokens.is_empty() {
        // Mix of text and/or images — collect any non-image text
        let text: String = text_tokens.join(" ");
        let text_trimmed = text.trim();

        if text_trimmed.is_empty() && image_tokens.len() == 1 {
            // Just one image, no surrounding text — use Image variant
            return Message {
                role: oben_models::MessageRole::User,
                content: MessageContent::Image {
                    url: image_tokens[0].clone(),
                    detail: None,
                },
                id: None,
                tool_call_ids: vec![],
                tool_calls: None,
                reasoning: None,
            };
        }

        // Build Parts: text (if any) followed by images
        let mut parts: Vec<MessagePart> = Vec::new();
        if !text_trimmed.is_empty() {
            parts.push(MessagePart::Text(text_trimmed.to_string()));
        }
        for url in image_tokens {
            parts.push(MessagePart::Image { url, detail: None });
        }

        return Message {
            role: oben_models::MessageRole::User,
            content: MessageContent::Parts(parts),
            id: None,
            tool_call_ids: vec![],
            tool_calls: None,
            reasoning: None,
        };
    }

    // 2. Check for image URLs (http/https)
    static IMAGE_URL_RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        // Match http/https URLs ending with image extensions.
        // We deliberately do not include quotes or angle brackets in the
        // negated set — URLs never contain them, and keeping them out
        // avoids Rust raw-string escaping issues.
        Regex::new(r"https?://[^\s'(),]+(?:\.(?:jpg|jpeg|png|gif|webp|svg|bmp|tiff?|ico|avif))([^\s'()]*)?").unwrap()
    });

    let urls: Vec<(usize, String)> = IMAGE_URL_RE
        .captures_iter(input)
        .filter_map(|cap| cap.get(0).map(|m| (m.start(), m.as_str().to_string())))
        .collect();

    if urls.is_empty() {
        // No images — fall back to plain text message
        return oben_models::Message::user(input);
    }

    // Strip URLs from text, leaving text-only content
    let remaining: String = {
        let mut out = String::with_capacity(input.len());
        let mut last = 0;
        for (start, _) in &urls {
            out.push_str(&input[last..*start]);
            last = *start;
            // skip past the URL
            let mut i = *start;
            while i < input.len() {
                let ch = input[i..].chars().next().unwrap();
                i += ch.len_utf8();
                if !ch.is_ascii_alphanumeric()
                    && !matches!(
                        ch,
                        '.' | '/'
                            | '?'
                            | '='
                            | '&'
                            | '%'
                            | '-'
                            | '_'
                            | '~'
                            | '#'
                            | '+'
                            | ','
                            | ';'
                            | ':'
                            | '@'
                            | '!'
                    )
                {
                    break;
                }
            }
        }
        out.push_str(&input[last..]);
        out.trim().to_string()
    };

    if urls.len() == 1 && remaining.is_empty() {
        // Single image, no surrounding text — use Image variant
        Message {
            role: oben_models::MessageRole::User,
            content: MessageContent::Image {
                url: urls[0].1.clone(),
                detail: None,
            },
            id: None,
            tool_call_ids: vec![],
            tool_calls: None,
            reasoning: None,
        }
    } else {
        // Multiple images or text + images — use Parts variant
        let mut parts: Vec<MessagePart> = if !remaining.is_empty() {
            vec![MessagePart::Text(remaining)]
        } else {
            Vec::new()
        };

        for (i, (_, url)) in urls.iter().enumerate() {
            // Add separator before each image if there's preceding content
            if i == 0 && !parts.is_empty() {
                parts.push(MessagePart::Text(" ".into()));
            } else if i > 0 {
                parts.push(MessagePart::Text(" ".into()));
            }
            parts.push(MessagePart::Image {
                url: url.clone(),
                detail: None,
            });
        }

        Message {
            role: oben_models::MessageRole::User,
            content: MessageContent::Parts(parts),
            id: None,
            tool_call_ids: vec![],
            tool_calls: None,
            reasoning: None,
        }
    }
}

/// Parse input text and return (text_without_images, image_urls).
fn parse_input_for_images(input: &str) -> (String, Vec<(String, Option<String>)>) {
    use regex::Regex;
    let re = Regex::new(
        r"https?://[^\s'(),]+(?:\.(?:jpg|jpeg|png|gif|webp|svg|bmp|tiff?|ico|avif))([^\s'()]*)?",
    )
    .unwrap();

    let urls: Vec<(String, Option<String>)> = re
        .captures_iter(input)
        .filter_map(|cap| cap.get(0).map(|m| (m.as_str().to_string(), None)))
        .collect();

    let text = re.replace_all(input, "").trim().to_string();
    (text, urls)
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
    {
        let mut ts = app.turn_state.lock();
        ts.on_turn_start();
    }
    tracing::info!("handle_chat_input: turn started");

    // Prepare ChatPanel for streaming
    if was_chat {
        if let Some(chat) = app.get_chat_mut() {
            tracing::info!("handle_chat_input: setting ChatPanel.streaming=true");
            chat.streaming = true;
            chat.input.streaming = true;
            chat.message_state
                .scroll_to_bottom
                .store(true, Ordering::SeqCst);
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
    let turn_state_clone = Arc::clone(&app.turn_state);
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
            let sm_arc = g.session_manager();
            let sm = sm_arc.lock().await;
            app.turn_message_count = g
                .context_window_manager()
                .session_id()
                .and_then(|sid| sm.session(&sid))
                .map(|s| s.messages.len())
                .unwrap_or(0);
        }
    }

    let handle = tokio::spawn({
        tracing::info!("handle_chat_input: tokio::spawn called");
        async move {
            info!("spawned_turn_task: calling agent");
            let (result, sid, messages) = {
                let mut guard = agent_clone.lock().await;
                // No longer create a separate delta_callback — TurnExecutor dispatches
                // all streaming deltas through config.callbacks.on_stream_delta (hook system).
                // The AgentCallbacks streaming hook is set up in App::init_agent() which
                // routes deltas to the EventBus, eliminating double-dispatch.

                // Detect image URLs in input and build the appropriate message type
                let input_msg = build_image_message(&input_clone);
                let has_images = matches!(
                    input_msg.content,
                    oben_models::MessageContent::Image { .. }
                        | oben_models::MessageContent::Parts(_)
                );

                let result = if has_images {
                    guard
                        .turn_with_message(input_msg, Some(Arc::clone(&interrupt_clone)))
                        .await
                } else {
                    guard
                        .turn(&input_clone, false, Some(Arc::clone(&interrupt_clone)))
                        .await
                };
                // Fetch session via CWM session_id (the trait no longer tracks "active").
                let sm_arc = guard.session_manager();
                let sm = sm_arc.lock().await;
                let sid = guard.context_window_manager()
                    .session_id()
                    .and_then(|sid| sm.session(&sid))
                    .map(|s| s.name.clone());
                let msgs = guard
                    .context_window_manager()
                    .session_id()
                    .and_then(|sid| sm.session(&sid))
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
                    let mut ts = turn_state_clone.lock();
                    ts.on_completed("completed");
                    let _ = done_tx_clone.send(TurnCompletion {
                        success: true,
                        session_name: sid,
                        messages,
                    });
                    tracing::info!("spawned_turn_task: sent done_tx success");
                }
                Err(e) => {
                    let mut ts = turn_state_clone.lock();
                    ts.on_error(&format!("{}", e));
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
    app.needs_redraw = false;
    let area = frame.area();
    let layout = Layouts::new(area);

    // update ChatPanel from turn_state (polling pattern via Arc<Mutex<TurnState>>)
    if let Some(chat) = app.get_chat_mut() {
        chat.update_from_turn_state();
    }

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

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_test_image(path: &std::path::Path) {
        let mut file = std::fs::File::create(path).unwrap();
        file.write_all(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A])
            .unwrap();
    }

    #[test]
    fn test_build_image_message_single_local_path() {
        let dir = std::env::temp_dir();
        let path = dir.join("oben_single_test.png");
        write_test_image(&path);

        let msg = build_image_message(&path.to_string_lossy());
        assert!(matches!(msg.content, MessageContent::Image { .. }));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_build_image_message_multiple_local_paths() {
        let dir = std::env::temp_dir();
        let path1 = dir.join("oben_multi_test_1.png");
        let path2 = dir.join("oben_multi_test_2.jpg");
        write_test_image(&path1);
        // .jpg is a valid image extension
        let mut file2 = std::fs::File::create(&path2).unwrap();
        file2.write_all(&[0xFF, 0xD8, 0xFF, 0xE0]).unwrap();
        drop(file2);

        let combined = format!(
            "{} 分析 {}",
            path1.to_string_lossy(),
            path2.to_string_lossy()
        );
        let msg = build_image_message(&combined);

        // Should produce a Parts message with both images
        if let MessageContent::Parts(parts) = msg.content {
            let image_count = parts
                .iter()
                .filter(|p| matches!(p, MessagePart::Image { .. }))
                .count();
            assert_eq!(image_count, 2, "expected 2 image parts");
            // First part should be text (the Chinese phrase)
            if let MessagePart::Text(t) = &parts[0] {
                assert!(t.contains("分析"), "first part should be text");
            }
        } else {
            panic!("expected Parts variant for multi-image");
        }

        let _ = std::fs::remove_file(&path1);
        let _ = std::fs::remove_file(&path2);
    }

    #[test]
    fn test_build_image_message_images_only() {
        let dir = std::env::temp_dir();
        let path1 = dir.join("oben_only_1.png");
        let path2 = dir.join("oben_only_2.jpg");
        write_test_image(&path1);
        let mut file2 = std::fs::File::create(&path2).unwrap();
        file2.write_all(&[0xFF, 0xD8, 0xFF, 0xE0]).unwrap();

        let combined = format!("{} {}", path1.to_string_lossy(), path2.to_string_lossy());
        let msg = build_image_message(&combined);

        if let MessageContent::Parts(parts) = msg.content {
            let image_count = parts
                .iter()
                .filter(|p| matches!(p, MessagePart::Image { .. }))
                .count();
            assert_eq!(image_count, 2, "expected 2 image parts with no text");
        } else {
            panic!("expected Parts variant for multi-image");
        }

        let _ = std::fs::remove_file(&path1);
        let _ = std::fs::remove_file(&path2);
    }

    #[test]
    fn test_build_image_message_single_with_text() {
        let dir = std::env::temp_dir();
        let path = dir.join("oben_single_text.png");
        write_test_image(&path);

        let input = format!("{} 分析下这个图片", path.to_string_lossy());
        let msg = build_image_message(&input);

        if let MessageContent::Parts(parts) = msg.content {
            assert_eq!(parts.len(), 2);
            if let MessagePart::Text(t) = &parts[0] {
                assert!(t.contains("分析下这个图片"));
            } else {
                panic!("expected first part to be text");
            }
        } else {
            panic!("expected Parts variant for image+text");
        }

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_build_image_message_no_images_fallback() {
        let msg = build_image_message("just some regular text");
        assert!(matches!(msg.content, MessageContent::Text(_)));
    }
}
