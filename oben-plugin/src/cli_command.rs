//! Plugin CLI command registry.
//!
//! Maps to Hermes' CLI command registration where plugins can add
//! `hermes subcmd` CLI subcommands.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tracing::debug;

/// A CLI subcommand registered by a plugin.
///
/// CLI commands are invoked as `obenagent <subcommand>` in the terminal.
/// They have setup and handler functions.
#[derive(Clone)]
pub struct CliCommand {
    /// Command name (e.g., "debug", "status").
    pub name: String,

    /// Human-readable description/help text.
    pub description: String,

    /// Plugin that registered this command.
    pub plugin: String,

    /// Setup function called before the handler.
    /// Used to parse args, load config, etc.
    pub setup_fn: Option<Arc<dyn CliCommandSetup + Send + Sync>>,

    /// Handler function called after setup.
    /// Used to execute the command logic.
    pub handler_fn: Option<Arc<dyn CliCommandHandler + Send + Sync>>,
}

/// Trait for CLI command setup functions.
pub trait CliCommandSetup: Send + Sync {
    /// Setup the command: parse args, validate config, etc.
    /// Returns (remaining_args, exit_on_error).
    fn setup(&self, args: &[String]) -> (Vec<String>, bool);
}

/// Trait for CLI command handler functions.
pub trait CliCommandHandler: Send + Sync {
    /// Handle the command after setup.
    fn handle(&self) -> Result<String, String>;
}

impl<F> CliCommandHandler for F
where
    F: Fn() -> Result<String, String> + Send + Sync,
{
    fn handle(&self) -> Result<String, String> {
        self()
    }
}

/// Registry for plugin CLI commands.
///
/// Plugins register CLI commands here; the CLI crate can then
/// wire them up to clap subcommands.
pub struct CliCommandRegistry {
    /// Registered commands: name → CliCommand.
    commands: RwLock<HashMap<String, CliCommand>>,
}

impl Default for CliCommandRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl CliCommandRegistry {
    /// Create a new empty CLI command registry.
    pub fn new() -> Self {
        Self {
            commands: RwLock::new(HashMap::new()),
        }
    }

    /// Register a CLI command from a plugin.
    pub fn register(&self, name: &str, description: &str, plugin: &str) {
        let cmd = CliCommand {
            name: name.to_string(),
            description: description.to_string(),
            plugin: plugin.to_string(),
            setup_fn: None,
            handler_fn: None,
        };
        self.commands.write().unwrap().insert(name.to_string(), cmd);
        debug!("Registered CLI command: {} (plugin={})", name, plugin);
    }

    /// Register a CLI command with setup and handler functions.
    pub fn register_with_fns(
        &self,
        name: &str,
        description: &str,
        plugin: &str,
        setup_fn: Option<Arc<dyn CliCommandSetup + Send + Sync>>,
        handler_fn: Option<Arc<dyn CliCommandHandler + Send + Sync>>,
    ) {
        let mut commands = self.commands.write().unwrap();
        if let Some(cmd) = commands.get_mut(name) {
            cmd.setup_fn = setup_fn;
            cmd.handler_fn = handler_fn;
        } else {
            drop(commands);
            self.register(name, description, plugin);
            // Re-register with functions
            let cmd = CliCommand {
                name: name.to_string(),
                description: description.to_string(),
                plugin: plugin.to_string(),
                setup_fn,
                handler_fn,
            };
            self.commands.write().unwrap().insert(name.to_string(), cmd);
        }
        debug!(
            "Registered CLI command with functions: {} (plugin={})",
            name, plugin
        );
    }

    /// List all registered CLI commands (owned clones).
    pub fn list_owned(&self) -> Vec<CliCommand> {
        self.commands.read().unwrap().values().cloned().collect()
    }

    /// Get a registered CLI command by name (owned clone).
    pub fn get_owned(&self, name: &str) -> Option<CliCommand> {
        self.commands.read().unwrap().get(name).cloned()
    }

    /// Check if a CLI command is registered.
    pub fn contains(&self, name: &str) -> bool {
        self.commands.read().unwrap().contains_key(name)
    }

    /// List command names for help display.
    pub fn list_names(&self) -> Vec<String> {
        self.commands.read().unwrap().keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_new() {
        /// given: a new CliCommandRegistry
        /// when: created
        /// then: no commands registered
        let registry = CliCommandRegistry::new();
        assert!(registry.list_owned().is_empty());
        assert_eq!(registry.list_names(), Vec::<String>::new());
    }

    #[test]
    fn test_register_cli_command() {
        /// given: a new registry
        /// when: register() is called
        /// then: command is in the registry
        let registry = CliCommandRegistry::new();
        registry.register("test", "Test command", "test-plugin");

        assert!(registry.contains("test"));
        let cmds = registry.list_owned();
        assert_eq!(cmds.len(), 1);
        let cmd = &cmds[0];
        assert_eq!(cmd.description, "Test command");
        assert_eq!(cmd.plugin, "test-plugin");
    }

    #[test]
    fn test_register_with_functions() {
        /// given: a registry with a command
        /// when: register_with_fns() is called
        /// then: command has setup and handler functions
        let registry = CliCommandRegistry::new();
        registry.register("test", "Test", "plugin");

        // Create boxed functions that satisfy the trait bounds
        struct SetupFn;
        impl CliCommandSetup for SetupFn {
            fn setup(&self, _args: &[String]) -> (Vec<String>, bool) {
                (vec![], false)
            }
        }
        struct HandlerFn;
        impl CliCommandHandler for HandlerFn {
            fn handle(&self) -> Result<String, String> {
                Ok("output".to_string())
            }
        }

        registry.register_with_fns(
            "test",
            "Test updated",
            "plugin",
            Some(Arc::new(SetupFn)),
            Some(Arc::new(HandlerFn)),
        );

        let cmd = registry.get_owned("test").unwrap();
        assert!(cmd.setup_fn.is_some());
        assert!(cmd.handler_fn.is_some());
    }

    #[test]
    fn test_list_commands() {
        /// given: a registry with multiple commands
        /// when: list_owned() is called
        /// then: all commands are returned
        let registry = CliCommandRegistry::new();
        registry.register("cmd1", "Description 1", "plugin1");
        registry.register("cmd2", "Description 2", "plugin2");

        let list = registry.list_owned();
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn test_get_nonexistent() {
        /// given: a registry with commands
        /// when: get_owned() is called for unknown name
        /// then: returns None
        let registry = CliCommandRegistry::new();
        registry.register("test", "Test", "plugin");

        assert!(registry.get_owned("unknown").is_none());
    }
}
