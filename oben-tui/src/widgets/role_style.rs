//! Message style traits for role-based rendering.
//!
//! Each message role (User, Assistant, System, Tool) has a `MessageStyle` adapter
//! that controls label, icon, and its semantic color hint. The renderer maps hints
//! to `ThemePalette` fields at render time so runtime theme switching works.

use std::borrow::Cow;

use oben_models::MessageRole;

/// Semantic color hint. Maps to a field on `ThemePalette` at render time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorHint {
    Success,
    Info,
    Accent,
    Warning,
}

/// Trait implemented by each message role adapter.
pub trait MessageStyle: Send + Sync + 'static {
    /// Role label for the header separator (e.g. "User", "Assistant").
    fn label(&self) -> Cow<'static, str>;

    /// Icon prefix rendered before the label.
    fn icon(&self) -> Cow<'static, str>;

    /// Semantic color hint. The renderer maps this to a palette field from the
    /// current theme so that runtime theme switching changes colors without
    /// rebuilding message content.
    fn color_hint(&self) -> ColorHint;
}

/// Adapter for the User role.
pub struct UserRoleStyle;
impl MessageStyle for UserRoleStyle {
    fn label(&self) -> Cow<'static, str> {
        "User".into()
    }
    fn icon(&self) -> Cow<'static, str> {
        "\u{1F464}".into()
    }
    fn color_hint(&self) -> ColorHint {
        ColorHint::Success
    }
}

/// Adapter for the Assistant role.
pub struct AssistantRoleStyle;
impl MessageStyle for AssistantRoleStyle {
    fn label(&self) -> Cow<'static, str> {
        "Assistant".into()
    }
    fn icon(&self) -> Cow<'static, str> {
        "\u{1F4AC}".into()
    }
    fn color_hint(&self) -> ColorHint {
        ColorHint::Info
    }
}

/// Adapter for the System role.
pub struct SystemRoleStyle;
impl MessageStyle for SystemRoleStyle {
    fn label(&self) -> Cow<'static, str> {
        "System".into()
    }
    fn icon(&self) -> Cow<'static, str> {
        "\u{2699}".into()
    }
    fn color_hint(&self) -> ColorHint {
        ColorHint::Accent
    }
}

/// Adapter for the Tool role.
pub struct ToolRoleStyle;
impl MessageStyle for ToolRoleStyle {
    fn label(&self) -> Cow<'static, str> {
        "Tool".into()
    }
    fn icon(&self) -> Cow<'static, str> {
        "\u{1F527}".into()
    }
    fn color_hint(&self) -> ColorHint {
        ColorHint::Warning
    }
}

/// Lookup the `MessageStyle` for a given `MessageRole`.
pub fn get_style_for_role(role: &MessageRole) -> &'static dyn MessageStyle {
    match role {
        MessageRole::User => &UserRoleStyle,
        MessageRole::Assistant => &AssistantRoleStyle,
        MessageRole::System => &SystemRoleStyle,
        MessageRole::Tool => &ToolRoleStyle,
    }
}
