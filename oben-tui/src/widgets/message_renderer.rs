//! Message renderer — renders `oben_models::Message` into `Line` slices.
//!
//! Holds a runtime `ThemePalette` lookup so colors change on `set_theme()` without rebuilding
//! message content. The renderer does not draw borders or titles — those belong to the
//! widget layer.

use ratatui::prelude::*;
use ratatui_themes::ThemeName;
use std::sync::Mutex;

use crate::widgets::role_style::{get_style_for_role, ColorHint};
use oben_models::{Message, MessageContent};

/// Internal color lookup mapped from `ColorHint` to `ThemePalette` fields.
#[derive(Debug, Clone, Copy)]
enum ColorField {
    Success,
    Info,
    Accent,
    Warning,
}

impl ColorField {
    fn to_color(self, palette: &ratatui_themes::ThemePalette) -> Color {
        match self {
            ColorField::Success => palette.success,
            ColorField::Info => palette.info,
            ColorField::Accent => palette.accent,
            ColorField::Warning => palette.warning,
        }
    }
}

impl ColorHint {
    fn to_field(self) -> ColorField {
        match self {
            ColorHint::Success => ColorField::Success,
            ColorHint::Info => ColorField::Info,
            ColorHint::Accent => ColorField::Accent,
            ColorHint::Warning => ColorField::Warning,
        }
    }
}

/// Message renderer with runtime theme support.
pub struct MessageRenderer {
    current_palette: Mutex<ratatui_themes::ThemePalette>,
    current_theme: Mutex<ThemeName>,
}

impl MessageRenderer {
    pub fn new() -> Self {
        let default_theme = ThemeName::default();
        Self {
            current_palette: Mutex::new(default_theme.palette()),
            current_theme: Mutex::new(default_theme),
        }
    }

    pub fn set_theme(&self, theme: ThemeName) {
        *self.current_theme.lock().unwrap() = theme;
        *self.current_palette.lock().unwrap() = theme.palette();
    }

    pub fn current_theme(&self) -> ThemeName {
        *self.current_theme.lock().unwrap()
    }

    pub fn render(&self, msg: &Message) -> Vec<Line<'static>> {
        let palette = self.current_palette.lock().unwrap();
        let style = get_style_for_role(&msg.role);
        let color_field = style.color_hint().to_field();

        let mut lines = Vec::new();
        let header = format!("── {} {} ──", style.icon(), style.label());
        lines.push(Line::from(Span::styled(
            header,
            Style::default()
                .fg(color_field.to_color(&palette))
                .add_modifier(Modifier::BOLD),
        )));

        let text = match &msg.content {
            MessageContent::Text(t) => t.clone(),
            // Image / Parts variants are silent (per spec).
            _ => String::new(),
        };

        for line in text.lines() {
            lines.push(Line::from(line.to_string()));
        }

        // Tool call indicators
        if let Some(tool_calls) = &msg.tool_calls {
            for tc in tool_calls {
                let tool_info = if tc.arguments.is_null() {
                    tc.tool_name.clone()
                } else {
                    format!(
                        "{}({})",
                        tc.tool_name,
                        tc.arguments
                            .to_string()
                            .chars()
                            .take(30)
                            .collect::<String>()
                    )
                };
                lines.push(Line::from(format!("   {}", tool_info)));
            }
        }

        lines.push(Line::from(""));
        lines
    }
}

impl Default for MessageRenderer {
    fn default() -> Self {
        Self::new()
    }
}
