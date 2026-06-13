/// Toolset Filter — intercepts tool execution to block specific tools.
///
/// Wraps a `ToolRegistry` and checks tool names against a blocklist before
/// dispatching. Zero overhead when no tools are blocked (early return on empty
/// blocklist).
///
/// Maps to `tools/delegate_tool.py:DELEGATE_BLOCKED_TOOLS`.
use std::collections::BTreeSet;

/// Tools blocked from subagent execution (leaf agents at max depth cannot delegate).
pub static DELEGATE_BLOCKED_TOOLS: &[&str] = &["delegate_task"];
use std::sync::Arc;

use oben_models::ToolResult;
use serde_json::Value;
use tracing::warn;

use super::registry::{ToolCall, ToolRegistry};

/// A filter that wraps a [ToolRegistry] and blocks specific tools from
/// being executed.
///
/// # Example
///
/// ```
/// # use oben_tools::toolset_filter::BlockedToolFilter;
/// # use std::sync::Arc;
/// let registry = Arc::new(oben_tools::ToolRegistry::new());
/// let filter = BlockedToolFilter::new(registry, ["delegate_task", "memory"]);
/// // Calling filter.execute("delegate_task", ...) returns an error immediately.
/// // Other tool calls pass through to the registry.
/// ```
pub struct BlockedToolFilter {
    registry: Arc<ToolRegistry>,
    blocked: BTreeSet<String>,
}

impl BlockedToolFilter {
    /// Creates a new filter. When `blocked` is empty, every call passes
    /// through to the underlying registry with no overhead.
    pub fn new(
        registry: Arc<ToolRegistry>,
        blocked: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            registry,
            blocked: blocked.into_iter().map(Into::into).collect(),
        }
    }

    /// Executes a tool call by name.
    ///
    /// - If the tool name is in the blocklist, returns a `ToolResult` with
    ///   `error` set immediately.
    /// - Otherwise, dispatches to the underlying registry.
    pub async fn execute(&self, tool_name: &str, arguments: &Value) -> ToolResult {
        if self.blocked.contains(tool_name) {
            warn!("Tool '{}' is blocked", tool_name);
            return ToolResult {
                call_id: arguments
                    .get("call_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                output: String::new(),
                error: Some(format!(
                    "Tool '{}' is not available to subagents",
                    tool_name
                )),
            };
        }

        self.registry.execute(tool_name, arguments).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{Tool, ToolRegistry};

    // -----------------------------------------------------------------------
    // Test helpers
    // -----------------------------------------------------------------------

    fn make_registry() -> Arc<ToolRegistry> {
        let mut registry = ToolRegistry::new();

        struct ReadFileTool;
        #[async_trait::async_trait]
        impl Tool for ReadFileTool {
            fn name(&self) -> &str { "read_file" }
            fn description(&self) -> &str { "Read a file" }
            async fn execute(&self, _call: &ToolCall) -> ToolResult {
                ToolResult {
                    call_id: "1".into(),
                    output: "file contents".into(),
                    error: None,
                }
            }
            fn clone_tool(&self) -> Box<dyn Tool> { Box::new(Self) }
        }

        struct MemoryTool;
        #[async_trait::async_trait]
        impl Tool for MemoryTool {
            fn name(&self) -> &str { "memory" }
            fn description(&self) -> &str { "Read/write shared memory" }
            async fn execute(&self, _call: &ToolCall) -> ToolResult {
                ToolResult {
                    call_id: "2".into(),
                    output: "memory content".into(),
                    error: None,
                }
            }
            fn clone_tool(&self) -> Box<dyn Tool> { Box::new(Self) }
        }

        struct DelegateTool;
        #[async_trait::async_trait]
        impl Tool for DelegateTool {
            fn name(&self) -> &str { "delegate_task" }
            fn description(&self) -> &str { "Delegate a subtask" }
            async fn execute(&self, _call: &ToolCall) -> ToolResult {
                ToolResult {
                    call_id: "3".into(),
                    output: "delegated".into(),
                    error: None,
                }
            }
            fn clone_tool(&self) -> Box<dyn Tool> { Box::new(Self) }
        }

        registry.register(Box::new(ReadFileTool));
        registry.register(Box::new(MemoryTool));
        registry.register(Box::new(DelegateTool));

        Arc::new(registry)
    }

    // -----------------------------------------------------------------------
    // Actual tests
    // -----------------------------------------------------------------------

    /// Given: an empty blocklist
    /// When: a tool is executed
    /// Then: the tool passes through to the registry without blocking
    #[tokio::test]
    async fn test_empty_blocklist_passes_through() {
        let registry = make_registry();
        let filter = BlockedToolFilter::new(registry, Vec::<String>::new());

        // read_file is not blocked, handler returns Ok (no error)
        let result = filter
            .execute("read_file", &Value::String("read_file".to_string()))
            .await;
        assert!(result.error.is_none());
    }

    /// Given: a blocklist containing "memory"
    /// When: "memory" is executed and "read_file" is executed
    /// Then: "memory" returns a blocking error, "read_file" passes through
    #[tokio::test]
    async fn test_single_blocked_tool_returns_error() {
        let registry = make_registry();
        let filter = BlockedToolFilter::new(registry.clone(), ["memory".to_string()]);

        // Blocked tool — returns error
        let result = filter
            .execute("memory", &Value::String("memory".to_string()))
            .await;
        assert!(result.error.as_ref().unwrap().contains("not available"));

        // Non-blocked tool — passes through (success)
        let result = filter
            .execute("read_file", &Value::String("read_file".to_string()))
            .await;
        assert!(result.error.is_none());
    }

    /// Given: a blocklist containing "memory" and "delegate_task"
    /// When: both are executed and "read_file" is executed
    /// Then: both blocked tools return errors, non-blocked passes through
    #[tokio::test]
    async fn test_multiple_blocked_tools() {
        let registry = make_registry();
        let filter = BlockedToolFilter::new(
            registry,
            ["memory".to_string(), "delegate_task".to_string()],
        );

        assert!(filter
            .execute("memory", &Value::String("test".into()),)
            .await
            .error
            .as_ref()
            .unwrap()
            .contains("not available"));

        assert!(filter
            .execute("delegate_task", &Value::String("test".into()),)
            .await
            .error
            .as_ref()
            .unwrap()
            .contains("not available"));

        // read_file is not blocked
        let result = filter
            .execute("read_file", &Value::String("test".into()))
            .await;
        assert!(result.error.is_none());
    }

    /// Given: a blocked tool with explicit call_id in arguments
    /// When: the blocked tool is executed
    /// Then: the call_id is propagated to the result
    #[tokio::test]
    async fn test_call_id_propagation_on_blocked() {
        let registry = make_registry();
        let filter = BlockedToolFilter::new(registry, ["memory".to_string()]);

        let result = filter
            .execute(
                "memory",
                &serde_json::json!({"call_id": "abc-123", "action": "add"}),
            )
            .await;
        assert_eq!(result.call_id, "abc-123");
        assert!(result.error.unwrap().contains("not available"));
    }

    /// Given: a blocked tool list that does NOT include "nonexistent"
    /// When: a non-existent tool is executed
    /// Then: the registry's "Unknown tool" error is returned
    #[tokio::test]
    async fn test_unknown_tool_passes_through_registry() {
        let registry = make_registry();
        let filter = BlockedToolFilter::new(registry, ["memory".to_string()]);

        let result = filter
            .execute("nonexistent", &Value::String("test".into()))
            .await;
        assert!(result.error.as_ref().unwrap().contains("Unknown tool"));
    }
}
