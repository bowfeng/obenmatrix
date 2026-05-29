//! Panel types for the TUI.

pub mod chat;
pub mod config;
pub mod setup;
pub mod sessions;

use crossterm::event::KeyEvent;
use ratatui::Frame;
use ratatui::layout::Rect;

use super::App;

/// Unique identifier for each panel type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PanelId {
    Chat,
    Sessions,
    Config,
    Setup,
}

/// Trait that every panel must implement.
pub trait Panel: Send + Sync {
    /// Cast to `dyn Any` for downcasting in TUI runtime.
    fn as_any(&self) -> &dyn std::any::Any;

    /// Draw the panel content in the given area.
    fn draw(&self, frame: &mut Frame, area: Rect);

    /// Handle a keyboard event. Returns true if the event was consumed.
    fn handle_key(&mut self, app: &mut App, key: KeyEvent);
}

impl dyn Panel {
    /// Downcast &dyn Panel to &T if this is a ChatPanel.
    pub fn downcast_ref<T: std::any::Any>(&self) -> Option<&T> {
        self.as_any().downcast_ref::<T>()
    }
}
