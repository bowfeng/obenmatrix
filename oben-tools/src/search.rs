/// Web search tool.
///
/// Maps to `agent/web_search_provider.py` and `tools/web_tools.py`.

use anyhow::Result;
use oben_models::ToolResult;

/// Simple web search using a request to a search API.
pub async fn web_search(args: serde_json::Value) -> Result<ToolResult> {
    let query = args.get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'query' argument"))?;

    let _max_results = args.get("max_results")
        .and_then(|v| v.as_u64())
        .unwrap_or(5);

    // Placeholder: in a full implementation, this would call a search API
    // (Google Custom Search, Exa, etc.)
    Ok(ToolResult {
        call_id: args.get("call_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        output: format!(
            "Web search for '{}': (placeholder - configure search provider in config)",
            query
        ),
        error: Some("Search provider not configured".to_string()),
    })
}
