use serde_json::Value;

use super::registry::{Tool, ToolRegistry};
use oben_models::{ToolMeta, ToolParameter, ToolParameters, ToolResult};

// ---------------------------------------------------------------------------
// Tool definition
// ---------------------------------------------------------------------------

fn make_search_tool_def() -> ToolMeta {
    let params = vec![
        ToolParameter {
            name: "query".into(),
            description: "Search query".into(),
            parameter_type: "string".into(),
            required: true,
        },
        ToolParameter {
            name: "max_results".into(),
            description: "Maximum number of results".into(),
            parameter_type: "number".into(),
            required: false,
        },
    ];
    ToolMeta {
        name: "web_search".into(),
        description: "Search the web for information".into(),
        parameters: ToolParameters::Flat(params),
    }
}

// ---------------------------------------------------------------------------
// Tool struct
// ---------------------------------------------------------------------------

pub struct WebSearchTool;

/// Placeholder handler — requires search provider configuration.
async fn execute_web_search(args: &Value) -> anyhow::Result<ToolResult> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'query' argument"))?;

    let _max_results = args
        .get("max_results")
        .and_then(|v| v.as_u64())
        .unwrap_or(5);

    Ok(ToolResult {
        call_id: args
            .get("call_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        output: format!(
            "Web search for '{}': (placeholder - configure search provider in config)",
            query
        ),
        error: Some("Search provider not configured".to_string()),
    })
}

#[async_trait::async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }
    fn description(&self) -> &str {
        "Search the web for information"
    }
    async fn execute(&self, args: &Value) -> ToolResult {
        execute_web_search(args).await.unwrap_or_else(|e| ToolResult {
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
        Box::new(Self)
    }
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

/// Register this module into the given registry.
pub fn register(registry: &mut ToolRegistry) {
    let tool = Box::new(WebSearchTool);
    registry.register_with_def(tool, make_search_tool_def());
}
