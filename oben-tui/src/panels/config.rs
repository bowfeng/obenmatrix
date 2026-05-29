//! Config panel — inline YAML editor for AppConfig.

use super::Panel;
use crate::App;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarState, ScrollbarOrientation};
use ratatui::layout::Rect;

pub struct ConfigPanel {
    pub text: String,
    pub lines: Vec<String>,
    pub cursor_line: usize,
    pub cursor_col: usize,
    pub scroll: usize,
}

impl ConfigPanel {
    pub fn new(yaml: String) -> Self {
        let lines: Vec<String> = yaml.split('\n').map(|s| s.to_string()).collect();
        Self {
            text: yaml,
            lines,
            cursor_line: 0,
            cursor_col: 0,
            scroll: 0,
        }
    }
}

impl Panel for ConfigPanel {
    fn as_any(&self) -> &dyn std::any::Any { self }

    fn draw(&self, frame: &mut Frame, area: Rect) {
        let text_lines: Vec<Line> = self
            .lines
            .iter()
            .enumerate()
            .skip(self.scroll)
            .take(area.height as usize)
            .map(|(i, line)| {
                let style = if i == self.cursor_line {
                    Style::default().bg(Color::DarkGray)
                } else {
                    Style::default()
                };
                Line::from(line.clone()).style(style)
            })
            .collect();

        let para = Paragraph::new(Text::from(text_lines))
            .block(Block::default().borders(Borders::ALL).title(" Config "));
        frame.render_widget(para, area);

        if self.lines.len() > area.height as usize {
            let mut state = ScrollbarState::new(self.lines.len()).position(self.scroll);
            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight);
            frame.render_stateful_widget(scrollbar, area, &mut state);
        }

        let legend = " Ctrl+S:save  j/k:nav  0:home  $:end  q:done  ";
        let span = Span::styled(legend, Style::default().fg(Color::Gray));
        let para = Paragraph::new(Line::from(span));
        let legend_area = Rect::new(area.x, area.y + area.height - 1, area.width, 1);
        frame.render_widget(para, legend_area);
    }

    fn handle_key(&mut self, app: &mut App, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') if key.modifiers == KeyModifiers::NONE => {
                app.active_panel = crate::PanelId::Chat;
            }
            KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                match serde_yaml::from_str::<oben_config::AppConfig>(&self.text) {
                    Ok(config) => {
                        if let Err(e) = config.save() {
                            app.status = format!("Save failed: {}", e);
                        } else {
                            app.status = "Config saved to ~/.obenalien/config.yaml".to_string();
                            app.config = config;
                        }
                    }
                    Err(e) => {
                        app.status = format!("Invalid YAML: {}", e);
                    }
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.cursor_line < self.lines.len().saturating_sub(1) {
                    self.cursor_line += 1;
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if self.cursor_line > 0 {
                    self.cursor_line -= 1;
                    if self.cursor_line < self.scroll {
                        self.scroll = self.cursor_line;
                    }
                }
            }
            KeyCode::PageUp => {
                let step = 5.min(self.cursor_line);
                self.cursor_line -= step;
                self.scroll = self.scroll.saturating_sub(step);
            }
            KeyCode::PageDown => {
                let step = 5.min(self.lines.len().saturating_sub(1 - self.cursor_line));
                self.cursor_line += step;
                self.scroll += step;
            }
            _ => {}
        }
    }
}
