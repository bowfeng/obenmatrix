//! Message style traits for role-based rendering.
//!
//! Each message role (User, Assistant, System, Tool) has a `RoleInfo` struct
//! that controls label, icon, and theme-aware colors. Used by the bordered-block
//! renderer in conversation.rs.

use ratatui::prelude::Color;
use std::borrow::Cow;

use oben_models::MessageRole;

/// Metadata for a message role, resolved from a theme palette.
///
/// Unlike the old `MessageStyle` trait which returned hardcoded colors,
/// `RoleInfo` takes a palette and returns semantic colors from it.
#[derive(Debug, Clone)]
pub struct RoleInfo {
    /// Role label for display (e.g. "User", "Assistant").
    pub label: Cow<'static, str>,
    /// Icon/emoji rendered before the label.
    pub icon: Cow<'static, str>,
    /// Header/title bar color derived from the palette.
    pub header_color: Color,
    /// Border color for the message block.
    pub border_color: Color,
    /// Whether this is a tool result (for compact styling).
    pub is_tool: bool,
}

/// Resolve role metadata from a theme palette.
///
/// Uses the palette's semantic colors (accent, success, warning, info, etc.)
/// to produce theme-aware role colors. The same role always returns the same
/// metadata for a given palette.
pub fn role_info_for_role(role: &MessageRole, palette: &ratatui_themes::ThemePalette) -> RoleInfo {
    match role {
        MessageRole::User => RoleInfo {
            label: "You".into(),
            icon: "\u{276F}".into(), // ❯ right arrow (hermes-agent prompt symbol)
            header_color: palette.success,
            border_color: palette.success,
            is_tool: false,
        },
        MessageRole::Assistant => RoleInfo {
            label: "Assistant".into(),
            icon: "\u{250A}".into(), // ┊ vertical bar (hermes-agent tool prefix)
            header_color: palette.accent,
            border_color: palette.accent,
            is_tool: false,
        },
        MessageRole::System => RoleInfo {
            label: "System".into(),
            icon: "\u{00B7}".into(), // · centered dot
            header_color: palette.muted,
            border_color: palette.muted,
            is_tool: false,
        },
        MessageRole::Tool => RoleInfo {
            label: "Tool".into(),
            icon: "\u{26A1}".into(), // ⚡ lightning bolt (hermes-agent tool symbol)
            header_color: palette.muted,
            border_color: palette.muted,
            is_tool: true,
        },
    }
}
