/// Tool registry — stores and dispatches tool calls.
///
/// Maps to `tools/registry.py`.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use anyhow::Result;
use serde_json::Value;
use tracing::{info, warn};
use futures::future::BoxFuture;

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
