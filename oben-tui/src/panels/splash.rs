//! Splash panel — renders a matrix rain effect during agent initialization.

use async_trait::async_trait;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};
use ratatui::layout::{Alignment, Rect};
use ratatui::prelude::*;
use ratatui::widgets::{Paragraph, Widget};
use std::time::Duration;
use tui_rain::{CharacterSet, Rain};

use super::{KeyAction, Panel};
use crate::App;

/// Rain drop fall speed (px/s), default is 5.0 * 2.0 = 10.0 px/s.
const RAIN_DROP_SPEED_PPS: f64 = 10.0;

/// Result of agent initialization, used to drive splash → Chat/Setup transitions.
#[derive(Debug)]
pub enum InitResult {
    /// Agent init succeeded.
    Ok,
    /// Agent init failed with a human-readable error message.
    Err(String),
}

pub struct SplashPanel {
    start_time: std::time::Instant,
    pub min_display_duration: Duration,
    pub error: Option<String>,
}

impl SplashPanel {
    pub fn new() -> Self {
        Self {
            start_time: std::time::Instant::now(),
            min_display_duration: Duration::from_secs(5),
            error: None,
        }
    }

    /// Set minimum display duration based on panel height.
    ///
    /// Ensures at least one full rain drop falls from top to bottom.
    pub fn set_min_duration(&mut self, height: u16) {
        let fall_secs = (height as f64 / RAIN_DROP_SPEED_PPS).max(1.0);
        self.min_display_duration = Duration::from_secs(fall_secs as u64);
    }

    pub fn set_error(&mut self, error: String) {
        self.error = Some(error);
    }

    /// Returns the remaining time to show splash to meet the minimum duration.
    pub fn remaining_min_display(&self) -> Duration {
        let elapsed = self.start_time.elapsed();
        if elapsed < self.min_display_duration {
            self.min_display_duration - elapsed
        } else {
            Duration::ZERO
        }
    }

    /// Returns remaining ms to display splash.
    pub fn remaining_ms(&self) -> u64 {
        let elapsed = self.start_time.elapsed();
        if elapsed < self.min_display_duration {
            (self.min_display_duration.as_millis() - elapsed.as_millis()) as u64
        } else {
            0
        }
    }
}

/// Japanese katakana character set for matrix rain drops.
const KANA_SET: CharacterSet = CharacterSet::UnicodeRange {
    start: 0x30A0,
    len: 0x96,
};

/// Matrix rain widget — renders matrix rain drops in a rect area.
struct MatrixRain {
    elapsed: Duration,
}

impl Widget for MatrixRain {
    fn render(self, area: Rect, buf: &mut Buffer) {
        Rain::new_matrix(self.elapsed)
            .with_character_set(KANA_SET)
            .render(area, buf);
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
        let rain = MatrixRain {
            elapsed: self.start_time.elapsed(),
        };

        // Rain fills the full area as background
        frame.render_widget(rain, area);

        // Show error message centered at bottom of screen if present
        if let Some(ref err) = self.error {
            let err_text = format!(
                "  ⚠  Agent initialization failed.\n  {}\n\n  Press Ctrl+C to quit.",
                err
            );
            let lines = err_text.lines().count() as u16;
            let para = Paragraph::new(err_text)
                .alignment(Alignment::Center)
                .style(Style::default().fg(Color::LightGreen));
            let error_area = Rect::new(
                area.x + area.width.saturating_sub(60) / 2,
                area.y + area.height.saturating_sub(lines).saturating_div(2),
                60.min(area.width),
                lines.max(1),
            );
            frame.render_widget(para, error_area);
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

    async fn on_activate(&mut self, _app: &mut App) {}

    async fn on_deactivate(&mut self, _app: &mut App) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Given: A splash panel with default min display duration
    /// When: set_min_duration is called with height 24
    /// Then: min_display_duration is set to 24/10=2s
    #[test]
    fn test_set_min_duration_24_lines() {
        let mut panel = SplashPanel::new();
        assert!(panel.remaining_ms() > 0);

        panel.set_min_duration(24);
        assert_eq!(panel.min_display_duration.as_secs(), 2); // 24/10=2.4→2
    }

    /// Then: 60/10=6s
    #[test]
    fn test_set_min_duration_60_lines() {
        let mut panel = SplashPanel::new();
        panel.set_min_duration(60);
        assert_eq!(panel.min_display_duration.as_secs(), 6); // 60/10=6.0
    }

    /// Given: A splash panel with min duration set and time has passed
    /// When: remaining_min_display returns ZERO
    /// Then: The splash is no longer shown
    #[test]
    fn test_remaining_zero_when_elapsed() {
        let mut panel = SplashPanel::new();
        // Use a very short duration and wait for it to pass
        panel.min_display_duration = Duration::from_millis(50);
        panel.start_time = std::time::Instant::now() - Duration::from_millis(100);
        assert_eq!(panel.remaining_min_display(), Duration::ZERO);
    }

    /// Given: A splash panel with min duration set
    /// When: remaining_min_display is called before elapsed
    /// Then: Returns remaining time > 0
    #[test]
    fn test_remaining_nonzero_before_elapsed() {
        let mut panel = SplashPanel::new();
        panel.min_display_duration = Duration::from_secs(5);
        // Immediately after creation, remaining should be close to 5s
        let remaining = panel.remaining_min_display();
        let elapsed = panel.start_time.elapsed();
        assert!(remaining + elapsed >= Duration::from_secs(5));
        assert!(remaining < Duration::from_secs(5));
    }

    /// Given: A splash panel after creating
    /// When: remaining_ms is called
    /// Then: Returns reasonable ms value
    #[test]
    fn test_remaining_ms_returns_value() {
        let panel = SplashPanel::new();
        let ms = panel.remaining_ms();
        assert!(ms > 0);
        assert!(ms <= 5000); // max default is 5s
    }
}
