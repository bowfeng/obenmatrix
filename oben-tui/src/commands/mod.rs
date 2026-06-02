//! TUI slash commands — each command is a struct implementing `TuiCommand`.
//!
//! Dispatch is handled in `InputBarWidget::handle_submit()`. The command is
//! parsed there and routed to the matching command's `execute()` method.

use crate::TuiEvent;
use crate::App;

/// Result of executing a TUI command.
pub enum CommandResult {
    /// Command succeeded.
    Ok,
    /// Command failed with an error message (shown as a toast).
    Error(String),
    /// User requested quit.
    Quit,
}

/// Trait that every TUI slash command must implement.
pub trait TuiCommand: Send + Sync {
    /// The command name without the leading slash, e.g. "clear".
    fn name(&self) -> &str;

    /// Human-readable description for tab completion.
    fn description(&self) -> &str;

    /// Execute the command. `app` is the current App state.
    fn execute(&self, app: &mut App);
}

// ─── Concrete commands ──────────────────────────────────────────────────────

/// Clear all messages from the current chat display and session.
pub struct ClearCommand;

impl TuiCommand for ClearCommand {
    fn name(&self) -> &str {
        "clear"
    }
    fn description(&self) -> &str {
        "Clear chat messages"
    }
    fn execute(&self, app: &mut App) {
        if let Some(agent) = &app.agent {
            let result = {
                let mut guard = agent.blocking_lock();
                guard.reset()
            };
            if let Err(e) = result {
                app.show_toast(format!("Clear failed: {e}"), ratatui_toaster::ToastType::Error);
                return;
            }
        }
        if let Some(chat) = app.get_chat_mut() {
            chat.clear_display();
        }
        app.show_toast("Cleared chat messages.", ratatui_toaster::ToastType::Success);
    }
}

/// Compact (summarize) the current session context.
/// Sends a TuiEvent::CompactSession to the main loop for async execution.
pub struct CompactCommand;

impl TuiCommand for CompactCommand {
    fn name(&self) -> &str {
        "compact"
    }
    fn description(&self) -> &str {
        "Compress current session context"
    }
    fn execute(&self, app: &mut App) {
        if app.agent.is_none() {
            app.show_toast("Cannot compact: agent not initialized", ratatui_toaster::ToastType::Error);
            return;
        }
        if app.turn_handle.is_some() {
            app.show_toast("Cannot compact: turn in progress", ratatui_toaster::ToastType::Warning);
            return;
        }
        // Signal the main loop to start compaction
        if let Some(chat) = app.get_chat_mut() {
            chat.streaming = true;
        }
        app.show_toast("Compacting session context...", ratatui_toaster::ToastType::Info);
        if let Some(tx) = &app.input_tx {
            let _ = tx.send(TuiEvent::CompactSession);
        }
    }
}

/// Start a new session (reset current session messages).
pub struct NewCommand;

impl TuiCommand for NewCommand {
    fn name(&self) -> &str {
        "new"
    }
    fn description(&self) -> &str {
        "Start a new session"
    }
    fn execute(&self, app: &mut App) {
        if let Some(agent) = &app.agent {
            let result = {
                let mut guard = agent.blocking_lock();
                guard.reset()
            };
            if let Err(e) = result {
                app.show_toast(format!("New session failed: {e}"), ratatui_toaster::ToastType::Error);
                return;
            }
        }
        if let Some(chat) = app.get_chat_mut() {
            chat.clear_display();
        }
        app.show_toast("New session started.", ratatui_toaster::ToastType::Success);
    }
}

/// Exit the TUI.
pub struct QuitCommand;

impl TuiCommand for QuitCommand {
    fn name(&self) -> &str {
        "quit"
    }
    fn description(&self) -> &str {
        "Exit TUI"
    }
    fn execute(&self, app: &mut App) {
        app.running = false;
    }
}

/// Toggle step-by-step reasoning mode.
pub struct ReasoningCommand;

impl TuiCommand for ReasoningCommand {
    fn name(&self) -> &str {
        "reasoning"
    }
    fn description(&self) -> &str {
        "Toggle reasoning mode"
    }
    fn execute(&self, app: &mut App) {
        app.reasoning_enabled = !app.reasoning_enabled;
        let msg = if app.reasoning_enabled {
            "Reasoning mode: ON"
        } else {
            "Reasoning mode: OFF"
        };
        app.show_toast(msg, ratatui_toaster::ToastType::Info);
    }
}

/// Switch to another existing session by name.
/// Usage: /session [session_name]
/// Rename the current session.
/// Usage: /rename [new_name]
///
/// Args are parsed by `handle_submit` before `execute()` is called.
pub struct RenameCommand;

impl TuiCommand for RenameCommand {
    fn name(&self) -> &str {
        "rename"
    }
    fn description(&self) -> &str {
        "Rename current session (e.g. /rename my-chat)"
    }
    fn execute(&self, app: &mut App) {
        app.show_toast(
            "Usage: /rename [new_name]",
            ratatui_toaster::ToastType::Error,
        );
    }
}

/// Execute a session rename with the given new name.
/// Called by `handle_submit` with the parsed args[1].
pub fn execute_session_rename(app: &mut App, new_name: &str) {
    if app.agent.is_none() {
        app.show_toast("Rename failed: agent not initialized", ratatui_toaster::ToastType::Error);
        return;
    }

    let success = {
        let mut guard = app.agent.as_ref().unwrap().blocking_lock();
        if let Some(session) = guard.session_manager_mut().active_session_mut() {
            session.name = new_name.to_string();
            // Save the session while guard is still held
            if let Err(e) = guard.session_manager_mut().save(None) {
                drop(guard);
                app.show_toast(
                    format!("Rename saved with error: {e}"),
                    ratatui_toaster::ToastType::Warning,
                );
                return;
            }
            true
        } else {
            false
        }
    };

    if success {
        app.show_toast(
            format!("Renamed session to: {new_name}"),
            ratatui_toaster::ToastType::Success,
        );
    } else {
        app.show_toast(
            "Rename failed: no active session",
            ratatui_toaster::ToastType::Error,
        );
    }
}

/// Show available commands.
/// Show help.
pub struct HelpCommand;

impl TuiCommand for HelpCommand {
    fn name(&self) -> &str {
        "help"
    }
    fn description(&self) -> &str {
        "Show this help message"
    }
    fn execute(&self, app: &mut App) {
        app.show_toast(
            "Slash commands: /clear /compact /new /quit /reasoning /rename [name]\n\
             Keyboard: Up/Down=history, Ctrl+A/E=home/end, Ctrl+W=delete word, Ctrl+K=kill line",
            ratatui_toaster::ToastType::Info,
        );
    }
}

/// Show theme info.
pub struct ThemeCommand;

impl TuiCommand for ThemeCommand {
    fn name(&self) -> &str {
        "theme"
    }
    fn description(&self) -> &str {
        "Current theme info"
    }
    fn execute(&self, app: &mut App) {
        app.show_toast(
            "Press Ctrl+T to cycle themes",
            ratatui_toaster::ToastType::Info,
        );
    }
}

/// Show pending tasks.
pub struct TodoCommand;

impl TuiCommand for TodoCommand {
    fn name(&self) -> &str {
        "todo"
    }
    fn description(&self) -> &str {
        "Show pending tasks"
    }
    fn execute(&self, app: &mut App) {
        app.show_toast(
            "TODO: No pending tasks.",
            ratatui_toaster::ToastType::Info,
        );
    }
}

// ─── Registry ──────────────────────────────────────────────────────────────

/// Registry of all built-in TUI commands.
pub struct TuiCommandRegistry {
    commands: Vec<Box<dyn TuiCommand>>,
}

impl TuiCommandRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            commands: Vec::new(),
        };
        registry.register(Box::new(ClearCommand));
        registry.register(Box::new(CompactCommand));
        registry.register(Box::new(NewCommand));
        registry.register(Box::new(QuitCommand));
        registry.register(Box::new(ReasoningCommand));
        registry.register(Box::new(RenameCommand));
        registry.register(Box::new(ThemeCommand));
        registry.register(Box::new(HelpCommand));
        registry.register(Box::new(TodoCommand));
        registry
    }

    fn register(&mut self, cmd: Box<dyn TuiCommand>) {
        self.commands.push(cmd);
    }

    /// Resolve a command by name (without leading slash).
    pub fn resolve(&self, name: &str) -> Option<&dyn TuiCommand> {
        self.commands
            .iter()
            .find(|c| c.name() == name)
            .map(|c| c.as_ref() as &dyn TuiCommand)
    }

    /// Find a command by name and execute it.
    /// Uses direct iteration to avoid borrow conflicts between resolve() and execute().
    pub fn execute_command(&self, name: &str, app: &mut crate::App) {
        if let Some(cmd) = self.commands.iter().find(|c| c.name() == name) {
            cmd.execute(app);
        }
    }

    /// Check if a command name exists (for completion).
    pub fn has_command(&self, name: &str) -> bool {
        self.commands.iter().any(|c| c.name() == name)
    }

    /// Get all command names for tab completion.
    pub fn all_names(&self) -> Vec<&str> {
        self.commands.iter().map(|c| c.name()).collect()
    }

    /// Get all command descriptions for tab completion.
    pub fn all_descriptions(&self) -> Vec<&str> {
        self.commands.iter().map(|c| c.description()).collect()
    }
}

impl Default for TuiCommandRegistry {
    fn default() -> Self {
        Self::new()
    }
}
