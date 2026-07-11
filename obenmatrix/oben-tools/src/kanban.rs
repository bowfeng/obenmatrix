use oben_models::{ToolMeta, ToolParameter, ToolParameters};

use super::registry::{Tool, ToolCall, ToolRegistry};
use oben_models::ToolResult;

// ---------------------------------------------------------------------------
// Tool definition
// ---------------------------------------------------------------------------

fn make_kanban_tool() -> ToolMeta {
    ToolMeta {
        name: "kanban".into(),
        description: "Manage tasks on a Kanban board. Supports creating, updating, moving, and deleting tasks across columns (To Do, In Progress, Done).".into(),
        parameters: ToolParameters::Flat(vec![
            ToolParameter::required("action", "Action: 'create', 'update', 'move', or 'delete'", "string"),
            ToolParameter::optional("task_id", "Task ID for update/move/delete actions", "string"),
            ToolParameter::optional("title", "Task title for create/update actions", "string"),
            ToolParameter::optional("description", "Task description", "string"),
            ToolParameter::optional("column", "Target column (To Do, In Progress, Done) for move/create actions", "string"),
            ToolParameter::optional("priority", "Task priority (Low, Medium, High, Critical)", "string"),
        ]),
    }
}

pub struct KanbanTool;

async fn execute_kanban<'a>(call: &ToolCall<'a>) -> anyhow::Result<oben_models::ToolResult> {
    let action = call.required_str("action")?;
    
    // In a real implementation, this would:
    // 1. Load Kanban configuration
    // 2. Connect to Kanban backend (database/API)
    // 3. Execute the requested action
    // 4. Return task details
    
    // For now, return a placeholder response
    // TODO: Implement actual Kanban integration
    Ok(oben_models::ToolResult {
        call_id: call.call_id.clone(),
        output: format!("Kanban action '{}' placeholder (not yet implemented)", action),
        error: None,
    })
}

#[async_trait::async_trait]
impl Tool for KanbanTool {
    fn name(&self) -> &str {
        "kanban"
    }
    fn description(&self) -> &str {
        "Manage tasks on a Kanban board"
    }
    async fn execute(&self, call: &ToolCall) -> ToolResult {
        execute_kanban(call).await.unwrap_or_else(|e| ToolResult {
            call_id: call.call_id.clone(),
            output: String::new(),
            error: Some(e.to_string()),
        })
    }
    fn clone_tool(&self) -> Box<dyn Tool> {
        Box::new(Self)
    }
}

/// Register this module into the given registry.
pub fn register(registry: &mut ToolRegistry) {
    let tool = Box::new(KanbanTool);
    registry.register_with_def(tool, make_kanban_tool());
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Given: valid create action
    /// When: kanban tool is called with create
    /// Then: returns placeholder response
    #[tokio::test]
    async fn test_kanban_create() {
        let test_args = serde_json::json!({
            "call_id": "test-1",
            "action": "create",
            "title": "Implement feature",
            "column": "To Do"
        });
        
        let tool = KanbanTool;
        let call = ToolCall::new("kanban", &test_args);
        let result = tool.execute(&call).await;
        
        assert!(result.error.is_none());
        assert!(result.output.contains("placeholder"));
    }

    /// Given: missing action argument
    /// When: kanban tool is called
    /// Then: returns error "Missing 'action' argument"
    #[tokio::test]
    async fn test_kanban_missing_action() {
        let test_args = serde_json::json!({
            "call_id": "test-2"
        });
        
        let tool = KanbanTool;
        let call = ToolCall::new("kanban", &test_args);
        let result = tool.execute(&call).await;
        
        assert!(result.error.is_some());
    }
}
