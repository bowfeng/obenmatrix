//! Sessions panel — fully independent of the Agent.
//!
//! Creates its own SessionManager that connects to the SQLite database.
//! Session CRUD operations are performed on this local manager.

use super::{KeyAction, Panel};
use crate::widgets::message_renderer::MessageRenderer;
use crate::widgets::conversation::ConversationState;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};
use tui_widget_list::{ListBuilder, ListState, ListView, ScrollDirection};
use std::sync::{Arc, Mutex, RwLock};
use oben_agent::Agent;
use oben_models::Session;

pub struct SessionsPanel {
    agent: Arc<tokio::sync::Mutex<Agent>>,
    sessions: Vec<Session>,
    filtered: Vec<usize>,
    selected: usize,
    message_state: ConversationState,
    list_state: RwLock<ListState>,
    right_lines: Arc<Mutex<Vec<Line<'static>>>>,
    sessions_loaded: RwLock<bool>,
    search_query: String,
    searching: bool,
    search_input: String,
    search_cursor: usize,
    /// Name of the currently active session (set by refresh_list).
    active_session_name: Option<String>,
}

impl SessionsPanel {
    /// Create a SessionsPanel with a shared Agent (for TUI ↔ Agent sharing).
    pub fn new_shared(agent: Arc<tokio::sync::Mutex<Agent>>) -> Self {
        let mut ls = ListState::default();
        ls.select(Some(0));
        Self {
            agent: agent,
            sessions: Vec::new(),
            filtered: Vec::new(),
            selected: 0,
            sessions_loaded: RwLock::new(false),
            message_state: ConversationState::new(),
            list_state: RwLock::new(ls),
            right_lines: Arc::new(Mutex::new(Vec::new())),
            search_query: String::new(),
            searching: false,
            search_input: String::new(),
            search_cursor: 0,
            active_session_name: None,
        }
    }

    /// Empty fallback (no agent). Use new_shared() in production.
    pub fn new_empty() -> Self {
        tracing::warn!("Warning: SessionsPanel::new_empty() should not be used in production");
        panic!("Fatal: SessionsPanel requires an Agent reference");
    }

    /// Test-only: construct a SessionsPanel with pre-loaded sessions.
    pub fn with_empty_sessions(_sessions: Vec<Session>) -> Self {
        panic!("with_empty_sessions should not be used in production");
    }

    /// Construct a SessionsPanel with a pre-configured SessionManager (test-only).
    #[allow(unused)]
    pub fn with_session_manager(_sm: oben_sessions::SessionManager, _sessions: Vec<Session>) -> Self {
        panic!("with_session_manager should not be used in production");
    }

    /// Load sessions from the SessionManager (called on panel activation).
    /// Always re-fetches from agent so session changes made in the Chat panel
    /// (rename, clear, new) are immediately reflected.
    pub async fn ensure_loaded(&mut self) {
        // Must init session manager first -- it loads from SQLite into the in-memory HashMap
        if let Err(e) = self.agent.lock().await.init_session_manager().await {
            tracing::error!("[SessionsPanel] Failed to init session manager: {}", e);
            return;
        }
        let sessions = self.agent.lock().await.list_sessions_full().await;
        self.sessions = sessions;
        self.filtered = (0..self.sessions.len()).collect();
        self.active_session_name = self.agent.lock().await.active_session_name().await;
    }

    pub fn update_sessions(&mut self, sessions: Vec<Session>) {
        self.sessions = sessions;
        self.apply_filter();
    }

    /// Update the display title of a single session without re-fetching the full list.
    /// Match by old_title and replace with new_title.
    pub fn refresh_display_name(&mut self, old_title: &str, new_title: &str) {
        for session in self.sessions.iter_mut() {
            let current = session
                .metadata
                .title
                .as_deref()
                .unwrap_or(&session.name);
            if current == old_title {
                session.metadata.title = Some(new_title.to_string());
                self.apply_filter();
                return;
            }
        }
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
        self.filtered.get(self.selected).and_then(|&idx| self.sessions.get(idx))
    }

    fn get_session_id(&self) -> Option<String> {
        self.filtered.get(self.selected).and_then(|&idx| self.sessions.get(idx).map(|s| s.id.clone()))
    }

    pub async fn refresh_list(&mut self) {
        let sessions = self.agent.lock().await.list_sessions_full().await;
        self.sessions = sessions;
        self.filtered = (0..self.sessions.len()).collect();
        self.active_session_name = self.agent.lock().await.active_session_name().await;
    }

    async fn load_preview(&mut self) {
        if self.filtered.is_empty() {
            self.message_state.message_entries.lock().unwrap().clear();
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
            let msgs = self.agent.lock().await
                .get_session_messages(&session.id).await
                .unwrap_or_default();
            if !msgs.is_empty() {
                lines.extend(self.format_messages(&msgs));
            } else {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    "[ No messages ]",
                    Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM),
                )));
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
        // Populate entries for bordered-block rendering
        let renderer = MessageRenderer::new();
        let mut entries = Vec::new();
        // Build a system message entry for metadata
        for line in lines.iter() {
            let body_lines = vec![crate::widgets::message_renderer::StyledLine {
                content: line.clone(),
                role_color: None,
            }];
            entries.push(crate::widgets::message_renderer::MessageRenderEntry {
                role: oben_models::MessageRole::System,
                body_lines,
                is_tool_result: false,
                tool_calls: Vec::new(),
            });
        }
        // Also add actual message entries
        for msg in &session.messages {
            entries.push(renderer.render_entry(msg));
        }
        self.message_state.message_entries.lock().unwrap().clear();
        self.message_state.message_entries.lock().unwrap().extend(entries);
        self.message_state.scroll_to_bottom = true;
        // right_lines is read by the tui-widget-list builder closure in render_message_view
        if let Ok(mut rl) = self.right_lines.lock() {
            rl.clear();
            rl.extend(lines);
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

    async fn handle_action(&mut self, action: Action) {
        let session_id = match self.get_session_id() {
            Some(id) => id,
            None => return,
        };

        match action {
            Action::Switch(_) => self.handle_switch(session_id).await,
            Action::Delete => {
                // Cannot delete the active session
                if let Some(session) = self.get_session() {
                    let current_name = session.metadata.title.as_deref()
                        .unwrap_or(&session.name);
                    if self.active_session_name.as_deref() == Some(current_name)
                        || self.active_session_name.as_deref() == Some(&session.name)
                    {
                        return;
                    }
                }
                self.handle_delete(session_id).await;
                self.refresh_list().await;
            }
            Action::New => {
                self.handle_new().await;
                self.refresh_list().await;
            }
            Action::Close => {
                self.handle_close().await;
                self.refresh_list().await;
            }
            Action::Rename => self.handle_rename(),
            Action::Compact => self.handle_compact(session_id).await,
            Action::Fork => {
                self.handle_fork();
                self.refresh_list().await;
            }
        }
        self.selected = self.filtered.len().saturating_sub(1);
    }

    async fn handle_switch(&mut self, _session_id: String) {
        // Just preview — switch in agent session is handled via /session command
        self.load_preview().await;
    }

    /// Load/switch to the selected session in the agent.
    /// If there's no active session, the selected becomes active.
    /// If there's an active session, switch to the selected one.
    pub async fn load_session(&mut self) {
        let session_id = match self.get_session_id() {
            Some(id) => id,
            None => return,
        };
        if let Err(e) = self.agent.lock().await.switch_session_to(&session_id).await {
            tracing::error!(
                "[SessionsPanel] Failed to switch to session {}: {}",
                session_id,
                e
            );
        }
    }

    async fn handle_delete(&mut self, session_id: String) {
        let _ = self.agent.lock().await.delete_session(&session_id).await;
    }

    async fn handle_new(&mut self) {
        let _ = self.agent.lock().await.new_session().await;
    }

    async fn handle_close(&mut self) {
        if self.filtered.is_empty() {
            return;
        }
        let _ = self.agent.lock().await.close_session().await;
    }

    fn handle_rename(&mut self) {
        // Rename display only — actual rename handled via /rename command
    }

    async fn handle_compact(&mut self, session_id: String) {
        let _ = self.agent.lock().await.switch_session(&session_id).await;
    }

    fn handle_fork(&mut self) {
        if self.filtered.is_empty() {
            return;
        }
        let session = match self.sessions.get(self.filtered[self.selected]) {
            Some(s) => s.clone(),
            None => return,
        };
        if session.metadata.title.is_none() {
            return;
        }
        // Fork is handled via /session command with app context
    }

    pub fn get_session_name(&self) -> Option<String> {
        let s = *self.filtered.first()?;
        self.sessions.get(s).map(|s| {
            s.metadata.title.as_deref().unwrap_or(&s.name).to_string()
        })
    }

    pub fn get_message_count(&self) -> Option<usize> {
        let s = *self.filtered.first()?;
        Some(self.sessions.get(s)?.metadata.message_count)
    }
}

enum Action {
    Switch(KeyModifiers),
    Delete,
    New,
    Close,
    Rename,
    Compact,
    Fork,
}

#[async_trait::async_trait]
impl Panel for SessionsPanel {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    async fn on_activate(&mut self, _app: &mut crate::App) {
        self.ensure_loaded().await;
    }

    async fn on_deactivate(&mut self, _app: &mut crate::App) {
        // Reset sessions_loaded and clear the cached sessions so ensure_loaded()
        // will re-fetch fresh data from the agent on next activation.
        *self.sessions_loaded.write().unwrap() = false;
        self.sessions.clear();
        self.active_session_name = None;
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

    async fn handle_key(&mut self, key: KeyEvent) -> KeyAction {
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
            return KeyAction::None;
        }

        match key.code {
            KeyCode::Char('q') if key.modifiers == KeyModifiers::NONE => {
                return KeyAction::SwitchPanel(super::PanelId::Chat);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if self.selected > 0 {
                    self.selected -= 1;
                    self.load_preview().await;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.selected < self.filtered.len().saturating_sub(1) {
                    self.selected += 1;
                    self.load_preview().await;
                }
            }
            KeyCode::PageUp => {
                if self.selected > 0 {
                    let step = 5.min(self.selected);
                    self.selected -= step;
                    self.load_preview().await;
                }
            }
            KeyCode::PageDown => {
                if self.selected < self.filtered.len().saturating_sub(1) {
                    let step = 5.min(self.filtered.len().saturating_sub(1) - self.selected);
                    self.selected += step;
                    self.load_preview().await;
                }
            }
            KeyCode::Enter if key.modifiers == KeyModifiers::NONE => {
                self.handle_action(Action::Switch(KeyModifiers::NONE)).await;
            }
            KeyCode::Enter if key.modifiers == KeyModifiers::ALT => {
                self.handle_action(Action::Switch(KeyModifiers::ALT)).await;
            }
            KeyCode::Char('n') if key.modifiers == KeyModifiers::NONE => {
                self.handle_action(Action::New).await;
            }
            KeyCode::Char('c') if key.modifiers == KeyModifiers::NONE => {
                self.handle_action(Action::Compact).await;
            }
            KeyCode::Char('d') if key.modifiers == KeyModifiers::NONE => {
                self.handle_action(Action::Delete).await;
            }
            KeyCode::Char('/') if key.modifiers == KeyModifiers::NONE => {
                self.searching = true;
                self.search_input.clear();
                self.search_cursor = 0;
            }
            KeyCode::Char('x') if key.modifiers == KeyModifiers::NONE => {
                self.handle_action(Action::Close).await;
            }
            KeyCode::Char('r') if key.modifiers == KeyModifiers::NONE => {
                self.handle_action(Action::Rename).await;
            }
            KeyCode::Char('v') if key.modifiers == KeyModifiers::NONE => {
                self.handle_action(Action::Fork).await;
            }
            KeyCode::Char('l') if key.modifiers == KeyModifiers::CONTROL => {
                self.load_session().await;
                self.refresh_list().await;
                return KeyAction::SessionChanged;
            }
            _ => {}
        }
        KeyAction::None
    }
}

impl SessionsPanel {
    fn render_session_list(&self, frame: &mut Frame, area: Rect) {
        let header_cells = ["Name"].iter().map(|h| {
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
                let is_selected = display_start.saturating_add(row_i) == self.selected;
                let style = if is_selected {
                    Style::default().bg(Color::DarkGray)
                } else {
                    Style::default()
                };
                let display_title = s.metadata.title.as_deref().unwrap_or(&s.name);
                let is_active = self.active_session_name.as_deref() == Some(&display_title)
                    || self.active_session_name.as_deref() == Some(&s.name);
                let display_name = if is_active {
                    format!("* {}", display_title)
                } else {
                    display_title.to_string()
                };
                let cells = vec![
                    Cell::new(display_name).style(style),
                ];
                Some(Row::new(cells))
            })
            .collect();

        let table = Table::new(rows, &[
            Constraint::Length(40),
        ])
        .header(header)
        .block(Block::default().borders(Borders::ALL).title(" Sessions "));

        frame.render_widget(table, area);
    }

    fn render_message_view(&self, frame: &mut Frame, area: Rect) {
        if self.filtered.is_empty() || self.message_state.message_entries.lock().unwrap().is_empty() {
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
    use oben_sessions::SessionManager;

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
        let panel = SessionsPanel::with_session_manager(sm, sessions.clone());
        let id = panel.get_session_id();
        // then: returns a non-empty UUID
        assert!(id.is_some());
        assert!(!id.unwrap().is_empty());
    }
}
