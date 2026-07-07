//! Splash panel — ASCII art title screen.

use async_trait::async_trait;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};
use ratatui::layout::{Rect};
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

use super::{KeyAction, Panel};

/// The ASCII art for "OBEN MATRIX" — loaded from a file to preserve exact formatting.
const SPLASH_ART: &str = include_str!("../../assets/splash.txt");

pub struct SplashPanel {
    error: Option<String>,
}

impl SplashPanel {
    pub fn new() -> Self {
        Self { error: None }
    }

    pub fn set_error(&mut self, error: String) {
        self.error = Some(error);
    }
}

#[async_trait]
impl Panel for SplashPanel {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn draw(&self, frame: &mut Frame, area: Rect) {
        // Compute display width of each line (some chars like ██ are 2 cols wide)
        let art_lines: Vec<&str> = SPLASH_ART.lines().collect();
        let art_width: u16 = art_lines
            .iter()
            .map(|l| unicode_width::UnicodeWidthStr::width(*l) as u16)
            .max()
            .unwrap_or(0)
            .max(1);

        // Art height in lines
        let art_height = art_lines.len() as u16;

        // Center art vertically in the terminal
        let y = area.y + area.height.saturating_sub(art_height).saturating_sub(art_height) / 2;

        // Center art horizontally: compute left offset
        let left_padding = if area.width >= art_width {
            (area.width - art_width) / 2
        } else {
            0
        };

        frame.render_widget(
            Paragraph::new(SPLASH_ART).fg(Color::Yellow),
            Rect::new(area.x + left_padding, y, art_width.max(1), art_height),
        );

        // Loading tagline below the art
        let tag_y = y + art_height + 1;
        let tag_text = "    Loading agent...";
        let tag_w = unicode_width::UnicodeWidthStr::width(tag_text) as u16;
        let tag_left = if area.width >= tag_w { (area.width - tag_w) / 2 } else { 0 };
        frame.render_widget(
            Paragraph::new(tag_text)
                .style(Style::default().fg(Color::DarkGray)),
            Rect::new(area.x + tag_left, tag_y, tag_w, 1),
        );

        // Error message if present
        if let Some(ref err) = self.error {
            frame.render_widget(
                Paragraph::new(err.as_str())
                    .style(Style::default().fg(Color::Yellow).bold()),
                Rect::new(
                    area.x + area.width.saturating_sub(60) / 2,
                    y + art_height + 2,
                    60.min(area.width),
                    2,
                ),
            );
        }
    }

    async fn handle_key(&mut self, key: KeyEvent) -> KeyAction {
        match key.code {
            KeyCode::Char('q') if key.modifiers == KeyModifiers::CONTROL => KeyAction::Quit,
            _ => KeyAction::Quit,
        }
    }

    fn handle_mouse(&mut self, _area: Rect, _event: &MouseEvent) -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Given: a fresh splash panel
    /// When: no error is set
    /// Then: error field should be None
    #[test]
    fn test_initial_no_error() {
        let panel = SplashPanel::new();
        assert!(panel.error.is_none());
    }

    /// Given: a splash panel
    /// When: set_error is called
    /// Then: error message should be stored
    #[test]
    fn test_set_error() {
        let mut panel = SplashPanel::new();
        panel.set_error("test error".to_string());
        assert_eq!(panel.error.unwrap(), "test error");
    }
}
