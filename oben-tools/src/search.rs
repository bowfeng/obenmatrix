use oben_models::{Tool, ToolParameter, ToolParameters, ToolResult};
use serde_json::Value;
/// Web search tool.
///
/// Self-registers via `SelfRegisteringTool` trait.
use std::sync::Arc;

use super::registry::{SelfRegisteringTool, ToolHandler, ToolRegistry};

fn make_search_tool() -> Tool {
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
    Tool {
        name: "web_search".into(),
        description: "Search the web for information".into(),
        parameters: ToolParameters::Flat(params),
    }
}

fn make_search_handler() -> ToolHandler {
    Arc::new(|args: Value| {
        Box::pin(async move {
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
        })
    })
}

pub struct SearchTool;

impl SelfRegisteringTool for SearchTool {
    fn tool() -> Tool {
        make_search_tool()
    }

    fn handler() -> ToolHandler {
        make_search_handler()
    }
}

/// Register this module into the given registry.
pub fn register(registry: &mut ToolRegistry) {
    SearchTool::register_self(registry);
}
