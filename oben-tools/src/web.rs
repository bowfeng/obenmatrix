use oben_models::{Tool, ToolParameter, ToolParameters, ToolResult};
use serde_json::Value;
/// Web request tools.
///
/// Self-registers via `SelfRegisteringTool` trait.
use std::sync::Arc;

use super::registry::{SelfRegisteringTool, ToolHandler, ToolRegistry};

fn make_http_get_tool() -> Tool {
    let params = vec![ToolParameter {
        name: "url".into(),
        description: "URL to fetch".into(),
        parameter_type: "string".into(),
        required: true,
    }];
    Tool {
        name: "http_get".into(),
        description: "Make an HTTP GET request".into(),
        parameters: ToolParameters::Flat(params),
    }
}

fn make_http_get_handler() -> ToolHandler {
    Arc::new(|args: Value| {
        Box::pin(async move {
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

            let body = if body.len() > 50_000 {
                format!("{}... (truncated)", &body[..50_000])
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
        })
    })
}

pub struct WebTools;

impl SelfRegisteringTool for WebTools {
    fn tool() -> Tool {
        make_http_get_tool()
    }

    fn handler() -> ToolHandler {
        make_http_get_handler()
    }
}

/// Register this module into the given registry.
pub fn register(registry: &mut ToolRegistry) {
    WebTools::register_self(registry);
}
