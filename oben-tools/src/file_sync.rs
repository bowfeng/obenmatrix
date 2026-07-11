use oben_models::{ToolMeta, ToolParameter, ToolParameters};

use super::registry::{Tool, ToolCall, ToolRegistry};
use oben_models::ToolResult;

// ---------------------------------------------------------------------------
// Tool definition
// ---------------------------------------------------------------------------

fn make_file_sync_tool() -> ToolMeta {
    ToolMeta {
        name: "file_sync".into(),
        description: "Sync files between local workspace and remote workspace. Supports bidirectional sync for remote development.".into(),
        parameters: ToolParameters::Flat(vec![
            ToolParameter::required("action", "Action to perform: 'push' (local to remote), 'pull' (remote to local), or 'status'", "string"),
            ToolParameter::optional("file_path", "Optional specific file path to sync. If omitted, syncs entire workspace", "string"),
        ]),
    }
}

pub struct FileSyncTool;

async fn execute_file_sync<'a>(call: &ToolCall<'a>) -> anyhow::Result<oben_models::ToolResult> {
    let action = call.required_str("action")?;
    
    // In a real implementation, this would:
    // 1. Load file sync configuration (SSH/SFTP settings, cloud storage credentials)
    // 2. Perform the requested sync operation
    // 3. Return sync results
    
    // For now, return a placeholder response
    // TODO: Implement actual file sync integration
    Ok(oben_models::ToolResult {
        call_id: call.call_id.clone(),
        output: format!("File sync action '{}' placeholder (not yet implemented)", action),
        error: None,
    })
}

#[async_trait::async_trait]
impl Tool for FileSyncTool {
    fn name(&self) -> &str {
        "file_sync"
    }
    fn description(&self) -> &str {
        "Sync files between local and remote workspace"
    }
    async fn execute(&self, call: &ToolCall) -> ToolResult {
        execute_file_sync(call).await.unwrap_or_else(|e| ToolResult {
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
    let tool = Box::new(FileSyncTool);
    registry.register_with_def(tool, make_file_sync_tool());
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Given: valid push action
    /// When: file_sync tool is called with push
    /// Then: returns placeholder response
    #[tokio::test]
    async fn test_file_sync_push() {
        let test_args = serde_json::json!({
            "call_id": "test-1",
            "action": "push"
        });
        
        let tool = FileSyncTool;
        let call = ToolCall::new("file_sync", &test_args);
        let result = tool.execute(&call).await;
        
        assert!(result.error.is_none());
        assert!(result.output.contains("placeholder"));
    }

    /// Given: missing action argument
    /// When: file_sync tool is called
    /// Then: returns error "Missing 'action' argument"
    #[tokio::test]
    async fn test_file_sync_missing_action() {
        let test_args = serde_json::json!({
            "call_id": "test-2"
        });
        
        let tool = FileSyncTool;
        let call = ToolCall::new("file_sync", &test_args);
        let result = tool.execute(&call).await;
        
        assert!(result.error.is_some());
    }
}
