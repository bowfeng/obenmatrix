use anyhow::Result;
use oben_models::ToolResult;
use serde_json::Value;
/// Tool registry — stores and dispatches tool calls.
///
/// Maps to `tools/registry.py`.
///
/// The `Tool` trait is the deep interface at the tool seam. Callers create
/// structs that implement the trait, and the registry stores them as `Box<dyn
/// Tool>`. Universal pre-checks (validation before dispatch) apply across all
/// tools in `execute()`.
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tracing::{info, warn};

// ---------------------------------------------------------------------------
// Tool trait — deep interface at the tool seam
// ---------------------------------------------------------------------------

/// A deep tool module. The interface is small (4 methods) but hides
/// the full implementation (parsing, validation, execution, error handling).
#[async_trait::async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn validate(&self, _args: &Value) -> Result<()> {
        Ok(())
    }
    async fn execute(&self, args: &Value) -> ToolResult;
    /// Clone this tool into a new Box<dyn Tool>.
    fn clone_tool(&self) -> Box<dyn Tool>;
}

// ---------------------------------------------------------------------------
// SelfRegisteringTool — backward compat, now delegates to SelfRegisteringToolAdapter
// ---------------------------------------------------------------------------

/// Closure-based handler type alias (used by all existing tool modules).
pub type ToolHandler =
    Arc<dyn Fn(Value) -> Pin<Box<dyn Future<Output = Result<ToolResult>> + Send>> + Send + Sync>;

/// Spawn function for subagent delegation.
/// Maps to `delegate_task_handler` from hermes-agent.
///
/// Parameters:
/// - `parent_session_id`: session ID of the delegating agent
/// - `goal`: task description for the child
/// - `agent_depth`: current nesting depth (0 = parent, increments on each delegate call)
/// - `role`: "leaf" (default, cannot delegate further) or "orchestrator" (can delegate further if depth < max)
///
/// Creates a child session in the shared database, then spawns a child agent.
/// Returns a `JoinHandle` for the child's async execution.
pub type SpawnFn = Arc<dyn Fn(String, String, usize, &str) -> tokio::task::JoinHandle<Result<SubagentResult>> + Send + Sync>;

/// Result of executing a child agent delegation run.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct SubagentResult {
    /// Whether the child completed successfully.
    pub status: String,
    /// The child's final response (truncated to 500 chars).
    pub summary: String,
    /// Number of API calls the child made.
    pub api_calls: u32,
    /// How long the child ran (monotonic seconds).
    pub duration_seconds: f64,
    /// The model the child used.
    pub model: Option<String>,
    /// The child's session ID in the shared database.
    pub session_id: String,
    /// The parent session ID that spawned this child.
    pub parent_session_id: String,
    /// Role: "leaf" or "orchestrator".
    pub role: Option<String>,
    /// Depth of this subagent in the delegate tree.
    pub depth: usize,
    /// Optional exit reason (for parity).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_reason: Option<String>,
}

/// Trait for tools that register via (tool_def, handler) pair.
/// The impl block below converts this to a Tool.
pub trait SelfRegisteringTool {
    fn tool() -> oben_models::Tool;
    fn handler() -> ToolHandler;
    fn register_self(registry: &mut ToolRegistry) {
        let tool = Self::tool();
        let handler = Self::handler();
        let adapter = Box::new(SelfRegisteringToolAdapter::new(
            tool.clone(),
            handler,
        ));
        registry.register_with_def(adapter, tool);
    }
}

/// Adapter that satisfies Tool from a (oben_models::Tool, ToolHandler) pair.
pub(crate) struct SelfRegisteringToolAdapter {
    tool_def: oben_models::Tool,
    handler: ToolHandler,
}

impl SelfRegisteringToolAdapter {
    pub(crate) fn new(tool_def: oben_models::Tool, handler: ToolHandler) -> Self {
        Self { tool_def, handler }
    }
}

#[async_trait::async_trait]
impl Tool for SelfRegisteringToolAdapter {
    fn name(&self) -> &str {
        &self.tool_def.name
    }
    fn description(&self) -> &str {
        &self.tool_def.description
    }
    async fn execute(&self, args: &Value) -> ToolResult {
        (self.handler)(args.clone())
            .await
            .unwrap_or_else(|e| ToolResult {
                call_id: args
                    .get("call_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                output: String::new(),
                error: Some(e.to_string()),
            })
    }
    fn clone_tool(&self) -> Box<dyn Tool> {
        Box::new(SelfRegisteringToolAdapter {
            tool_def: self.tool_def.clone(),
            handler: Arc::clone(&self.handler),
        })
    }
}

// ---------------------------------------------------------------------------
// ToolRegistry
// ---------------------------------------------------------------------------

pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
    /// Full tool definitions with parameters, stored alongside the trait objects.
    /// Used by `list_tool_definitions()` to return complete specs to the LLM.
    tool_defs: HashMap<String, oben_models::Tool>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
            tool_defs: HashMap::new(),
        }
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        info!("Registering tool: {}", tool.name());
        self.tools.insert(tool.name().to_string(), tool);
    }

    /// Register a tool and store its full definition (including parameters).
    pub fn register_with_def(&mut self, tool: Box<dyn Tool>, def: oben_models::Tool) {
        let name = tool.name().to_string();
        info!("Registering tool: {}", name);
        self.tools.insert(name.clone(), tool);
        self.tool_defs.insert(name, def);
    }

    pub fn list_tools(&self) -> Vec<oben_models::Tool> {
        // Return stored tool definitions with full parameter specs.
        // Fall back to empty params for any tool registered without a def.
        let mut defs: Vec<oben_models::Tool> = self.tool_defs.values().cloned().collect();
        // If there are tools without definitions, fill them in with empty params
        for name in self.tools.keys() {
            if !defs.iter().any(|d| d.name == *name) {
                let tool = &self.tools[name];
                defs.push(oben_models::Tool {
                    name: tool.name().to_string(),
                    description: tool.description().to_string(),
                    parameters: oben_models::ToolParameters::Flat(vec![]),
                });
            }
        }
        defs
    }

    pub async fn execute(&self, tool_name: &str, arguments: &Value) -> ToolResult {
        info!("Executing tool: {} with args...", tool_name);
        match self.tools.get(tool_name) {
            Some(tool) => {
                if let Err(e) = tool.validate(arguments) {
                    warn!("Tool {} validation failed: {}", tool_name, e);
                    return ToolResult {
                        call_id: arguments
                            .get("call_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        output: String::new(),
                        error: Some(format!("Validation: {}", e)),
                    };
                }
                let result = tool.execute(arguments).await;
                if result.error.is_none() {
                    info!("Tool {} succeeded", tool_name);
                } else {
                    warn!("Tool {} failed: {:?}", tool_name, result.error);
                }
                result
            }
            None => {
                warn!("Unknown tool: {}", tool_name);
                ToolResult {
                    call_id: arguments
                        .get("call_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    output: String::new(),
                    error: Some(format!("Unknown tool: {}", tool_name)),
                }
            }
        }
    }

    pub fn has_tool(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    /// Create a new registry containing only the tools NOT in the `blocked` list.
    /// Used by `build_child_toolset` to restrict subagent tool access.
    pub fn filtered_clone(&self, blocked: &[&str]) -> Self {
        let blocked_set: std::collections::HashSet<&str> = blocked.iter().copied().collect();
        let mut filtered = Self::new();
        for name in self.tools.keys() {
            if blocked_set.contains(name.as_str()) {
                continue;
            }
            if let Some(tool) = self.tools.get(name) {
                // Clone via re-registering the boxed dyn Tool
                filtered.register(tool.as_ref().clone_tool());
                if let Some(def) = self.tool_defs.get(name) {
                    filtered.tool_defs.insert(name.clone(), def.clone());
                }
            }
        }
        filtered
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}

impl Clone for ToolRegistry {
    fn clone(&self) -> Self {
        let mut cloned = Self::new();
        for (name, tool) in &self.tools {
            cloned.tools.insert(name.clone(), tool.clone_tool());
        }
        for (name, def) in &self.tool_defs {
            cloned.tool_defs.insert(name.clone(), def.clone());
        }
        cloned
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub fn discover_builtin_tools(registry: &mut ToolRegistry) {
    for module_fn in super::ALL_TOOLS {
        module_fn(registry);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    struct EchoTool;
    impl SelfRegisteringTool for EchoTool {
        fn tool() -> oben_models::Tool {
            oben_models::Tool {
                name: "echo-test".into(),
                description: "Test echo".into(),
                parameters: oben_models::ToolParameters::Flat(vec![]),
            }
        }
        fn handler() -> ToolHandler {
            Arc::new(|args: Value| {
                Box::pin(async move {
                    let msg = args
                        .get("message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("no-msg");
                    Ok(ToolResult {
                        call_id: "t1".into(),
                        output: format!("echo: {}", msg),
                        error: None,
                    })
                })
            })
        }
    }

    struct FailTool;
    impl SelfRegisteringTool for FailTool {
        fn tool() -> oben_models::Tool {
            oben_models::Tool {
                name: "fail-test".into(),
                description: "Test fail".into(),
                parameters: oben_models::ToolParameters::Flat(vec![]),
            }
        }
        fn handler() -> ToolHandler {
            Arc::new(|_args: Value| {
                Box::pin(async move {
                    Ok(ToolResult {
                        call_id: "t2".into(),
                        output: String::new(),
                        error: Some("boom".into()),
                    })
                })
            })
        }
    }

    struct ValidatingTool;
    #[async_trait::async_trait]
    impl Tool for ValidatingTool {
        fn name(&self) -> &str {
            "val-tool"
        }
        fn description(&self) -> &str {
            "Validates args"
        }
        fn validate(&self, args: &Value) -> Result<()> {
            if args.get("block").and_then(|v| v.as_bool()) == Some(true) {
                Err(anyhow::anyhow!("Blocked by validation"))
            } else {
                Ok(())
            }
        }
        async fn execute(&self, _args: &Value) -> ToolResult {
            ToolResult {
                call_id: "".into(),
                output: "ok".into(),
                error: None,
            }
        }
        fn clone_tool(&self) -> Box<dyn Tool> {
            Box::new(ValidatingTool)
        }
    }

    #[tokio::test]
    async fn test_empty() {
        let r = ToolRegistry::new();
        assert!(r.is_empty());
        assert_eq!(r.len(), 0);
    }

    #[tokio::test]
    async fn test_register_list_has() {
        let mut r = ToolRegistry::new();
        r.register(Box::new(SelfRegisteringToolAdapter::new(
            oben_models::Tool {
                name: "x".into(),
                description: "x".into(),
                parameters: oben_models::ToolParameters::Flat(vec![]),
            },
            Arc::new(|_| {
                Box::pin(async {
                    Ok(ToolResult {
                        call_id: "c".into(),
                        output: "ok".into(),
                        error: None,
                    })
                })
            }),
        )));
        assert_eq!(r.len(), 1);
        assert!(r.has_tool("x"));
    }

    #[tokio::test]
    async fn test_register_self_adaptor() {
        let mut r = ToolRegistry::new();
        EchoTool::register_self(&mut r);
        assert_eq!(r.len(), 1);
        let res = r
            .execute("echo-test", &serde_json::json!({"message": "hi"}))
            .await;
        assert_eq!(res.output, "echo: hi");
    }

    #[tokio::test]
    async fn test_unknown_tool() {
        let r = ToolRegistry::new();
        let res = r.execute("nope", &serde_json::json!({})).await;
        assert!(res.error.is_some());
        assert!(res.error.unwrap().contains("Unknown tool"));
    }

    #[tokio::test]
    async fn test_handler_error() {
        let mut r = ToolRegistry::new();
        r.register(Box::new(SelfRegisteringToolAdapter::new(
            oben_models::Tool {
                name: "fail".into(),
                description: "fail".into(),
                parameters: oben_models::ToolParameters::Flat(vec![]),
            },
            Arc::new(|_| {
                Box::pin(async {
                    Ok(ToolResult {
                        call_id: "c".into(),
                        output: String::new(),
                        error: Some("boom".into()),
                    })
                })
            }),
        )));
        let res = r.execute("fail", &serde_json::json!({"call_id":"x"})).await;
        assert_eq!(res.error.as_ref().unwrap(), "boom");
    }

    #[tokio::test]
    async fn test_validation_blocks() {
        let mut r = ToolRegistry::new();
        r.register(Box::new(ValidatingTool));
        let res = r
            .execute("val-tool", &serde_json::json!({"block": true}))
            .await;
        assert!(res.error.is_some());
        assert!(res.error.unwrap().contains("Validation"));
    }
}
