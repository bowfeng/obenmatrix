/// PluginContext provides a developer-friendly API for WASM plugins
/// to register tools, commands, and use host services.
///
/// This is the Hermes-style `ctx` facade — plugin developers call
/// register_tool(), register_command(), inject_message(), and llm_complete()
/// without worrying about WASM internals.
///
/// Under the hood, registrations are stored in internal maps.
/// The host (plugin loader) later collects these and pushes to ToolRegistry / CLI.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

// WasmRuntime available for Phase 2 host service binding

/// Capabilities a plugin declares to the host.
#[derive(Debug, Clone, Default)]
pub struct PluginCapabilities {
    /// Plugin can read files in the workspace (gates read_file host service)
    pub workspace_read: bool,
    /// Plugin can make HTTP requests (gates http_request host service)
    pub http: bool,
    /// Plugin can invoke other tools through the host (gates tool_invoke host service)
    pub tool_invoke: bool,
}

impl PluginCapabilities {
    pub fn new(workspace_read: bool, http: bool, tool_invoke: bool) -> Self {
        Self {
            workspace_read,
            http,
            tool_invoke,
        }
    }

    pub fn workspace_read(mut self) -> Self {
        self.workspace_read = true;
        self
    }

    pub fn http(mut self) -> Self {
        self.http = true;
        self
    }

    pub fn tool_invoke(mut self) -> Self {
        self.tool_invoke = true;
        self
    }
}

/// A tool that was registered by a plugin via PluginContext.
/// The WASM handler reference lives here until the host collects it.
#[derive(Debug, Clone)]
pub struct RegisteredTool {
    pub name: String,
    pub description: String,
    pub schema: String,
    pub capabilities: Vec<String>,
}

/// A CLI command that was registered by a plugin via PluginContext.
#[derive(Debug, Clone)]
pub struct RegisteredCommand {
    pub name: String,
    pub description: String,
    pub aliases: Vec<String>,
}

/// An injected message stored for Phase 2 delivery to the agent queue.
#[derive(Debug, Clone)]
pub struct QueuedMessage {
    pub content: String,
    pub role: String,
}

/// The plugin registration context — the Hermes-style `ctx` API.
///
/// Plugin developers interact with this type via WASM exports.
/// The host creates one instance per plugin and calls the plugin's
/// `init()` function with it. All registrations are collected later
/// by the plugin loader.
#[derive(Debug)]
pub struct PluginContext {
    capabilities: PluginCapabilities,
    tools: Arc<Mutex<HashMap<String, RegisteredTool>>>,
    commands: Arc<Mutex<HashMap<String, RegisteredCommand>>>,
    message_queue: Arc<Mutex<Vec<QueuedMessage>>>,
}

impl PluginContext {
    /// Create a new PluginContext for a plugin.
    pub fn new(capabilities: PluginCapabilities) -> Self {
        Self {
            capabilities,
            tools: Arc::new(Mutex::new(HashMap::new())),
            commands: Arc::new(Mutex::new(HashMap::new())),
            message_queue: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Register a tool with the host.
    ///
    /// This stores the tool metadata in the internal map. The actual
    /// ToolRegistry push happens at load time by the plugin loader.
    ///
    /// # Parameters
    /// - `name`: Tool name (unique within this plugin)
    /// - `description`: Human-readable description
    /// - `schema`: JSON Schema string describing the tool's arguments
    ///
    /// # Example (WASM guest):
    /// ```wit
    /// ctx.register-tool("weather", "Get current weather", "{...schema...}", ["workspace-read"])
    /// ```
    pub async fn register_tool(
        &self,
        name: &str,
        description: &str,
        schema: &str,
        capabilities: Vec<String>,
    ) {
        self.tools.lock().await.insert(
            name.to_string(),
            RegisteredTool {
                name: name.to_string(),
                description: description.to_string(),
                schema: schema.to_string(),
                capabilities,
            },
        );
    }

    /// Register a CLI command with the host.
    ///
    /// Stores the command metadata in the internal map. The actual
    /// CLI subcommand registration happens at load time.
    pub async fn register_command(
        &self,
        name: &str,
        description: &str,
        aliases: Vec<String>,
    ) {
        self.commands.lock().await.insert(
            name.to_string(),
            RegisteredCommand {
                name: name.to_string(),
                description: description.to_string(),
                aliases,
            },
        );
    }

    /// Phase 1: Queue a message for later injection into the agent queue.
    ///
    /// Phase 2: This will connect to the actual agent message channel
    /// once the agent loop is running.
    pub async fn inject_message(&self, content: &str, role: &str) {
        self.message_queue.lock().await.push(QueuedMessage {
            content: content.to_string(),
            role: role.to_string(),
        });
    }

    /// Phase 1: Always returns an error.
    ///
    /// The LLM access interface is deferred until Phase 2 when the
    /// host can bind to the active model config. This avoids the
    /// chicken-and-egg problem of needing the model config before
    /// the plugin is loaded (which needs the model config).
    pub async fn llm_complete(&self, _messages: &[(String, String)]) -> Result<String, String> {
        Err("llm-not-available".to_string())
    }

    /// Collect all registered tools.
    pub async fn take_tools(&self) -> Vec<RegisteredTool> {
        let mut tools = self.tools.lock().await;
        let mut result: Vec<_> = tools.drain().map(|(_, v)| v).collect();
        result.sort_by(|a, b| a.name.cmp(&b.name));
        result
    }

    /// Collect all registered commands.
    pub async fn take_commands(&self) -> Vec<RegisteredCommand> {
        let mut commands = self.commands.lock().await;
        let mut result: Vec<_> = commands.drain().map(|(_, v)| v).collect();
        result.sort_by(|a, b| a.name.cmp(&b.name));
        result
    }

    /// Collect all queued messages.
    pub async fn take_messages(&self) -> Vec<QueuedMessage> {
        self.message_queue.lock().await.drain(..).collect()
    }

    /// Get the declared capabilities.
    pub fn capabilities(&self) -> &PluginCapabilities {
        &self.capabilities
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plugin_capabilities_default() {
        let caps = PluginCapabilities::default();
        assert!(!caps.workspace_read);
        assert!(!caps.http);
        assert!(!caps.tool_invoke);
    }

    #[test]
    fn test_plugin_capabilities_builder() {
        let caps = PluginCapabilities::new(true, false, false);
        assert!(caps.workspace_read);
        assert!(!caps.http);
        assert!(!caps.tool_invoke);
    }

    #[tokio::test]
    async fn test_register_and_collect_tool() {
        let ctx = PluginContext::new(PluginCapabilities::default());
        ctx.register_tool("test-tool", "A test tool", "{}", vec![]).await;
        let tools = ctx.take_tools().await;
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "test-tool");
        assert_eq!(tools[0].schema, "{}");
    }

    #[tokio::test]
    async fn test_register_and_collect_command() {
        let ctx = PluginContext::new(PluginCapabilities::default());
        ctx.register_command("test-cmd", "A test command", vec!["-t".to_string(), "--test".to_string()]).await;
        let cmds = ctx.take_commands().await;
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].name, "test-cmd");
    }

    #[tokio::test]
    async fn test_inject_message() {
        let ctx = PluginContext::new(PluginCapabilities::default());
        ctx.inject_message("Hello", "user").await;
        let msgs = ctx.take_messages().await;
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "Hello");
        assert_eq!(msgs[0].role, "user");
    }

    #[tokio::test]
    async fn test_llm_complete_returns_error() {
        let ctx = PluginContext::new(PluginCapabilities::default());
        let result = ctx.llm_complete(&[]).await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "llm-not-available");
    }

    #[tokio::test]
    async fn test_take_tools_cleans_map() {
        let ctx = PluginContext::new(PluginCapabilities::default());
        ctx.register_tool("tool-a", "desc", "{}", vec![]).await;
        ctx.register_tool("tool-b", "desc", "{}", vec![]).await;
        let first = ctx.take_tools().await;
        let second = ctx.take_tools().await;
        assert_eq!(first.len(), 2);
        assert_eq!(second.len(), 0); // drained
    }
}
