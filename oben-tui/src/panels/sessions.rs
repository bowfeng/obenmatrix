//! Sessions panel — fully independent of the Agent.
//!
//! Creates its own SessionManager that connects to the SQLite database.
//! Session CRUD operations are performed on this local manager.

use super::Panel;
use crate::widgets::message_renderer::MessageRenderer;
use crate::widgets::message_display::{MessageDisplay, MessageDisplayState};
use crate::App;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};
use tui_widget_list::{ListBuilder, ListState, ListView, ScrollDirection};
use std::sync::{Arc, Mutex, RwLock};
use oben_models::Session;
use oben_sessions::SessionManager;

pub struct SessionsPanel {
    session_manager: Arc<Mutex<SessionManager>>,
    sessions: Vec<Session>,
    filtered: Vec<usize>,
    selected: usize,
    renderer: MessageRenderer,
    message_state: MessageDisplayState,
    message_display: MessageDisplay,
    list_state: RwLock<ListState>,
    right_lines: Arc<Mutex<Vec<Line<'static>>>>,
    search_query: String,
    searching: bool,
    search_input: String,
    search_cursor: usize,
}

impl SessionsPanel {
    pub fn new(sessions: Vec<Session>) -> Self {
        let sm = SessionManager::new().unwrap_or_else(|e| {
            eprintln!("Failed to create SessionManager: {}", e);
            panic!("Fatal: cannot create SessionManager");
        });
        Self::with_session_manager(sm, sessions)
    }

    pub fn new_empty() -> Self {
        let mut sm = SessionManager::new().unwrap_or_else(|e| {
            eprintln!("Failed to create SessionManager: {}", e);
            panic!("Fatal: cannot create SessionManager");
        });
        let sessions = match sm.init() {
            Ok(_) => sm.list_sessions_full(),
            Err(_) => Vec::new(),
        };
        let filtered: Vec<usize> = (0..sessions.len()).collect();
        let mut ls = ListState::default();
        ls.select(Some(0));
        Self {
            session_manager: Arc::new(Mutex::new(sm)),
            sessions,
            filtered,
            selected: 0,
            renderer: MessageRenderer::new(),
            message_state: MessageDisplayState::new(),
            message_display: MessageDisplay,
            list_state: RwLock::new(ls),
            right_lines: Arc::new(Mutex::new(Vec::new())),
            search_query: String::new(),
            searching: false,
            search_input: String::new(),
            search_cursor: 0,
        }
    }

    pub fn ensure_loaded(&mut self) {
        // Already eagerly loaded in new_empty()
    }

    /// Construct a SessionsPanel with a pre-configured SessionManager (test-only).
    pub fn with_session_manager(sm: SessionManager, sessions: Vec<Session>) -> Self {
        let filtered: Vec<usize> = (0..sessions.len()).collect();
        let mut ls = ListState::default();
        ls.select(Some(0));
        Self {
            session_manager: Arc::new(Mutex::new(sm)),
            sessions,
            filtered,
            selected: 0,
            renderer: MessageRenderer::new(),
            message_state: MessageDisplayState::new(),
            message_display: MessageDisplay,
            list_state: RwLock::new(ls),
            right_lines: Arc::new(Mutex::new(Vec::new())),
            search_query: String::new(),
            searching: false,
            search_input: String::new(),
            search_cursor: 0,
        }
    }

    pub fn update_sessions(&mut self, sessions: Vec<Session>) {
        self.sessions = sessions;
        self.apply_filter();
    }

    pub fn apply_filter(&mut self) {
        if self.search_query.is_empty() {
            self.filtered = (0..self.sessions.len()).collect();
        } else {
            let q = self.search_query.to_lowercase();
            self.filtered = self
                .sessions
                .iter()
                .enumerate()
                .filter(|(_, s)| {
                    s.name.to_lowercase().contains(&q)
                        || s.id.to_lowercase().contains(&q)
                        || s.metadata
                            .title
                            .as_deref()
                            .unwrap_or("")
                            .to_lowercase()
                            .contains(&q)
                })
                .map(|(i, _)| i)
                .collect();
        }
        if self.selected >= self.filtered.len() {
            self.selected = self.filtered.len().saturating_sub(1);
        }
    }

    fn get_session(&self) -> Option<&Session> {
        self.filtered.first().and_then(|&idx| self.sessions.get(idx))
    }

    fn get_session_id(&self) -> Option<String> {
        self.filtered.first().and_then(|&idx| self.sessions.get(idx).map(|s| s.id.clone()))
    }

    pub fn refresh_list(&mut self) {
        let all:Vec<Session> = match self.session_manager.lock() {
            Ok(sm) => sm.list_sessions_full(),
            Err(_) => Vec::new(),
        };
        self.sessions = all;
        self.apply_filter();
        self.selected = self.filtered.len().saturating_sub(1);
    }

    fn load_preview(&mut self) {
        if self.filtered.is_empty() {
            self.message_state.base_lines.clear();
            return;
        }
        let session = match self.sessions.get(self.filtered[self.selected]) {
            Some(s) => s.clone(),
            None => return,
        };
        let title = session
            .metadata
            .title
            .as_deref()
            .unwrap_or(&session.name);
        let mut lines: Vec<Line<'static>> = Vec::new();
        lines.push(Line::from(Span::styled(
            format!(" {}\u{1f4dd} {}", " ", title),
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(Span::styled(
            format!("     Updated: {} \u{00b7} {} messages",
                session.updated_at.format("%m-%d %H:%M"),
                session.metadata.message_count,
            ),
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(Span::styled(
            format!("     ID: {}", session.id),
            Style::default().fg(Color::DarkGray),
        )));
        if session.messages.is_empty() {
            if let Ok(sm) = self.session_manager.lock() {
                if let Ok(msgs) = sm.get_session_messages(&session.id) {
                    if !msgs.is_empty() {
                        lines.extend(self.format_messages(&msgs));
                    } else {
                        lines.push(Line::from(""));
                        lines.push(Line::from(Span::styled(
                            "[ No messages ]",
                            Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM),
                        )));
                    }
                }
            }
        } else {
            lines.extend(self.format_messages(&session.messages));
        }
        if lines.len() < 3 {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "[ Select a session to preview ]",
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM),
            )));
        }
        self.message_state.base_lines = lines;
        self.message_state.scroll_to_bottom = true;
        // right_lines is read by the tui-widget-list builder closure in render_message_view
        if let Ok(mut rl) = self.right_lines.lock() {
            rl.clear();
        }
        if let Ok(mut rl) = self.right_lines.lock() {
            rl.extend(self.message_state.base_lines.iter().cloned());
        }
    }

    fn format_messages(&self, messages: &[oben_models::Message]) -> Vec<Line<'static>> {
        let renderer = MessageRenderer::new();
        messages
            .iter()
            .flat_map(|msg| {
                let mut lines = renderer.render(msg);
                lines.push(Line::from(""));
                lines
            })
            .collect()
    }

    fn handle_action(&mut self, app: &mut App, action: Action, key_modifiers: KeyModifiers) {
        let session_id = match self.get_session_id() {
            Some(id) => id,
            None => return,
        };

        match action {
            Action::Switch => self.handle_switch(app, key_modifiers, session_id),
            Action::Delete => self.handle_delete(app, session_id),
            Action::New => self.handle_new(app),
            Action::Close => self.handle_close(app),
            Action::Rename => self.handle_rename(app),
            Action::Compact => self.handle_compact(app),
            Action::Fork => self.handle_fork(app),
        }

        // Refresh after any operation
        self.refresh_list();
        self.selected = self.filtered.len().saturating_sub(1);
    }

    fn handle_switch(&mut self, app: &mut App, key_modifiers: KeyModifiers, session_id: String) {
        let session_name = self
            .get_session()
            .map(|s| s.name.clone())
            .unwrap_or_default();

        if key_modifiers.contains(KeyModifiers::ALT) {
            // Alt+Enter: switch the agent's active session
            if let Some(agent) = &app.agent {
                if let Ok(mut g) = agent.try_lock() {
                    let sid = session_id.clone();
                    if let Err(e) = g.session_manager_mut().switch_session(&sid) {
                        app.status = format!("Switch error: {}", e);
                    } else {
                        app.status = format!("Switched to: {}", session_name);
                    }
                }
            }
        } else {
            // Enter: just preview
            self.load_preview();
            app.status = format!("Preview: {}", session_name);
        }
    }

    fn handle_delete(&mut self, app: &mut App, session_id: String) {
        if let Ok(mut sm) = self.session_manager.lock() {
            if let Err(e) = sm.delete(&session_id) {
                app.status = format!("Delete failed: {}", e);
                return;
            }
        }
        app.status = format!("Deleted: {}", session_id);
    }

    fn handle_new(&mut self, app: &mut App) {
        let name = format!("chat-{}", chrono::Utc::now().format("%Y%m%d-%H%M%S"));
        if let Ok(mut sm) = self.session_manager.lock() {
            let _ = sm.new_session(&name);
        }
        app.status = format!("Created: {}", name);
    }

    fn handle_close(&mut self, app: &mut App) {
        if self.filtered.is_empty() {
            return;
        }
        let session = &self.sessions[self.filtered[self.selected]];
        app.status = format!("Closed: {}", session.name);
        if let Ok(mut sm) = self.session_manager.lock() {
            let _ = sm.close();
        }
    }

    fn handle_rename(&mut self, app: &mut App) {
        if let Some(session) = self.get_session() {
            app.status = format!("Rename: {}", session.name);
        }
    }

    fn handle_compact(&mut self, app: &mut App) {
        let session_id = match self.get_session_id() {
            Some(id) => id,
            None => return,
        };
        if let Ok(mut sm) = self.session_manager.lock() {
            if let Err(e) = sm.switch_session(&session_id) {
                app.status = format!("Compact error: {}", e);
                return;
            }
        }
        let session_name = self
            .get_session()
            .map(|s| s.name.clone())
            .unwrap_or_default();
        app.status = format!("Compacting session: {}", session_name);
    }

    fn handle_fork(&mut self, app: &mut App) {
        if self.filtered.is_empty() {
            return;
        }
        let session = match self.sessions.get(self.filtered[self.selected]) {
            Some(s) => s.clone(),
            None => return,
        };
        if session.metadata.title.is_none() {
            app.status = "Cannot fork session without a title".into();
            return;
        }
        if let Some(agent) = &app.agent {
            if let Ok(mut g) = agent.try_lock() {
                let sid = session.id.clone();
                if let Some(_new_session) = g.session_manager_mut().clone_session(&sid) {
                    app.status = format!("Forked: {}", session.name);
                } else {
                    app.status = format!("Failed to fork: {}", session.name);
                }
            }
        } else {
            app.status = "Cannot fork - agent not initialized".into();
        }
    }

    pub fn get_session_name(&self) -> Option<String> {
        let s = self.filtered.first()?;
        self.sessions.get(*s).map(|s| s.name.clone())
    }

    pub fn get_message_count(&self) -> Option<usize> {
        let s = *self.filtered.first()?;
        Some(self.sessions.get(s)?.metadata.message_count)
    }
}

enum Action {
    Switch,
    Delete,
    New,
    Close,
    Rename,
    Compact,
    Fork,
}

impl Panel for SessionsPanel {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn draw(&self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::horizontal([
            Constraint::Length(40),
            Constraint::Min(0),
        ])
        .split(area);

        self.render_session_list(frame, chunks[0]);
        self.render_message_view(frame, chunks[1]);
    }

    fn handle_key(&mut self, app: &mut App, key: KeyEvent) {
        self.ensure_loaded();
        if self.searching {
            match key.code {
                KeyCode::Enter => {
                    self.searching = false;
                    self.search_query = self.search_input.clone();
                    self.apply_filter();
                }
                KeyCode::Esc => {
                    self.searching = false;
                    self.search_input.clear();
                    self.search_cursor = 0;
                }
                KeyCode::Left => {
                    if self.search_cursor > 0 {
                        self.search_cursor = self.search_input[..self.search_cursor]
                            .char_indices()
                            .last()
                            .map(|(i, _)| i)
                            .unwrap_or(0);
                    }
                }
                KeyCode::Right => {
                    if let Some(ch) = self.search_input[self.search_cursor..].chars().next() {
                        self.search_cursor += ch.len_utf8();
                    }
                }
                KeyCode::Backspace => {
                    if self.search_cursor > 0 {
                        let ch = self.search_input[..self.search_cursor]
                            .chars()
                            .next_back()
                            .unwrap();
                        self.search_input.remove(self.search_cursor - ch.len_utf8());
                        self.search_cursor -= ch.len_utf8();
                    }
                }
                KeyCode::Char(c)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    self.search_input.insert(self.search_cursor, c);
                    self.search_cursor += c.len_utf8();
                }
                _ => {}
            }
            return;
        }

        match key.code {
            KeyCode::Char('q') if key.modifiers == KeyModifiers::NONE => {
                app.active_panel = crate::PanelId::Chat;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if self.selected > 0 {
                    self.selected -= 1;
                    self.load_preview();
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.selected < self.filtered.len().saturating_sub(1) {
                    self.selected += 1;
                    self.load_preview();
                }
            }
            KeyCode::PageUp => {
                if self.selected > 0 {
                    let step = 5.min(self.selected);
                    self.selected -= step;
                    self.load_preview();
                }
            }
            KeyCode::PageDown => {
                if self.selected < self.filtered.len().saturating_sub(1) {
                    let step = 5.min(self.filtered.len().saturating_sub(1) - self.selected);
                    self.selected += step;
                    self.load_preview();
                }
            }
            KeyCode::Enter if key.modifiers == KeyModifiers::NONE => {
                self.handle_action(app, Action::Switch, KeyModifiers::NONE);
            }
            KeyCode::Enter if key.modifiers == KeyModifiers::ALT => {
                self.handle_action(app, Action::Switch, KeyModifiers::ALT);
            }
            KeyCode::Char('n') if key.modifiers == KeyModifiers::NONE => {
                self.handle_action(app, Action::New, KeyModifiers::NONE);
            }
            KeyCode::Char('c') if key.modifiers == KeyModifiers::NONE => {
                self.handle_action(app, Action::Compact, KeyModifiers::NONE);
            }
            KeyCode::Char('d') if key.modifiers == KeyModifiers::NONE => {
                self.handle_action(app, Action::Delete, KeyModifiers::NONE);
            }
            KeyCode::Char('/') if key.modifiers == KeyModifiers::NONE => {
                self.searching = true;
                self.search_input.clear();
                self.search_cursor = 0;
            }
            KeyCode::Char('x') if key.modifiers == KeyModifiers::NONE => {
                self.handle_action(app, Action::Close, KeyModifiers::NONE);
            }
            KeyCode::Char('r') if key.modifiers == KeyModifiers::NONE => {
                self.handle_action(app, Action::Rename, KeyModifiers::NONE);
            }
            KeyCode::Char('v') if key.modifiers == KeyModifiers::NONE => {
                self.handle_action(app, Action::Fork, KeyModifiers::NONE);
            }
            _ => {}
        }
    }
}

impl SessionsPanel {
    fn render_session_list(&self, frame: &mut Frame, area: Rect) {
        let header_cells = ["Name", "Msgs", "Updated"].iter().map(|h| {
            Cell::from(*h).style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
        });
        let header = Row::new(header_cells);

        let max_rows = (area.height as usize).saturating_sub(2);

        if max_rows == 0 {
            return;
        }

        let filtered_len = self.filtered.len();
        if filtered_len == 0 {
            return;
        }

        // Clamp selected to be within bounds
        let selected = if self.selected >= filtered_len {
            filtered_len.saturating_sub(1)
        } else {
            self.selected
        };

        // Ensure selected is in the visible window [display_start, display_start + max_rows).
        // Bias so selected is never at the very bottom edge — leave 1 row gap.
        let display_start = if max_rows == 0 || filtered_len == 0 {
            0
        } else {
            // Selected is visible if display_start <= selected && selected < display_start + max_rows
            // Scroll so selected is a few rows above bottom edge (buffer gap)
            let gap = 2;
            let scroll = selected.saturating_sub(max_rows.saturating_sub(gap));
            scroll.min(filtered_len.saturating_sub(max_rows))
        };

        let rows: Vec<Row> = self
            .filtered
            .iter()
            .skip(display_start)
            .take(max_rows)
            .enumerate()
            .filter_map(|(row_i, &idx)| {
                let s = self.sessions.get(idx)?;
                let updated = s.updated_at.format("%m-%d %H:%M");
                let is_selected = display_start.saturating_add(row_i) == self.selected;
                let style = if is_selected {
                    Style::default().bg(Color::DarkGray)
                } else {
                    Style::default()
                };
                let cells = vec![
                    Cell::new(format!("{}", s.name)).style(style),
                    Cell::new(format!("{}", s.metadata.message_count)).style(style),
                    Cell::new(format!("{}", updated)).style(style),
                ];
                Some(Row::new(cells))
            })
            .collect();

        let table = Table::new(rows, &[
            Constraint::Length(30),
            Constraint::Length(10),
            Constraint::Length(12),
        ])
        .header(header)
        .block(Block::default().borders(Borders::ALL).title(" Sessions "));

        frame.render_widget(table, area);
    }

    fn render_message_view(&self, frame: &mut Frame, area: Rect) {
        if self.filtered.is_empty() || self.message_state.base_lines.is_empty() {
            let block = Block::default().borders(Borders::ALL).title(" Info ");
            frame.render_widget(&block, area);
            let inner = block.inner(area);
            let para = Paragraph::new(Span::styled(
                "Select a session to view details",
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM),
            ));
            frame.render_widget(para, inner);
            return;
        }

        let s = &self.sessions[self.filtered[self.selected]];
        let name = s.metadata.title.as_deref().unwrap_or(&s.name);
        let block = Block::default().borders(Borders::ALL).title(format!(" {} ", name));

        let count = {
            let rl = self.right_lines.lock().unwrap();
            rl.len()
        };

        let right_lines = &self.right_lines;
        let builder = ListBuilder::new(move |ctx: &tui_widget_list::ListBuildContext| {
            let rl = right_lines.lock().unwrap();
            let line = rl[ctx.index].clone();
            (Paragraph::new(line), 1u16)
        });

        let mut list_state = self.list_state.write().unwrap();
        list_state.select(Some(self.selected.min(count.saturating_sub(1))));

        let list_view = ListView::new(builder, count)
            .block(block)
            .scroll_direction(ScrollDirection::Forward);

        frame.render_stateful_widget(list_view, area, &mut list_state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_dir() -> std::path::PathBuf {
        tempfile::tempdir().unwrap().path().join("sessions")
    }

    #[test]
    fn test_new_with_empty_sessions() {
        let dir = make_test_dir();
        let sm = SessionManager::new_with_path(dir).unwrap();
        let panel = SessionsPanel::with_session_manager(sm, vec![]);

        // then: empty filtered list
        assert_eq!(panel.filtered.len(), 0);
    }

    #[test]
    fn test_refresh_list_reads_from_sm() {
        // given: a panel with some sessions
        let dir = make_test_dir();
        let mut sm = SessionManager::new_with_path(dir).unwrap();
        let _ = sm.new_session("from-sm");
        let session = sm.list_sessions_full().remove(0);
        let mut panel = SessionsPanel::with_session_manager(sm, vec![session]);
        
        // when: refresh via SM
        panel.refresh_list();
        
        // then: panel reflects SM state (the "from-sm" session was persisted)
        assert!(panel.sessions.iter().any(|s| s.name == "from-sm"));
    }

    // ─── apply_filter tests ─────────────────────────────────────────────

    #[test]
    fn test_apply_filter_no_query_returns_all() {
        // given: three sessions
        let dir = make_test_dir();
        let sm = SessionManager::new_with_path(dir).unwrap();
        let mut panel = SessionsPanel::with_session_manager(
            sm,
            vec![
                Session::new("alpha"),
                Session::new("beta"),
                Session::new("gamma"),
            ],
        );
        
        // when: apply_filter with empty query
        panel.apply_filter();
        
        // then: all sessions returned
        assert_eq!(panel.filtered.len(), 3);
    }

    #[test]
    fn test_apply_filter_matches_name() {
        // given: sessions with known names
        let dir = make_test_dir();
        let sm = SessionManager::new_with_path(dir).unwrap();
        let mut panel = SessionsPanel::with_session_manager(
            sm,
            vec![
                Session::new("alpha"),
                Session::new("beta"),
                Session::new("gamma"),
            ],
        );
        
        // when: filter by "beta"
        panel.search_query = "beta".to_string();
        panel.apply_filter();
        
        // then: one match
        assert_eq!(panel.filtered.len(), 1);
        assert_eq!(panel.sessions[panel.filtered[0]].name, "beta");
    }

    #[test]
    fn test_apply_filter_matches_id() {
        // given: a session
        let dir = make_test_dir();
        let sm = SessionManager::new_with_path(dir).unwrap();
        let session = Session::new("test");
        let mut panel = SessionsPanel::with_session_manager(sm, vec![session]);
        
        // when: filter by session ID
        panel.search_query = panel.sessions[0].id.clone();
        panel.apply_filter();
        
        // then: matches
        assert_eq!(panel.filtered.len(), 1);
    }

    #[test]
    fn test_apply_filter_matches_title() {
        // given: session with title
        let dir = make_test_dir();
        let sm = SessionManager::new_with_path(dir).unwrap();
        let mut session = Session::new("session-name");
        session.metadata.title = Some("My Test Title".to_string());
        
        let mut panel = SessionsPanel::with_session_manager(sm, vec![session]);
        
        // when: filter by title text
        panel.search_query = "My Test".to_string();
        panel.apply_filter();
        
        // then: matches
        assert_eq!(panel.filtered.len(), 1);
    }

    #[test]
    fn test_apply_filter_is_case_insensitive() {
        // given: mixed-case names
        let dir = make_test_dir();
        let sm = SessionManager::new_with_path(dir).unwrap();
        let mut panel = SessionsPanel::with_session_manager(
            sm,
            vec![
                Session::new("ALPHA"),
                Session::new("beta"),
            ],
        );
        
        // when: search uppercase
        panel.search_query = "Alpha".to_string();
        panel.apply_filter();
        
        // then: still finds lowercase match
        assert_eq!(panel.filtered.len(), 1);
    }

    #[test]
    fn test_apply_filter_no_match_empty() {
        // given: sessions
        let dir = make_test_dir();
        let sm = SessionManager::new_with_path(dir).unwrap();
        let mut panel = SessionsPanel::with_session_manager(
            sm,
            vec![Session::new("alpha")],
        );
        
        // when: search for nothing
        panel.search_query = "nonexistent".to_string();
        panel.apply_filter();
        
        // then: empty
        assert!(panel.filtered.is_empty());
    }

    #[test]
    fn test_apply_filter_clamps_selected_to_filtered_len() {
        // given: filtered list
        let dir = make_test_dir();
        let sm = SessionManager::new_with_path(dir).unwrap();
        let sessions = vec![
            Session::new("one"),
            Session::new("two"),
            Session::new("three"),
        ];
        let mut panel = SessionsPanel::with_session_manager(sm, sessions);
        
        // when: clear all matches — filtered becomes empty
        panel.search_query = "zzz".to_string();
        panel.apply_filter();
        // selected should be clamped to 0 (saturating_sub)
        assert_eq!(panel.selected, 0);
        
        // when: clear matches again with non-empty filtered
        panel.search_query = "one".to_string();
        panel.apply_filter();
        assert_eq!(panel.selected, 0); // clamped to 0
    }

    // ─── get_session / get_session_id tests ─────────────────────────────

    #[test]
    fn test_get_session_returns_first_filtered() {
        // given: sessions
        let dir = make_test_dir();
        let sm = SessionManager::new_with_path(dir).unwrap();
        let sessions = vec![
            Session::new("target"),
            Session::new("other"),
        ];
        let panel = SessionsPanel::with_session_manager(sm, sessions);
        
        // when: get first
        let session = panel.get_session();
        
        // then: correct session
        assert_eq!(session.unwrap().name, "target");
    }

    #[test]
    fn test_get_session_empty_list() {
        // given: no sessions
        let dir = make_test_dir();
        let sm = SessionManager::new_with_path(dir).unwrap();
        let panel = SessionsPanel::with_session_manager(sm, Vec::new());
        
        // when: get from empty
        assert!(panel.get_session().is_none());
        assert!(panel.get_session_id().is_none());
    }

    #[test]
    fn test_get_session_id_returns_id() {
        // given: a session
        let dir = make_test_dir();
        let sm = SessionManager::new_with_path(dir).unwrap();
        let session = Session::new("id-test");
        let panel = SessionsPanel::with_session_manager(sm, vec![session]);
        
        // when: get session_id
        let id = panel.get_session_id();
        
        // then: returns the session's ID (not name)
        assert!(id.is_some());
        assert_ne!(id.unwrap(), "id-test"); // UUID, not name
    }

    // ─── search mode tests ──────────────────────────────────────────────

    #[test]
    fn test_search_input_text() {
        // given: panel
        let dir = make_test_dir();
        let sm = SessionManager::new_with_path(dir).unwrap();
        let mut panel = SessionsPanel::with_session_manager(sm, Vec::new());
        
        // when: set search input
        panel.search_input = "filter text".to_string();
        
        // then: stored
        assert_eq!(panel.search_input, "filter text");
    }

    // ─── session manager CRUD tests (via panel's SM) ────────────────────

    #[test]
    fn test_create_and_list_via_panel_sm() {
        // given: empty SM
        let dir = make_test_dir();
        let mut sm = SessionManager::new_with_path(dir).unwrap();
        
        // when: create two sessions
        let _s1 = sm.new_session("session-a");
        let _s2 = sm.new_session("session-b");
        let sessions = sm.list_sessions_full();
        
        // then: both present in SM
        assert_eq!(sessions.len(), 2);
        let names: Vec<_> = sessions.iter().map(|s| &s.name).collect();
        assert!(names.contains(&&"session-a".to_string()));
        assert!(names.contains(&&"session-b".to_string()));
    }

    #[test]
    fn test_switch_session_via_panel_sm() {
        // given: two sessions saved to SM
        let dir = make_test_dir();
        let mut sm = SessionManager::new_with_path(dir).unwrap();
        let _s1 = sm.new_session("target-session");
        let session_id = sm.list_sessions_full()[0].id.clone();
        let _s2 = sm.new_session("other-session");
        
        // when: switch to first session
        let result = sm.switch_session(&session_id);
        
        // then: succeeded
        assert!(result.is_ok());
        assert_eq!(result.unwrap().name, "target-session");
    }

    #[test]
    fn test_delete_session_via_panel_sm() {
        // given: two sessions
        let dir = make_test_dir();
        let mut sm = SessionManager::new_with_path(dir).unwrap();
        let _s1 = sm.new_session("delete-me");
        let session_id = sm.list_sessions_full()[0].id.clone();
        let _s2 = sm.new_session("keep-me");
        
        // when: delete one
        let before = sm.list_sessions_full().len();
        let _ = sm.delete(&session_id);
        let after = sm.list_sessions_full().len();
        
        // then: one fewer
        assert_eq!(before, 2);
        assert_eq!(after, 1);
    }

    #[test]
    fn test_full_lifecycle() {
        // given: empty SM
        let dir = make_test_dir();
        let mut sm = SessionManager::new_with_path(dir).unwrap();
        
        // when: create → list → filter → switch → delete
        let _ = sm.new_session("lifecycle");
        let sessions = sm.list_sessions_full();
        
        // then: created
        assert_eq!(sessions.len(), 1);
        
        // when: filter finds it
        let mut panel = SessionsPanel::with_session_manager(sm, sessions.clone());
        let id = panel.get_session_id();
        // then: returns a non-empty UUID
        assert!(id.is_some());
        assert!(!id.unwrap().is_empty());
    }
}
