/// File read/write tools.
///
/// Maps to `tools/read_write.py` / built-in file tools.

use anyhow::Result;
use oben_models::ToolResult;
use oben_utils::path_security::is_path_safe;
use std::path::PathBuf;

/// Read a file's contents.
pub async fn read_file(args: serde_json::Value) -> Result<ToolResult> {
    let path = args.get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'path' argument"))?;

    // Security: ensure path is within allowed directories
    if !is_path_safe(std::path::Path::new(path)) {
        return Ok(ToolResult {
            call_id: args.get("call_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            output: String::new(),
            error: Some("Unsafe file path".to_string()),
        });
    }

    let content = tokio::fs::read_to_string(path).await.map_err(|e| {
        anyhow::anyhow!("Failed to read {}: {}", path, e)
    })?;

    // Truncate very long files
    let content = if content.len() > 100_000 {
        format!("{}... (truncated, {} chars total)", &content[..100_000], content.len())
    } else {
        content
    };

    Ok(ToolResult {
        call_id: args.get("call_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        output: content,
        error: None,
    })
}

/// Write content to a file.
pub async fn write_file(args: serde_json::Value) -> Result<ToolResult> {
    let path = args.get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'path' argument"))?;

    let content = args.get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'content' argument"))?;

    if !is_path_safe(std::path::Path::new(path)) {
        return Ok(ToolResult {
            call_id: args.get("call_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            output: String::new(),
            error: Some("Unsafe file path".to_string()),
        });
    }

    let path_buf = PathBuf::from(path);
    let dir = path_buf.parent().unwrap_or(std::path::Path::new("."));
    tokio::fs::create_dir_all(dir).await?;
    tokio::fs::write(path, content).await?;

    Ok(ToolResult {
        call_id: args.get("call_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        output: format!("Wrote {} bytes to {}", content.len(), path),
        error: None,
    })
}
