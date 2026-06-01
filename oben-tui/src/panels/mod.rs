//! Panel types for the TUI.

pub mod chat;
pub mod config;
pub mod sessions;
pub mod setup;

use async_trait::async_trait;
use crossterm::event::{KeyEvent, MouseEvent};
use ratatui::layout::Rect;
use ratatui::Frame;

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
#[async_trait]
pub trait Panel: Send + Sync {
    /// Cast to `dyn Any` for downcasting in TUI runtime.
    fn as_any(&self) -> &dyn std::any::Any;

    /// Cast to `dyn Any` for mut downcasting in TUI runtime.
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;

    /// Draw the panel content in the given area.
    fn draw(&self, frame: &mut Frame, area: Rect);

    /// Handle a keyboard event. Returns true if the event was consumed.
    fn handle_key(&mut self, app: &mut App, key: KeyEvent);

    /// Handle a mouse event. Default no-op (returns false).
    fn handle_mouse(&mut self, _event: &MouseEvent) -> bool {
        false
    }

    /// Called when this panel becomes active. Default no-op.
    async fn on_activate(&mut self, _app: &mut App) {}

    /// Called when this panel becomes inactive. Default no-op.
    async fn on_deactivate(&mut self, _app: &mut App) {}
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
