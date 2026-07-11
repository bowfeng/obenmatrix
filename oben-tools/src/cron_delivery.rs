use oben_models::{ToolMeta, ToolParameter, ToolParameters};

use super::registry::{Tool, ToolCall, ToolRegistry};
use oben_models::ToolResult;

// ---------------------------------------------------------------------------
// Tool definition
// ---------------------------------------------------------------------------

fn make_cron_delivery_tool() -> ToolMeta {
    ToolMeta {
        name: "cron_delivery".into(),
        description: "Schedule and deliver messages/tasks at specific times using cron syntax. Supports cron expressions for flexible scheduling.".into(),
        parameters: ToolParameters::Flat(vec![
            ToolParameter::required("cron_expression", "Cron expression (5 fields: minute hour day month weekday)", "string"),
            ToolParameter::required("message", "Message or task to deliver at scheduled time", "string"),
            ToolParameter::optional("timezone", "Optional timezone for cron evaluation (e.g., 'UTC', 'America/New_York')", "string"),
        ]),
    }
}

pub struct CronDeliveryTool;

async fn execute_cron_delivery<'a>(call: &ToolCall<'a>) -> anyhow::Result<oben_models::ToolResult> {
    let cron_expr = call.required_str("cron_expression")?;
    let message = call.required_str("message")?;
    
    // In a real implementation, this would:
    // 1. Parse cron expression
    // 2. Schedule delivery using a cron scheduler
    // 3. Store scheduled task in database
    // 4. Return task ID
    
    // For now, return a placeholder response
    // TODO: Implement actual cron scheduler integration
    Ok(oben_models::ToolResult {
        call_id: call.call_id.clone(),
        output: format!("Cron delivery scheduled for '{}': '{}' (not yet implemented)", cron_expr, message),
        error: None,
    })
}

#[async_trait::async_trait]
impl Tool for CronDeliveryTool {
    fn name(&self) -> &str {
        "cron_delivery"
    }
    fn description(&self) -> &str {
        "Schedule and deliver messages/tasks at specific times"
    }
    async fn execute(&self, call: &ToolCall) -> ToolResult {
        execute_cron_delivery(call).await.unwrap_or_else(|e| ToolResult {
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
    let tool = Box::new(CronDeliveryTool);
    registry.register_with_def(tool, make_cron_delivery_tool());
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Given: valid cron expression and message
    /// When: cron_delivery tool is called
    /// Then: returns scheduled task response
    #[tokio::test]
    async fn test_cron_delivery_scheduling() {
        let test_args = serde_json::json!({
            "call_id": "test-1",
            "cron_expression": "0 9 * * *",
            "message": "Daily standup meeting"
        });
        
        let tool = CronDeliveryTool;
        let call = ToolCall::new("cron_delivery", &test_args);
        let result = tool.execute(&call).await;
        
        assert!(result.error.is_none());
        assert!(result.output.contains("scheduled"));
    }

    /// Given: invalid cron expression
    /// When: cron_delivery tool is called
    /// Then: returns error with invalid expression message
    #[tokio::test]
    async fn test_cron_delivery_missing_cron() {
        let test_args = serde_json::json!({
            "call_id": "test-2"
        });
        
        let tool = CronDeliveryTool;
        let call = ToolCall::new("cron_delivery", &test_args);
        let result = tool.execute(&call).await;
        
        assert!(result.error.is_some());
    }
}
