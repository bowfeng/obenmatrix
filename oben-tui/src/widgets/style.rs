//! Shared style definitions for the TUI.

use ratatui::prelude::Color;
use ratatui::style::Style;

/// Default text color.
pub const FOREGROUND: Color = Color::White;
/// Default background color.
pub const BACKGROUND: Color = Color::Black;
/// Accent color.
pub const ACCENT: Color = Color::Cyan;
/// Danger/error color.
pub const DANGER: Color = Color::Red;
/// Success color.
pub const SUCCESS: Color = Color::Green;
/// Warning color.
pub const WARNING: Color = Color::Yellow;
/// Muted/decorative color.
pub const MUTED: Color = Color::Gray;

/// Theme for the TUI.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub foreground: Color,
    pub background: Color,
    pub accent: Color,
    pub danger: Color,
    pub success: Color,
    pub warning: Color,
    pub muted: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            foreground: FOREGROUND,
            background: BACKGROUND,
            accent: ACCENT,
            danger: DANGER,
            success: SUCCESS,
            warning: WARNING,
            muted: MUTED,
        }
    }
}

impl Theme {
    /// Apply this theme's style to a style builder.
    pub fn apply(self) -> Style {
        Style::default()
            .fg(self.foreground)
            .bg(self.background)
    }
}
