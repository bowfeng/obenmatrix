//! Panel types for the TUI.

pub mod chat;
pub mod config;
pub mod sessions;
pub mod setup;

use async_trait::async_trait;
use crossterm::event::{KeyEvent, MouseEvent};
use ratatui::layout::Rect;
use ratatui::Frame;

/// Unique identifier for each panel type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PanelId {
    Chat,
    Sessions,
    Config,
    Setup,
}

/// Actions returned by `Panel::handle_key` for `App::handle_key` to process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyAction {
    /// No action taken.
    None,
    /// Execute /clear command.
    Clear,
    /// Execute /new command.
    New,
    /// Execute /compact command.
    Compact,
    /// Exit TUI.
    Quit,
    /// Toggle reasoning mode.
    Reasoning,
    /// Cycle theme.
    Theme,
    /// Execute slash command with optional arguments.
    Command { cmd_name: String, extra: String },
    /// Switch to a different panel.
    SwitchPanel(PanelId),
    /// Send chat input text.
    ChatInput(String),
    /// Session switched by SessionsPanel — refresh ChatPanel.
    SessionChanged,
}

/// Trait that every panel must implement.
#[async_trait]
pub trait Panel: Send + Sync {
    /// Cast to `dyn Any` for downcasting in TUI runtime.
    fn as_any(&self) -> &dyn std::any::Any;

    /// Cast to `dyn Any` for mut downcasting in TUI runtime.
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;

    /// Draw the panel content in the given area.
    fn draw(&self, frame: &mut Frame, area: Rect);

    /// Handle a keyboard event. Returns a `KeyAction` for the app to process.
    /// The panel does NOT execute commands itself — `App::handle_key`
    /// processes the returned action after the panel has consumed the key.
    async fn handle_key(&mut self, key: KeyEvent) -> KeyAction;

    /// Handle a mouse event in the given area.
    /// Returns Some(text) when selection was made and text should be auto-copied.
    fn handle_mouse(&mut self, _area: Rect, _event: &MouseEvent) -> Option<String> {
        None
    }

    /// Called when this panel becomes active. Default no-op.
    async fn on_activate(&mut self, _app: &mut crate::App) {}

    /// Called when this panel becomes inactive. Default no-op.
    async fn on_deactivate(&mut self, _app: &mut crate::App) {}
}

impl dyn Panel {
    /// Downcast &dyn Panel to &T if this is a ChatPanel.
    pub fn downcast_ref<T: std::any::Any>(&self) -> Option<&T> {
        self.as_any().downcast_ref::<T>()
    }

    /// Downcast &mut dyn Panel to &mut T if this is a ChatPanel.
    pub fn downcast_mut<T: std::any::Any>(&mut self) -> Option<&mut T> {
        self.as_any_mut().downcast_mut::<T>()
    }
}
