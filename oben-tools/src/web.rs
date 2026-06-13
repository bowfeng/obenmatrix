use serde_json::Value;

use super::registry::{Tool, ToolRegistry};
use oben_models::{ToolMeta, ToolParameter, ToolParameters, ToolResult};

// ---------------------------------------------------------------------------
// Tool definition
// ---------------------------------------------------------------------------

fn make_http_get_tool_def() -> ToolMeta {
    let params = vec![ToolParameter {
        name: "url".into(),
        description: "URL to fetch".into(),
        parameter_type: "string".into(),
        required: true,
    }];
    ToolMeta {
        name: "http_get".into(),
        description: "Make an HTTP GET request".into(),
        parameters: ToolParameters::Flat(params),
    }
}

// ---------------------------------------------------------------------------
// Tool struct
// ---------------------------------------------------------------------------

pub struct HttpGetTool;

/// Execute a raw HTTP GET request and return status, headers, and body.
async fn execute_http_get(args: &Value) -> anyhow::Result<ToolResult> {
    let url = args
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'url' argument"))?;

    let client = reqwest::Client::new();
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Request failed: {}", e))?;

    let status = response.status();
    let headers: Vec<(String, String)> = response
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();

    let body = response.text().await?;

    let body: String = if body.chars().count() > 50_000 {
        format!(
            "{}... (truncated)",
            body.chars().take(50_000).collect::<String>()
        )
    } else {
        body
    };

    Ok(ToolResult {
        call_id: args
            .get("call_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        output: format!(
            "Status: {}\nHeaders: {}\nBody:\n{}",
            status,
            serde_json::to_string_pretty(&headers).unwrap_or_default(),
            body
        ),
        error: if status.is_success() {
            None
        } else {
            Some(format!("HTTP {}", status))
        },
    })
}

#[async_trait::async_trait]
impl Tool for HttpGetTool {
    fn name(&self) -> &str {
        "http_get"
    }
    fn description(&self) -> &str {
        "Make an HTTP GET request"
    }
    async fn execute(&self, args: &Value) -> ToolResult {
        execute_http_get(args).await.unwrap_or_else(|e| ToolResult {
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
    let tool = Box::new(HttpGetTool);
    registry.register_with_def(tool, make_http_get_tool_def());
}
