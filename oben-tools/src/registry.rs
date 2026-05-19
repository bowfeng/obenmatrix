/// Tool registry — stores and dispatches tool calls.
///
/// Maps to `tools/registry.py`.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use anyhow::Result;
use serde_json::Value;
#[cfg(test)]
use serde_json::json;
use tracing::{info, warn};


use oben_models::ToolResult;

/// A tool handler function.
pub type ToolHandler = Arc<dyn Fn(Value) -> Pin<Box<dyn Future<Output = Result<ToolResult>> + Send>> + Send + Sync>;

/// The global tool registry.
pub struct ToolRegistry {
    tools: HashMap<String, (oben_models::Tool, ToolHandler)>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    /// Register a tool with its handler.
    pub fn register(&mut self, tool: oben_models::Tool, handler: ToolHandler) {
        info!("Registering tool: {}", tool.name);
        self.tools.insert(tool.name.clone(), (tool, handler));
    }

    /// Get a list of available tools.
    pub fn list_tools(&self) -> Vec<&oben_models::Tool> {
        self.tools.values().map(|(tool, _)| tool).collect()
    }

    /// Execute a tool by name with arguments.
    pub async fn execute(&self, tool_name: &str, arguments: &Value) -> ToolResult {
        info!("Executing tool: {} with args", tool_name);

        match self.tools.get(tool_name) {
            Some((_tool, handler)) => {
                match handler(arguments.clone()).await {
                    Ok(result) => {
                        info!("Tool {} succeeded", tool_name);
                        result
                    }
                    Err(e) => {
                        warn!("Tool {} failed: {}", tool_name, e);
                        ToolResult {
                            call_id: arguments.get("call_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                            output: String::new(),
                            error: Some(e.to_string()),
                        }
                    }
                }
            }
            None => {
                warn!("Unknown tool: {}", tool_name);
                ToolResult {
                    call_id: arguments.get("call_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    output: String::new(),
                    error: Some(format!("Unknown tool: {}", tool_name)),
                }
            }
        }
    }

    /// Check if a tool is registered.
    pub fn has_tool(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    /// Get tool count.
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Trait for tool modules that can self-register into a registry.
///
/// Implement this to add a new tool. The module calls
/// `registry.register(Self::tool(), Self::handler())` in its
/// `register_self` method.
pub trait SelfRegisteringTool {
    /// The tool definition (name, description, parameters).
    fn tool() -> oben_models::Tool;
    /// The handler that executes the tool.
    fn handler() -> ToolHandler;
    /// Register this tool into the given registry.
    fn register_self(registry: &mut ToolRegistry) {
        registry.register(Self::tool(), Self::handler());
    }
}

/// Discover and register all built-in tool modules.
///
/// Each tool module implements `SelfRegisteringTool` and is registered
/// here. Add a new line to register additional tools.
pub fn discover_builtin_tools(registry: &mut ToolRegistry) {
    // Shell tool
    crate::shell::ShellTool::register_self(registry);
    // File tools (read + write)
    crate::read_write::register_file_tools(registry);
    // Web tools
    crate::web::WebTools::register_self(registry);
    // Search tool
    crate::search::SearchTool::register_self(registry);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_echo_handler() -> ToolHandler {
        Arc::new(|args: Value| {
            let msg = args.get("message").map(|v| v.as_str().unwrap_or("")).unwrap_or("no-msg").to_string();
            Box::pin(async move {
                Ok(ToolResult {
                    call_id: "test-1".to_string(),
                    output: format!("echo: {}", msg),
                    error: None,
                })
            })
        })
    }

    fn make_error_handler() -> ToolHandler {
        Arc::new(|_args: Value| {
            Box::pin(async move {
                Ok(ToolResult {
                    call_id: "test-2".to_string(),
                    output: String::new(),
                    error: Some("intentional failure".to_string()),
                })
            })
        })
    }

    fn make_tool(name: &str) -> oben_models::Tool {
        oben_models::Tool {
            name: name.to_string(),
            description: format!("Test tool: {}", name),
            parameters: oben_models::ToolParameters::Flat(vec![
                oben_models::ToolParameter {
                    name: "message".to_string(),
                    description: "Input message".to_string(),
                    parameter_type: "string".to_string(),
                    required: true,
                },
            ]),
        }
    }

    #[tokio::test]
    async fn test_registry_empty() {
        let registry = ToolRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
    }

    #[tokio::test]
    async fn test_registry_register_and_list() {
        let mut registry = ToolRegistry::new();
        let tool = make_tool("echo");
        registry.register(tool, make_echo_handler());
        assert_eq!(registry.len(), 1);
        assert!(registry.has_tool("echo"));
        assert!(!registry.has_tool("missing"));
    }

    #[tokio::test]
    async fn test_execute_registered_tool() {
        let mut registry = ToolRegistry::new();
        let tool = make_tool("echo");
        registry.register(tool, make_echo_handler());
        let result = registry.execute("echo", &json!({"message": "hello"})).await;
        assert!(result.error.is_none());
        assert_eq!(result.output, "echo: hello");
    }

    #[tokio::test]
    async fn test_execute_unknown_tool_returns_error() {
        let registry = ToolRegistry::new();
        let result = registry.execute("nonexistent", &json!({})).await;
        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("Unknown tool"));
    }

    #[tokio::test]
    async fn test_execute_tool_with_handler_error() {
        let mut registry = ToolRegistry::new();
        let tool = make_tool("fail");
        registry.register(tool, make_error_handler());
        let result = registry.execute("fail", &json!({"call_id": "t2"})).await;
        assert!(result.error.is_some());
        assert_eq!(result.error.unwrap(), "intentional failure");
    }

    #[tokio::test]
    async fn test_multiple_tools() {
        let mut registry = ToolRegistry::new();
        registry.register(make_tool("a"), make_echo_handler());
        registry.register(make_tool("b"), make_error_handler());
        assert_eq!(registry.len(), 2);
        assert!(registry.has_tool("a"));
        assert!(registry.has_tool("b"));
        let result_a = registry.execute("a", &json!({"message": "hi"})).await;
        assert_eq!(result_a.output, "echo: hi");
    }
}
