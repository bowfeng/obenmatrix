//! Plugin slash command system.
//!
//! Maps to Hermes' slash command registration and resolution.
//! Plugins register `/cmd` handlers that are available in CLI and
//! gateway sessions. Handlers are async with a 30s timeout.

use anyhow::{anyhow, Result};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use tracing::{debug, warn};

/// A slash command registered by a plugin.
///
/// Slash commands are invoked in-session as `/command [args]`.
/// They are available in both CLI and gateway sessions.
#[derive(Clone)]
pub struct SlashCommand {
    /// Command name (without leading `/`).
    pub name: String,

    /// Async handler: takes raw args string, returns result string.
    pub handler: Arc<dyn SlashCommandHandler>,

    /// Human-readable description shown in help.
    pub description: String,

    /// Args hint shown in help (e.g., "query", "name value").
    pub args_hint: String,

    /// Plugin that registered this command.
    pub plugin: String,
}

/// Async handler trait for slash commands.
pub trait SlashCommandHandler: Send + Sync {
    /// Handle the command with raw args string.
    fn handle(&self, args: &str) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String>> + Send>>;
}

impl<F, Fut> SlashCommandHandler for F
where
    F: Fn(&str) -> Fut + Send + Sync,
    Fut: std::future::Future<Output = Result<String>> + Send + 'static,
{
    fn handle(&self, args: &str) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String>> + Send>> {
        Box::pin(self(args))
    }
}

/// Registry for plugin slash commands.
///
/// Thread-safe map of command name → SlashCommand.
/// Resolves conflicts against built-in commands.
pub struct SlashCommandRegistry {
    /// Registered commands: name → command.
    commands: RwLock<HashMap<String, SlashCommand>>,

    /// Built-in command names that cannot be overridden.
    builtins: RwLock<HashSet<String>>,
}

impl Default for SlashCommandRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl SlashCommandRegistry {
    /// Create a new empty slash command registry.
    pub fn new() -> Self {
        Self {
            commands: RwLock::new(HashMap::new()),
            builtins: RwLock::new(HashSet::new()),
        }
    }

    /// Register a built-in command name (cannot be overridden by plugins).
    pub fn reserve_builtin(&self, name: &str) {
        self.builtins.write().unwrap().insert(name.to_string());
    }

    /// Register a slash command from a plugin.
    ///
    /// Returns error if the command conflicts with a built-in.
    pub fn register(&self, cmd: SlashCommand) -> Result<()> {
        let name = cmd.name.trim_start_matches('/').to_lowercase();
        let builtins = self.builtins.read().unwrap();
        if builtins.contains(&name) {
            return Err(anyhow!(
                "Cannot register slash command '{}': conflicts with built-in command",
                name
            ));
        }

        let plugin = cmd.plugin.clone();
        let debug_name = name.clone();
        let mut cmd = cmd;
        cmd.name = name.clone();
        self.commands.write().unwrap().insert(name, cmd);
        debug!(
            "Registered slash command: /{} (plugin={})",
            debug_name, plugin
        );
        Ok(())
    }

    /// Resolve a slash command invocation.
    ///
    /// Parses `/command args` format and returns the command + args.
    /// Returns None if no matching command is found.
    ///
    /// Supports both `/command` and `command` formats (without leading `/`).
    pub fn resolve(&self, raw: &str) -> Option<(SlashCommand, String)> {
        let parts: Vec<&str> = raw.splitn(2, ' ').collect();
        let (name, args) = match parts.len() {
            1 => (parts[0], ""),
            _ => (parts[0], parts[1]),
        };

        let name = name.trim_start_matches('/').to_lowercase();
        let commands = self.commands.read().unwrap();
        commands.get(&name).map(|cmd| (cmd.clone(), args.to_string()))
    }

    /// Execute a resolved slash command with a 30s timeout.
    ///
    /// Returns the command output or an error if the handler panicked/timed out.
    pub async fn execute(&self, raw: &str) -> Result<String> {
        let Some((cmd, args)) = self.resolve(raw) else {
            return Err(anyhow!("Unknown slash command: {}", raw.trim()));
        };

        // Use tokio timeout for async execution
        let timeout = tokio::time::Duration::from_secs(30);
        let result = tokio::time::timeout(timeout, cmd.handler.handle(&args)).await;

        match result {
            Ok(Ok(output)) => Ok(output),
            Ok(Err(e)) => Err(anyhow::anyhow!("Command '{}' error: {}", cmd.name, e)),
            Err(_) => Err(anyhow::anyhow!(
                "Command '{}' timed out after {}s",
                cmd.name,
                timeout.as_secs()
            )),
        }
    }

    /// List all registered slash commands (owned clones).
    #[allow(dead_code)]
    pub fn list(&self) -> Vec<SlashCommand> {
        self.commands.read().unwrap().values().cloned().collect()
    }

    /// List all registered slash commands (owned clones).
    pub fn list_owned(&self) -> Vec<SlashCommand> {
        self.commands.read().unwrap().values().cloned().collect()
    }

    /// List command names for help display.
    pub fn list_names(&self) -> Vec<String> {
        self.commands.read().unwrap().keys().cloned().collect()
    }

    /// Check if a command name is registered.
    pub fn contains(&self, name: &str) -> bool {
        let name = name.trim_start_matches('/').to_lowercase();
        self.commands.read().unwrap().contains_key(&name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::ready;

    fn make_handler(output: String) -> Arc<dyn SlashCommandHandler> {
        Arc::new(move |_args: &str| {
            Box::pin(ready(Ok(output.clone())))
        })
    }

    fn make_cmd(name: &str, plugin: &str, output: String) -> SlashCommand {
        SlashCommand {
            name: name.to_string(),
            handler: make_handler(output),
            description: "Test command".to_string(),
            args_hint: "args".to_string(),
            plugin: plugin.to_string(),
        }
    }

    #[test]
    fn test_registry_new() {
        /// given: a new SlashCommandRegistry
        /// when: created
        /// then: no commands registered
        let registry = SlashCommandRegistry::new();
        assert!(registry.list().is_empty());
        assert_eq!(registry.list_names(), Vec::<String>::new());
    }

    #[test]
    fn test_register_command() {
        /// given: an empty registry
        /// when: register() is called
        /// then: command is available in list() and contains()
        let registry = SlashCommandRegistry::new();
        let cmd = make_cmd("test", "test-plugin", "hello".to_string());
        registry.register(cmd.clone()).unwrap();

        assert!(registry.contains("test"));
        assert!(registry.contains("/test"));
        assert_eq!(registry.list().len(), 1);
        assert_eq!(registry.list()[0].name, "test");
        assert_eq!(registry.list()[0].plugin, "test-plugin");
    }

    #[test]
    fn test_resolve_command() {
        /// given: a registry with a command registered
        /// when: resolve() is called with "/test args"
        /// then: returns the command and args
        let registry = SlashCommandRegistry::new();
        registry.register(make_cmd("test", "plugin", "result".into())).unwrap();

        let result = registry.resolve("/test hello world");
        assert!(result.is_some());
        let (cmd, args) = result.unwrap();
        assert_eq!(cmd.name, "test");
        assert_eq!(args, "hello world");

        // Without leading /
        let result2 = registry.resolve("test hello world");
        assert!(result2.is_some());
        let (_cmd2, args2) = result2.unwrap();
        assert_eq!(args2, "hello world");
    }

    #[test]
    fn test_resolve_no_match() {
        /// given: a registry with commands
        /// when: resolve() is called for unknown command
        /// then: returns None
        let registry = SlashCommandRegistry::new();
        registry.register(make_cmd("test", "plugin", "result".into())).unwrap();

        assert!(registry.resolve("/unknown").is_none());
        assert!(registry.resolve("/nonexistent args").is_none());
    }

    #[test]
    fn test_builtins_prevent_override() {
        /// given: a registry with a builtin reserved
        /// when: plugin tries to register with same name
        /// then: returns Err
        let registry = SlashCommandRegistry::new();
        registry.reserve_builtin("help");

        let result = registry.register(make_cmd("help", "plugin", "output".into()));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("conflicts with built-in"));
    }

    #[test]
    fn test_case_insensitive() {
        /// given: a registry with a command
        /// when: resolve() is called with different case
        /// then: command is found
        let registry = SlashCommandRegistry::new();
        registry.register(make_cmd("Test", "plugin", "result".into())).unwrap();

        assert!(registry.resolve("/test").is_some());
        assert!(registry.resolve("/TEST").is_some());
        assert!(registry.resolve("/tEsT").is_some());
    }

    #[test]
    fn test_execute_command() {
        /// given: a registry with a command
        /// when: execute() is called
        /// then: handler is invoked and result returned
        let rt = tokio::runtime::Runtime::new().unwrap();
        let registry = SlashCommandRegistry::new();
        registry.register(make_cmd("ping", "plugin", "pong".into())).unwrap();

        rt.block_on(async {
            let result = registry.execute("/ping").await;
            assert_eq!(result.unwrap(), "pong");
        });
    }

    #[test]
    fn test_execute_unknown_command() {
        /// given: an empty registry
        /// when: execute() is called for unknown command
        /// then: returns Err
        let rt = tokio::runtime::Runtime::new().unwrap();
        let registry = SlashCommandRegistry::new();

        rt.block_on(async {
            let result = registry.execute("/unknown").await;
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("Unknown slash command"));
        });
    }

    #[test]
    fn test_execute_with_args() {
        /// given: a registry with a command that uses args
        /// when: execute() is called with args
        /// then: args are passed to handler
        let registry = SlashCommandRegistry::new();
        registry.register(make_cmd("greet", "plugin", "Hello, World!".into())).unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let result = registry.execute("/greet").await;
            assert_eq!(result.unwrap(), "Hello, World!");
        });
    }

    #[test]
    fn test_multiple_commands() {
        /// given: a registry with multiple commands
        /// when: list() is called
        /// then: all commands are returned
        let registry = SlashCommandRegistry::new();
        registry.register(make_cmd("cmd1", "p1", "a".into())).unwrap();
        registry.register(make_cmd("cmd2", "p2", "b".into())).unwrap();

        let list = registry.list();
        assert_eq!(list.len(), 2);
    }
}
