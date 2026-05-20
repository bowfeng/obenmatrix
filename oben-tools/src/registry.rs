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
use anyhow::Result;
use serde_json::Value;
use tracing::{info, warn};
use oben_models::ToolResult;

// ---------------------------------------------------------------------------
// Tool trait — deep interface at the tool seam
// ---------------------------------------------------------------------------

/// A deep tool module. The interface is small (4 methods) but hides
/// the full implementation (parsing, validation, execution, error handling).
#[async_trait::async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn validate(&self, _args: &Value) -> Result<()> { Ok(()) }
    async fn execute(&self, args: &Value) -> ToolResult;
}

// ---------------------------------------------------------------------------
// SelfRegisteringTool — backward compat, now delegates to SelfRegisteringToolAdapter
// ---------------------------------------------------------------------------

/// Closure-based handler type alias (used by all existing tool modules).
pub type ToolHandler = Arc<dyn Fn(Value) -> Pin<Box<dyn Future<Output = Result<ToolResult>> + Send>> + Send + Sync>;

/// Trait for tools that register via (tool_def, handler) pair.
/// The impl block below converts this to a Tool.
pub trait SelfRegisteringTool {
    fn tool() -> oben_models::Tool;
    fn handler() -> ToolHandler;
    fn register_self(registry: &mut ToolRegistry) {
        registry.register(Box::new(SelfRegisteringToolAdapter::new(Self::tool(), Self::handler())));
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
    fn name(&self) -> &str { &self.tool_def.name }
    fn description(&self) -> &str { &self.tool_def.description }
    async fn execute(&self, args: &Value) -> ToolResult {
        (self.handler)(args.clone()).await.unwrap_or_else(|e| ToolResult {
            call_id: args.get("call_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            output: String::new(),
            error: Some(e.to_string()),
        })
    }
}

// ---------------------------------------------------------------------------
// ToolRegistry
// ---------------------------------------------------------------------------

pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self { Self { tools: HashMap::new() } }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        info!("Registering tool: {}", tool.name());
        self.tools.insert(tool.name().to_string(), tool);
    }

    pub fn list_tools(&self) -> Vec<oben_models::Tool> {
        self.tools.values().map(|t| oben_models::Tool {
            name: t.name().to_string(),
            description: t.description().to_string(),
            parameters: oben_models::ToolParameters::Flat(vec![]),
        }).collect()
    }

    pub async fn execute(&self, tool_name: &str, arguments: &Value) -> ToolResult {
        info!("Executing tool: {} with args...", tool_name);
        match self.tools.get(tool_name) {
            Some(tool) => {
                if let Err(e) = tool.validate(arguments) {
                    warn!("Tool {} validation failed: {}", tool_name, e);
                    return ToolResult {
                        call_id: arguments.get("call_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
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
                    call_id: arguments.get("call_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    output: String::new(),
                    error: Some(format!("Unknown tool: {}", tool_name)),
                }
            }
        }
    }

    pub fn has_tool(&self, name: &str) -> bool { self.tools.contains_key(name) }
    pub fn len(&self) -> usize { self.tools.len() }
    pub fn is_empty(&self) -> bool { self.tools.is_empty() }
}

impl Default for ToolRegistry { fn default() -> Self { Self::new() } }

pub fn discover_builtin_tools(registry: &mut ToolRegistry) {
    for module_fn in super::ALL_TOOLS { module_fn(registry); }
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
            oben_models::Tool { name: "echo-test".into(), description: "Test echo".into(), parameters: oben_models::ToolParameters::Flat(vec![]) }
        }
        fn handler() -> ToolHandler {
            Arc::new(|args: Value| Box::pin(async move {
                let msg = args.get("message").and_then(|v| v.as_str()).unwrap_or("no-msg");
                Ok(ToolResult { call_id: "t1".into(), output: format!("echo: {}", msg), error: None })
            }))
        }
    }

    struct FailTool;
    impl SelfRegisteringTool for FailTool {
        fn tool() -> oben_models::Tool {
            oben_models::Tool { name: "fail-test".into(), description: "Test fail".into(), parameters: oben_models::ToolParameters::Flat(vec![]) }
        }
        fn handler() -> ToolHandler {
            Arc::new(|_args: Value| Box::pin(async move {
                Ok(ToolResult { call_id: "t2".into(), output: String::new(), error: Some("boom".into()) })
            }))
        }
    }

    struct ValidatingTool;
    #[async_trait::async_trait]
    impl Tool for ValidatingTool {
        fn name(&self) -> &str { "val-tool" }
        fn description(&self) -> &str { "Validates args" }
        fn validate(&self, args: &Value) -> Result<()> {
            if args.get("block").and_then(|v| v.as_bool()) == Some(true) {
                Err(anyhow::anyhow!("Blocked by validation"))
            } else { Ok(()) }
        }
        async fn execute(&self, _args: &Value) -> ToolResult {
            ToolResult { call_id: "".into(), output: "ok".into(), error: None }
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
            oben_models::Tool { name: "x".into(), description: "x".into(), parameters: oben_models::ToolParameters::Flat(vec![]) },
            Arc::new(|_| Box::pin(async { Ok(ToolResult { call_id: "c".into(), output: "ok".into(), error: None }) })),
        )));
        assert_eq!(r.len(), 1);
        assert!(r.has_tool("x"));
    }

    #[tokio::test]
    async fn test_register_self_adaptor() {
        let mut r = ToolRegistry::new();
        EchoTool::register_self(&mut r);
        assert_eq!(r.len(), 1);
        let res = r.execute("echo-test", &serde_json::json!({"message": "hi"})).await;
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
            oben_models::Tool { name: "fail".into(), description: "fail".into(), parameters: oben_models::ToolParameters::Flat(vec![]) },
            Arc::new(|_| Box::pin(async { Ok(ToolResult { call_id: "c".into(), output: String::new(), error: Some("boom".into()) }) })),
        )));
        let res = r.execute("fail", &serde_json::json!({"call_id":"x"})).await;
        assert_eq!(res.error.as_ref().unwrap(), "boom");
    }

    #[tokio::test]
    async fn test_validation_blocks() {
        let mut r = ToolRegistry::new();
        r.register(Box::new(ValidatingTool));
        let res = r.execute("val-tool", &serde_json::json!({"block": true})).await;
        assert!(res.error.is_some());
        assert!(res.error.unwrap().contains("Validation"));
    }
}
