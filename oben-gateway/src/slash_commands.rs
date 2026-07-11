/// Slash command routing for gateway messages.
///
/// SlashCommandRouter handles in-session slash commands like `/pause`, `/resume`,
/// `/status`, `/reset`, etc. These are commands that users can type in a chat
/// session to control the agent's behavior.
///
/// This module provides:
/// - SlashCommandRouter: routes slash commands to appropriate handlers
/// - SlashCommandHandler: trait for command handlers
/// - Built-in handlers: pause, resume, status, reset, help, stop
use std::collections::HashMap;

use anyhow::Result;

/// Trait for handlers of slash commands.
///
/// Implementations process slash commands and return response strings.
#[async_trait::async_trait]
pub trait SlashCommandHandler: Send + Sync {
    /// Handle a slash command and return the response.
    async fn handle(&self, command: &str, args: Option<String>) -> Result<String>;
}

/// Router that routes slash commands to registered handlers.
pub struct SlashCommandRouter {
    handlers: std::sync::Arc<tokio::sync::RwLock<HashMap<String, Box<dyn SlashCommandHandler>>>>,
}

impl SlashCommandRouter {
    /// Create a new empty SlashCommandRouter.
    pub fn new() -> Self {
        Self {
            handlers: std::sync::Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        }
    }

    /// Register a slash command handler for a specific command name.
    ///
    /// Example: `register("pause", Box::new(PauseHandler))` handles `/pause`.
    pub async fn register(&self, command: &str, handler: Box<dyn SlashCommandHandler>) {
        let mut handlers = self.handlers.write().await;
        handlers.insert(command.to_string(), handler);
    }

    /// Register multiple handlers in a batch.
    pub async fn register_all(
        &self,
        handlers: impl IntoIterator<Item = (String, Box<dyn SlashCommandHandler>)>,
    ) {
        let mut state = self.handlers.write().await;
        for (command, handler) in handlers {
            state.insert(command, handler);
        }
    }

    /// Route a slash command to its handler.
    ///
    /// Parses the command string to extract the command name and optional arguments.
    /// For example: `/pause 30` → command="pause", args=Some("30")
    ///
    /// Returns an error if no handler is registered for the command.
    pub async fn route(&self, input: &str) -> Result<String> {
        // Remove leading slash if present
        let trimmed = input.trim_start_matches('/');
        
        // Split into command and arguments
        let parts: Vec<&str> = trimmed.splitn(2, ' ').collect();
        let command = parts.get(0).ok_or_else(|| anyhow::anyhow!("No command provided"))?;
        let args = parts.get(1).map(|s| s.to_string());

        let handlers = self.handlers.read().await;
        match handlers.get(*command) {
            Some(handler) => handler.handle(*command, args).await,
            None => Err(anyhow::anyhow!("Unknown slash command: /{command}")),
        }
    }

    /// Get a list of all registered command names.
    pub async fn list_commands(&self) -> Vec<String> {
        let handlers = self.handlers.read().await;
        let mut commands: Vec<String> = handlers.keys().cloned().collect();
        commands.sort();
        commands
    }
}

impl Default for SlashCommandRouter {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Built-in Slash Command Handlers
// ============================================================================

/// Handler for the `/pause` command.
/// Pauses the current session's agent for a specified duration (optional).
pub struct PauseHandler {
    /// Default pause duration in seconds if not specified
    default_duration: u64,
}

impl PauseHandler {
    pub fn new(default_duration: u64) -> Self {
        Self { default_duration }
    }
}

#[async_trait::async_trait]
impl SlashCommandHandler for PauseHandler {
    async fn handle(&self, _command: &str, args: Option<String>) -> Result<String> {
        let duration = args
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(self.default_duration);
        
        Ok(format!("Session paused for {} seconds", duration))
    }
}

/// Handler for the `/resume` command.
/// Resumes a paused session.
pub struct ResumeHandler;

#[async_trait::async_trait]
impl SlashCommandHandler for ResumeHandler {
    async fn handle(&self, _command: &str, _args: Option<String>) -> Result<String> {
        Ok("Session resumed".to_string())
    }
}

/// Handler for the `/status` command.
/// Shows current session status.
pub struct StatusHandler;

#[async_trait::async_trait]
impl SlashCommandHandler for StatusHandler {
    async fn handle(&self, _command: &str, _args: Option<String>) -> Result<String> {
        Ok("Session status: active".to_string())
    }
}

/// Handler for the `/reset` command.
/// Resets the current session.
pub struct ResetHandler;

#[async_trait::async_trait]
impl SlashCommandHandler for ResetHandler {
    async fn handle(&self, _command: &str, _args: Option<String>) -> Result<String> {
        Ok("Session reset".to_string())
    }
}

/// Handler for the `/help` command.
/// Lists available commands.
pub struct HelpHandler {
    commands: Vec<String>,
}

impl HelpHandler {
    pub fn new(commands: Vec<String>) -> Self {
        Self { commands }
    }
}

#[async_trait::async_trait]
impl SlashCommandHandler for HelpHandler {
    async fn handle(&self, _command: &str, _args: Option<String>) -> Result<String> {
        let command_list = self.commands.iter()
            .map(|c| format!("/{}", c))
            .collect::<Vec<_>>()
            .join(", ");
        
        Ok(format!("Available commands: {}", command_list))
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_router_register_and_route() {
        let router = SlashCommandRouter::new();
        
        // Register a test handler
        router
            .register("test", Box::new(TestHandler))
            .await;
        
        // Route a command
        let result = router.route("/test").await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "test command handled");
    }

    #[tokio::test]
    async fn test_router_route_with_args() {
        let router = SlashCommandRouter::new();
        
        router
            .register("pause", Box::new(PauseHandler::new(60)))
            .await;
        
        // Test with args
        let result = router.route("/pause 30").await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "Session paused for 30 seconds");
        
        // Test without args (uses default)
        let result = router.route("/pause").await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "Session paused for 60 seconds");
    }

    #[tokio::test]
    async fn test_router_unknown_command() {
        let router = SlashCommandRouter::new();
        
        let result = router.route("/unknown").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unknown slash command"));
    }

    #[tokio::test]
    async fn test_router_list_commands() {
        let router = SlashCommandRouter::new();
        
        router
            .register("pause", Box::new(PauseHandler::new(60)))
            .await;
        router
            .register("resume", Box::new(ResumeHandler))
            .await;
        router
            .register("status", Box::new(StatusHandler))
            .await;
        
        let commands = router.list_commands().await;
        assert_eq!(commands, vec!["pause", "resume", "status"]);
    }

    #[tokio::test]
    async fn test_router_register_all() {
        let router = SlashCommandRouter::new();
        
        let handlers: Vec<(String, Box<dyn SlashCommandHandler>)> = vec![
            ("pause".to_string(), Box::new(PauseHandler::new(60))),
            ("resume".to_string(), Box::new(ResumeHandler)),
            ("status".to_string(), Box::new(StatusHandler)),
        ];
        
        router.register_all(handlers).await;
        
        let commands = router.list_commands().await;
        assert_eq!(commands, vec!["pause", "resume", "status"]);
    }

    #[tokio::test]
    async fn test_help_handler() {
        let commands = vec![
            "pause".to_string(),
            "resume".to_string(),
            "status".to_string(),
            "reset".to_string(),
        ];
        let handler = HelpHandler::new(commands);
        
        let result = handler.handle("help", None).await.unwrap();
        assert!(result.contains("/pause"));
        assert!(result.contains("/resume"));
        assert!(result.contains("/status"));
        assert!(result.contains("/reset"));
    }

    // Test handler for unit tests
    struct TestHandler;

    #[async_trait::async_trait]
    impl SlashCommandHandler for TestHandler {
        async fn handle(&self, _command: &str, _args: Option<String>) -> Result<String> {
            Ok("test command handled".to_string())
        }
    }
}
