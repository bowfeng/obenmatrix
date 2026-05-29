//! Sessions panel — list, search, select, compact, delete sessions.

use super::Panel;
use crate::App;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Table, Row, Cell};
use ratatui::layout::Rect;

use oben_models::Session;

pub struct SessionsPanel {
    pub sessions: Vec<Session>,
    pub filtered: Vec<usize>,
    pub selected: usize,
    pub search_query: String,
    pub search_input: String,
    pub search_cursor: usize,
    pub searching: bool,
    pub scroll_offset: usize,
}

impl SessionsPanel {
    pub fn new(sessions: Vec<Session>) -> Self {
        let filtered: Vec<usize> = (0..sessions.len()).collect();
        Self {
            sessions,
            filtered,
            selected: 0,
            search_query: String::new(),
            search_input: String::new(),
            search_cursor: 0,
            searching: false,
            scroll_offset: 0,
        }
    }

    fn apply_filter(&mut self) {
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
                        || s.metadata.title.as_deref().unwrap_or("").to_lowercase().contains(&q)
                })
                .map(|(i, _)| i)
                .collect();
        }
        if self.selected >= self.filtered.len() {
            self.selected = self.filtered.len().saturating_sub(1);
        }
    }

    fn compact_selected(&self, app: &mut App) {
        if self.filtered.is_empty() { return; }
        let session = &self.sessions[self.filtered[self.selected]];
        app.chat.as_mut().unwrap().session_manager_mut().load(Some(&session.id)).ok();
        app.status = format!("Compacting session: {}", session.name);
    }

    fn delete_selected(&mut self, app: &mut App) {
        if self.filtered.is_empty() { return; }
        let session_id = self.sessions[self.filtered[self.selected]].id.clone();
        let chat = app.chat.as_mut().unwrap();
        if let Err(e) = chat.session_manager_mut().delete(&session_id) {
            app.status = format!("Delete failed: {}", e);
            return;
        }
        self.sessions.retain(|s| s.id != session_id);
        self.apply_filter();
        chat.session_manager_mut().load(None).ok();
        self.selected = self.filtered.len().saturating_sub(1);
        app.status = format!("Deleted session: {}", session_id);
    }

    fn new_session(&mut self, app: &mut App) {
        let name = format!("chat-{}", chrono::Utc::now().format("%Y%m%d-%H%M%S"));
        let session = app.chat.as_mut().unwrap().session_manager_mut().new_session(&name);
        self.sessions.push(session.clone());
        self.apply_filter();
        self.selected = self.filtered.len() - 1;
        app.status = format!("Created session: {}", session.name);
    }
}

impl Panel for SessionsPanel {
    fn as_any(&self) -> &dyn std::any::Any { self }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }

    fn draw(&self, frame: &mut Frame, area: Rect) {
        let header_cells = ["Name", "Msgs", "Updated"]
            .iter()
            .map(|h| Cell::from(*h).style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)));
        let header = Row::new(header_cells);

        let rows: Vec<Row> = self
            .filtered
            .iter()
            .skip(self.scroll_offset)
            .take((area.height as usize).saturating_sub(2))
            .filter_map(|&idx| {
                let s = self.sessions.get(idx)?;
                let updated = s.updated_at.format("%m-%d %H:%M");
                let cells = vec![
                    Cell::new(format!("{}", s.name)),
                    Cell::new(format!("{}", s.messages.len())),
                    Cell::new(format!("{}", updated)),
                ];
                Some(Row::new(cells))
            })
            .collect();

        let table = Table::new(
            rows,
            &[
                Constraint::Length(30),
                Constraint::Length(10),
                Constraint::Length(12),
            ],
        )
        .header(header)
        .block(Block::default().borders(Borders::ALL).title(" Sessions "));

        frame.render_widget(table, area);

        if self.searching {
            let search_text = format!("🔍 {}", &self.search_input[self.search_cursor..]);
            let para = Paragraph::new(Text::from(search_text.as_str()))
                .block(Block::default().borders(Borders::ALL).title(" Search "));
            frame.render_widget(para, area);
            let cursor_x = 3 + self.search_cursor.min(area.width as usize - 3) as u16;
            frame.set_cursor_position(Position::new(cursor_x, area.y));
        }

        let legend = " Enter:switch  n:new  c:compact  d:delete  /:search  Esc:done  q:chat ";
        let span = Span::styled(legend, Style::default().fg(Color::Gray));
        let para = Paragraph::new(Line::from(span));
        let legend_area = Rect::new(area.x, area.y + area.height - 1, area.width, 1);
        frame.render_widget(para, legend_area);
    }

    fn handle_key(&mut self, app: &mut App, key: KeyEvent) {
        if self.searching {
            match key.code {
                KeyCode::Enter => { self.searching = false; self.apply_filter(); }
                KeyCode::Esc => { self.searching = false; self.search_input.clear(); self.search_cursor = 0; }
                KeyCode::Left => { if self.search_cursor > 0 { self.search_cursor -= 1; } }
                KeyCode::Right => { self.search_cursor += 1; }
                KeyCode::Backspace => {
                    if self.search_cursor > 0 {
                        self.search_input.remove(self.search_cursor - 1);
                        self.search_cursor -= 1;
                    }
                }
                KeyCode::Char(c) if key.modifiers == KeyModifiers::NONE => {
                    self.search_input.insert(self.search_cursor, c);
                    self.search_cursor += 1;
                }
                _ => {}
            }
            return;
        }

        match key.code {
            KeyCode::Char('q') if key.modifiers == KeyModifiers::NONE => {
                app.active_panel = crate::PanelId::Chat;
            }
            KeyCode::Char('n') if key.modifiers == KeyModifiers::NONE => {
                self.new_session(app);
            }
            KeyCode::Char('c') if key.modifiers == KeyModifiers::NONE => {
                self.compact_selected(app);
            }
            KeyCode::Char('d') if key.modifiers == KeyModifiers::NONE => {
                self.delete_selected(app);
            }
            KeyCode::Char('/') if key.modifiers == KeyModifiers::NONE => {
                self.searching = true;
                self.search_input.clear();
                self.search_cursor = 0;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if self.selected > 0 { self.selected -= 1; }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.selected < self.filtered.len().saturating_sub(1) {
                    self.selected += 1;
                }
            }
            KeyCode::PageUp => {
                let step = 5.min(self.scroll_offset);
                self.scroll_offset -= step;
            }
            KeyCode::PageDown => {
                self.scroll_offset += 5;
            }
            KeyCode::Enter if key.modifiers == KeyModifiers::NONE => {
                if self.selected < self.filtered.len() {
                    let idx = self.filtered[self.selected];
                    let session = &self.sessions[idx];
                    let chat = app.chat.as_mut().unwrap();
                    if let Err(e) = chat.session_manager_mut().load(Some(&session.id)) {
                        app.status = format!("Load error: {}", e);
                    } else {
                        app.status = format!("Switched to: {}", session.name);
                    }
                }
            }
            _ => {}
        }
    }
}
