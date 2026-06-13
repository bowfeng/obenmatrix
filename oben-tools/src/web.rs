use super::registry::{Tool, ToolCall, ToolRegistry};
use oben_models::{ToolMeta, ToolParameter, ToolParameters, ToolResult};

// ---------------------------------------------------------------------------
// Tool definition
// ---------------------------------------------------------------------------

fn make_http_get_tool_def() -> ToolMeta {
    ToolMeta {
        name: "http_get".into(),
        description: "Fetch a web page or API response".into(),
        parameters: ToolParameters::Flat(vec![
            ToolParameter::required("url", "The URL to fetch", "string"),
        ]),
    }
}// ---------------------------------------------------------------------------
// Tool struct
// ---------------------------------------------------------------------------

pub struct HttpGetTool;

/// Execute a raw HTTP GET request and return status, headers, and body.
async fn execute_http_get<'a>(call: &ToolCall<'a>) -> anyhow::Result<ToolResult> {
    let url = call
        .required_str("url")?;

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
        call_id: call.call_id.clone(),
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
    async fn execute(&self, call: &ToolCall) -> ToolResult {
        execute_http_get(call).await.unwrap_or_else(|e| ToolResult {
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
    let tool = Box::new(HttpGetTool);
    registry.register_with_def(tool, make_http_get_tool_def());
}
