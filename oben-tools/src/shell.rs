/// Shell command execution tool.
///
/// Maps to core shell functionality in Hermes.

use anyhow::Result;
use oben_models::ToolResult;
use oben_utils::path_security::is_path_safe;

/// Execute a shell command safely.
pub async fn execute_shell(args: serde_json::Value) -> Result<ToolResult> {
    let cmd = args.get("command")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'command' argument"))?;

    let cwd = args.get("cwd")
        .and_then(|v| v.as_str())
        .unwrap_or(".");

    if !is_path_safe(std::path::Path::new(cwd)) {
        return Ok(ToolResult {
            call_id: args.get("call_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            output: String::new(),
            error: Some("Unsafe working directory path".to_string()),
        });
    }

    // Shell out - use /bin/sh for cross-platform
    let output = tokio::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(cmd)
        .current_dir(cwd)
        .output()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to execute command: {}", e))?;

    let status = output.status;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    let mut output_parts = Vec::new();
    if !stdout.is_empty() {
        output_parts.push(format!("stdout:\n{}", stdout));
    }
    if !stderr.is_empty() {
        output_parts.push(format!("stderr:\n{}", stderr));
    }

    let output_str = if output_parts.is_empty() {
        "(no output)".to_string()
    } else {
        output_parts.join("\n")
    };

    Ok(ToolResult {
        call_id: args.get("call_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        output: if !output_str.is_empty() { output_str } else { String::new() },
        error: if status.success() { None } else {
            Some(format!("Command exited with code: {}", status.code().unwrap_or(-1)))
        },
    })
}
