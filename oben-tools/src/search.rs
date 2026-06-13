use super::registry::{Tool, ToolCall, ToolRegistry};
use oben_models::{ToolMeta, ToolParameter, ToolParameters, ToolResult};

// ---------------------------------------------------------------------------
// Tool definition
// ---------------------------------------------------------------------------

fn make_search_tool_def() -> ToolMeta {
    ToolMeta {
        name: "web_search".into(),
        description: "Search the web for information".into(),
        parameters: ToolParameters::Flat(vec![
            ToolParameter::required("query", "Search query", "string"),
            ToolParameter::optional("max_results", "Maximum number of results", "number"),
        ]),
    }
}// ---------------------------------------------------------------------------
// Tool struct
// ---------------------------------------------------------------------------

pub struct WebSearchTool;

/// Placeholder handler — requires search provider configuration.
async fn execute_web_search<'a>(call: &ToolCall<'a>) -> anyhow::Result<ToolResult> {
    let query = call
        .required_str("query")?;

    let _max_results = call.optional_u64("max_results", 5);

    Ok(ToolResult {
        call_id: call.call_id.clone(),
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
    async fn execute(&self, call: &ToolCall) -> ToolResult {
        execute_web_search(call).await.unwrap_or_else(|e| ToolResult {
            call_id: call.call_id.clone(),
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
