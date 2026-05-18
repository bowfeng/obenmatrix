/// Web request tools.
///
/// Maps to `tools/web_tools.py`.

use anyhow::Result;
use oben_models::ToolResult;

/// Make an HTTP GET request.
pub async fn http_get(args: serde_json::Value) -> Result<ToolResult> {
    let url = args.get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'url' argument"))?;

    let client = reqwest::Client::new();
    let response = client.get(url)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Request failed: {}", e))?;

    let status = response.status();
    let headers: Vec<(String, String)> = response.headers().iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();

    let body = response.text().await?;

    // Truncate response body
    let body = if body.len() > 50_000 {
        format!("{}... (truncated)", &body[..50_000])
    } else {
        body
    };

    Ok(ToolResult {
        call_id: args.get("call_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        output: format!(
            "Status: {}\nHeaders: {}\nBody:\n{}",
            status,
            serde_json::to_string_pretty(&headers).unwrap_or_default(),
            body
        ),
        error: if status.is_success() { None } else {
            Some(format!("HTTP {}", status))
        },
    })
}
