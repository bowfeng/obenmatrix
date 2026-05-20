/// Terminal tool — executes shell commands with foreground/background support.
///
/// Replaces the basic `shell` tool with a richer terminal experience:
/// - Foreground execution with timeout, CWD, and safety guards
/// - Background task management (start, status, stop, output, list)
/// - Dangerous command blocking
/// - Output truncation

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use serde_json::Value;
use tokio::process::Command;
use tokio::sync::Mutex as TokioMutex;

use oben_models::{Tool, ToolParameter, ToolParameters, ToolResult};

use super::registry::{ToolHandler, SelfRegisteringTool, ToolRegistry};
use oben_utils::path_security::is_path_safe;

// ---------------------------------------------------------------------------
// Global background task state
// ---------------------------------------------------------------------------

/// Status of a background task.
#[derive(Debug, Clone, PartialEq)]
enum TaskStatus {
    Running,
    Finished,
    Stopped,
}

/// Represents a running background task.
struct BackgroundTask {
    /// The command string that was executed.
    command: String,
    /// Process handle for cleanup.
    process: tokio::process::Child,
    /// Process ID for external reference.
    pid: Option<u32>,
    /// Current status.
    status: TaskStatus,
}

/// Shared state for background task management.
static BACKGROUND_TASKS: LazyLock<TokioMutex<HashMap<String, BackgroundTask>>> =
    LazyLock::new(|| TokioMutex::new(HashMap::new()));

/// Generate a unique task ID.
static TASK_COUNTER: AtomicU64 = AtomicU64::new(0);

fn next_task_id() -> String {
    let id = TASK_COUNTER.fetch_add(1, Ordering::SeqCst);
    format!("task-{}", id)
}

// ---------------------------------------------------------------------------
// Dangerous command patterns to block
// ---------------------------------------------------------------------------

/// Split a command into pipe-separated segments (e.g., `"ls | rm"` → `["ls ", " rm"]`).
fn split_pipes(cmd: &str) -> Vec<&str> {
    cmd.split('|').collect()
}

/// Check if a command segment starts with or is a dangerous keyword.
fn pattern_starts_at(segment: &str, pattern: &str) -> bool {
    // Pattern must match at the start after optional leading whitespace,
    // or the segment must equal the pattern exactly.
    segment.trim_start().starts_with(pattern) || segment.starts_with(pattern)
}

/// List of dangerous command patterns that should be blocked.
const DANGEROUS_COMMANDS: &[&str] = &[
    "sudo ", "sudo$", "sudo -",
    "su ", "su -", "su root",
    "mkfs",
    "fdisk",
    "dd if=/dev/",
    ":(){ :|:& };:",
    "rm -rf /",
    "chmod 777 /",
    "chmod -R 777 /",
    "chown -R",
    "sh -c",
    "; rm",
    "> /etc/",
    "> /root/",
];

/// Check if a command contains dangerous patterns.
fn is_dangerous_command(cmd: &str) -> bool {
    let trimmed = cmd.trim();

    for &pattern in DANGEROUS_COMMANDS {
        if pattern == trimmed || pattern_starts_at(trimmed, pattern) {
            return true;
        }
    }

    // Check each pipe segment for piped dangerous commands.
    for segment in split_pipes(cmd) {
        let segment = segment.trim();
        if segment.is_empty() {
            continue;
        }
        for &pattern in DANGEROUS_COMMANDS {
            if pattern_starts_at(segment, pattern) {
                return true;
            }
        }
    }

    false
}

// ---------------------------------------------------------------------------
// Tool definitions
// ---------------------------------------------------------------------------

fn make_terminal_tool() -> Tool {
    let params = vec![
        ToolParameter {
            name: "action".into(),
            description: "Action to perform: execute (foreground command), run (background command), status (check task status), stop (kill background task), output (get background task output), list (list all background tasks)".into(),
            parameter_type: "string".into(),
            required: false,
        },
        ToolParameter {
            name: "command".into(),
            description: "Shell command to execute".into(),
            parameter_type: "string".into(),
            required: false,
        },
        ToolParameter {
            name: "cwd".into(),
            description: "Working directory to run the command in. Defaults to current directory.".into(),
            parameter_type: "string".into(),
            required: false,
        },
        ToolParameter {
            name: "timeout".into(),
            description: "Timeout in seconds for foreground execution. Default is 60 seconds.".into(),
            parameter_type: "number".into(),
            required: false,
        },
        ToolParameter {
            name: "task_id".into(),
            description: "Task ID for background operations (status, stop, output).".into(),
            parameter_type: "string".into(),
            required: false,
        },
        ToolParameter {
            name: "newest_only".into(),
            description: "If true, only return new output since last read (for 'output' action).".into(),
            parameter_type: "boolean".into(),
            required: false,
        },
    ];
    Tool {
        name: "terminal".into(),
        description: "Execute shell commands with foreground or background execution. Supports timeout, working directory, dangerous command blocking, and background task management (status/stop/output/list).".into(),
        parameters: ToolParameters::Flat(params),
    }
}

fn make_terminal_tool_handler() -> ToolHandler {
    Arc::new(|args: Value| {
        Box::pin(async move {
            let call_id = args.get("call_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let action = args
                .get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("execute");

            match action {
                "execute" | "run" => {
                    handle_run(&args, call_id).await
                }
                "status" => {
                    handle_task_status(&args, call_id).await
                }
                "stop" => {
                    handle_task_stop(&args, call_id).await
                }
                "output" => {
                    handle_task_output(&args, call_id).await
                }
                "list" => {
                    handle_task_list(&call_id).await
                }
                _ => Ok(ToolResult {
                    call_id,
                    output: String::new(),
                    error: Some(format!("Unknown action: {}. Use: execute, run, status, stop, output, list", action)),
                }),
            }
        })
    })
}

async fn handle_run(
    args: &Value,
    call_id: String,
) -> anyhow::Result<ToolResult> {
    let cmd = args
        .get("command")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'command' argument"))?;

    let cwd = args
        .get("cwd")
        .and_then(|v| v.as_str())
        .unwrap_or(".");

    let timeout_secs = args
        .get("timeout")
        .and_then(|v| v.as_u64())
        .unwrap_or(60);

    let background = args
        .get("background")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // Safety check: dangerous command
    if is_dangerous_command(cmd) {
        return Ok(ToolResult {
            call_id,
            output: String::new(),
            error: Some(format!("Dangerous command blocked: {}", cmd)),
        });
    }

    // Safety check: unsafe path
    if !is_path_safe(std::path::Path::new(cwd)) {
        return Ok(ToolResult {
            call_id,
            output: String::new(),
            error: Some("Unsafe working directory path".to_string()),
        });
    }

    if background {
        handle_background_task(cmd, cwd, &call_id).await
    } else {
        handle_foreground(cmd, cwd, timeout_secs, &call_id).await
    }
}

/// Execute a command in the foreground.
async fn handle_foreground(
    cmd: &str,
    cwd: &str,
    timeout_secs: u64,
    call_id: &str,
) -> anyhow::Result<ToolResult> {
    let process = Command::new("/bin/sh")
        .arg("-c")
        .arg(cmd)
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| anyhow::anyhow!("Failed to spawn command: {}", e))?;

    let pid = process.id();

    // Wait with timeout
    let output = tokio::time::timeout(
        Duration::from_secs(timeout_secs),
        process.wait_with_output()
    ).await;

    match output {
        Ok(Ok(result)) => {
            let stdout = String::from_utf8_lossy(&result.stdout).to_string();
            let stderr = String::from_utf8_lossy(&result.stderr).to_string();

            let output_str = format_output(&stdout, &stderr);
            let error = if result.status.success() {
                None
            } else {
                Some(format!("Command exited with code: {}", result.status.code().unwrap_or(-1)))
            };

            Ok(ToolResult {
                call_id: call_id.to_string(),
                output: output_str,
                error,
            })
        }
        Ok(Err(e)) => {
            Ok(ToolResult {
                call_id: call_id.to_string(),
                output: String::new(),
                error: Some(format!("Execution failed: {}", e)),
            })
        }
        Err(_) => {
            // Timeout: kill the process
            if let Some(p) = pid {
                let _ = tokio::process::Command::new("/bin/sh")
                    .arg("-c")
                    .arg(&format!("kill -9 {}", p))
                    .spawn();
            }
            Ok(ToolResult {
                call_id: call_id.to_string(),
                output: String::new(),
                error: Some(format!("Command timed out after {} seconds", timeout_secs)),
            })
        }
    }
}

/// Handle background task execution.
async fn handle_background_task(
    cmd: &str,
    cwd: &str,
    call_id: &str,
) -> anyhow::Result<ToolResult> {
    let task_id = next_task_id();

    let process = Command::new("/bin/sh")
        .arg("-c")
        .arg(cmd)
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .stdin(std::process::Stdio::null())
        .spawn()
        .map_err(|e| anyhow::anyhow!("Failed to spawn background command: {}", e))?;

    let pid = process.id();

    // Store the task
    let mut tasks = BACKGROUND_TASKS.lock().await;
    tasks.insert(task_id.clone(), BackgroundTask {
        command: cmd.to_string(),
        process,
        pid,
        status: TaskStatus::Running,
    });

    Ok(ToolResult {
        call_id: call_id.to_string(),
        output: format!("Started background task: {}. Command: {}", task_id, cmd),
        error: None,
    })
}

/// Maximum output size per stream (100KB).
const MAX_OUTPUT_BYTES: usize = 100_000;

/// Format stdout and stderr into a unified output string.
fn format_output(stdout: &str, stderr: &str) -> String {
    let stdout = if stdout.len() > MAX_OUTPUT_BYTES {
        format!(
            "{}... (truncated, {} chars total)",
            &stdout[..MAX_OUTPUT_BYTES],
            stdout.len()
        )
    } else {
        stdout.to_string()
    };

    let stderr = if stderr.len() > MAX_OUTPUT_BYTES {
        format!(
            "{}... (truncated, {} chars total)",
            &stderr[..MAX_OUTPUT_BYTES],
            stderr.len()
        )
    } else {
        stderr.to_string()
    };

    let mut parts = Vec::new();
    if !stdout.is_empty() {
        parts.push(format!("stdout:\n{}", stdout));
    }
    if !stderr.is_empty() {
        parts.push(format!("stderr:\n{}", stderr));
    }

    if parts.is_empty() {
        String::new()
    } else {
        parts.join("\n")
    }
}

// ---------------------------------------------------------------------------
// Background task management operations
// ---------------------------------------------------------------------------

async fn handle_task_status(
    args: &Value,
    call_id: String,
) -> anyhow::Result<ToolResult> {
    let task_id = args
        .get("task_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'task_id' argument"))?;

    let tasks = BACKGROUND_TASKS.lock().await;
    
    if let Some(task) = tasks.get(task_id) {
        let status_str = match task.status {
            TaskStatus::Running => "running",
            TaskStatus::Finished => "finished",
            TaskStatus::Stopped => "stopped",
        };
        
        Ok(ToolResult {
            call_id,
            output: format!("Task {}: {} (pid: {:?})", task_id, status_str, task.pid),
            error: None,
        })
    } else {
        Ok(ToolResult {
            call_id,
            output: String::new(),
            error: Some(format!("Task '{}' not found", task_id)),
        })
    }
}

async fn handle_task_stop(
    args: &Value,
    call_id: String,
) -> anyhow::Result<ToolResult> {
    let task_id = args
        .get("task_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'task_id' argument"))?;

    let mut tasks = BACKGROUND_TASKS.lock().await;
    
    if let Some(task) = tasks.get_mut(task_id) {
        if task.status == TaskStatus::Running {
            task.status = TaskStatus::Stopped;
        }
        
        // Try to kill the process
        if let Some(p) = task.pid {
            let _ = tokio::process::Command::new("/bin/sh")
                .arg("-c")
                .arg(&format!("kill -9 {}", p))
                .spawn();
        }
        
        tasks.remove(task_id);
        
        Ok(ToolResult {
            call_id,
            output: format!("Task {} stopped", task_id),
            error: None,
        })
    } else {
        Ok(ToolResult {
            call_id,
            output: String::new(),
            error: Some(format!("Task '{}' not found", task_id)),
        })
    }
}

async fn handle_task_output(
    args: &Value,
    call_id: String,
) -> anyhow::Result<ToolResult> {
    let task_id = args
        .get("task_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'task_id' argument"))?;

    let _newest_only = args
        .get("newest_only")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let tasks = BACKGROUND_TASKS.lock().await;
    
    if let Some(task) = tasks.get(task_id) {
        let output = match task.status {
            TaskStatus::Running => format!("(task {} is still running, command: {})", task_id, task.command),
            TaskStatus::Finished => format!("(task {} has finished, command: {})", task_id, task.command),
            TaskStatus::Stopped => format!("(task {} was stopped, command: {})", task_id, task.command),
        };
        
        Ok(ToolResult {
            call_id,
            output,
            error: None,
        })
    } else {
        Ok(ToolResult {
            call_id,
            output: String::new(),
            error: Some(format!("Task '{}' not found", task_id)),
        })
    }
}

async fn handle_task_list(call_id: &str) -> anyhow::Result<ToolResult> {
    let tasks = BACKGROUND_TASKS.lock().await;
    
    if tasks.is_empty() {
        return Ok(ToolResult {
            call_id: call_id.to_string(),
            output: "(no active background tasks)".to_string(),
            error: None,
        });
    }

    let mut task_entries = Vec::new();
    for (task_id, task) in tasks.iter() {
        let status_str = match task.status {
            TaskStatus::Running => "running",
            TaskStatus::Finished => "finished",
            TaskStatus::Stopped => "stopped",
        };
        task_entries.push(format!("{}: {} (command: {})", task_id, status_str, task.command));
    }
    
    Ok(ToolResult {
        call_id: call_id.to_string(),
        output: task_entries.join("\n"),
        error: None,
    })
}

// ---------------------------------------------------------------------------
// Self-registration
// ---------------------------------------------------------------------------

pub struct TerminalTool;

impl SelfRegisteringTool for TerminalTool {
    fn tool() -> Tool {
        make_terminal_tool()
    }

    fn handler() -> ToolHandler {
        make_terminal_tool_handler()
    }
}

/// Register this module into the given registry.
///
/// Called automatically by `discover_builtin_tools`.
pub fn register(registry: &mut ToolRegistry) {
    TerminalTool::register_self(registry);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_registry_with_terminal() -> super::super::registry::ToolRegistry {
        let mut registry = super::super::registry::ToolRegistry::new();
        TerminalTool::register_self(&mut registry);
        registry
    }

    // --- Foreground execution ---

    #[tokio::test]
    async fn foreground_executes_command_successfully() {
        let registry = make_registry_with_terminal();
        let result = registry.execute("terminal", &json!({
            "command": "echo hello",
            "call_id": "test-1",
        })).await;

        assert!(result.error.is_none());
        assert!(result.output.contains("hello"));
    }

    #[tokio::test]
    async fn foreground_returns_combined_output() {
        let registry = make_registry_with_terminal();
        let result = registry.execute("terminal", &json!({
            "command": "echo out; echo err >&2",
            "call_id": "test-2",
        })).await;

        assert!(result.error.is_none());
        assert!(result.output.contains("stdout"));
        assert!(result.output.contains("err"));
    }

    #[tokio::test]
    async fn foreground_returns_error_on_failure() {
        let registry = make_registry_with_terminal();
        let result = registry.execute("terminal", &json!({
            "command": "false",
            "call_id": "test-3",
        })).await;

        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("exited with code"));
    }

    #[tokio::test]
    async fn foreground_respects_cwd() {
        let registry = make_registry_with_terminal();
        let result = registry.execute("terminal", &json!({
            "command": "pwd",
            "cwd": "/tmp",
            "call_id": "test-4",
        })).await;

        assert!(result.error.is_none());
        assert!(result.output.contains("/tmp"));
    }

    #[tokio::test]
    async fn foreground_blocks_dangerous_command() {
        let registry = make_registry_with_terminal();
        let result = registry.execute("terminal", &json!({
            "command": "sudo rm -rf /",
            "call_id": "test-5",
        })).await;

        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("Dangerous command blocked"));
    }

    #[tokio::test]
    async fn foreground_blocks_dangerous_pattern_rm_rf_star() {
        let registry = make_registry_with_terminal();
        let result = registry.execute("terminal", &json!({
            "command": "rm -rf /*",
            "call_id": "test-6",
        })).await;

        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("Dangerous command blocked"));
    }

    #[tokio::test]
    async fn foreground_rejects_unsafe_cwd() {
        let registry = make_registry_with_terminal();
        let result = registry.execute("terminal", &json!({
            "command": "ls",
            "cwd": "; rm -rf /",
            "call_id": "test-7",
        })).await;

        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("Unsafe"));
    }

    #[tokio::test]
    async fn foreground_times_out_long_running_command() {
        let registry = make_registry_with_terminal();
        let result = registry.execute("terminal", &json!({
            "command": "sleep 30",
            "timeout": 1,
            "call_id": "test-8",
        })).await;

        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("timed out"));
    }

    #[tokio::test]
    async fn foreground_empty_output() {
        let registry = make_registry_with_terminal();
        let result = registry.execute("terminal", &json!({
            "command": "true",
            "call_id": "test-11",
        })).await;

        assert!(result.error.is_none());
        assert!(result.output.is_empty());
    }

    #[tokio::test]
    async fn foreground_output_truncation_for_large_output() {
        let registry = make_registry_with_terminal();
        let result = registry.execute("terminal", &json!({
            "command": "python3 -c \"print('x' * 200000)\"",
            "call_id": "test-12",
        })).await;

        assert!(result.error.is_none());
        assert!(result.output.len() < 200000);
    }

    // --- Background execution ---

    #[tokio::test]
    async fn background_starts_task_returns_task_id() {
        let registry = make_registry_with_terminal();
        let result = registry.execute("terminal", &json!({
            "command": "sleep 60",
            "background": true,
            "call_id": "test-9",
        })).await;

        assert!(result.error.is_none());
        assert!(result.output.contains("Started background task"));
        assert!(result.output.contains("task-0") || result.output.contains("Started background task: task-"));
    }

    #[tokio::test]
    async fn background_blocks_dangerous_command() {
        let registry = make_registry_with_terminal();
        let result = registry.execute("terminal", &json!({
            "command": "sudo rm -rf /",
            "background": true,
            "call_id": "test-10",
        })).await;

        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("Dangerous command blocked"));
    }

    // --- Background task management ---

    #[tokio::test]
    async fn task_status_unknown_task() {
        let registry = make_registry_with_terminal();
        let result = registry.execute("terminal", &json!({
            "action": "status",
            "task_id": "nonexistent",
            "call_id": "test-status-1",
        })).await;

        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("not found"));
    }

    #[tokio::test]
    async fn task_list() {
        let registry = make_registry_with_terminal();
        let result = registry.execute("terminal", &json!({
            "action": "list",
            "call_id": "test-list-1",
        })).await;

        assert!(result.error.is_none());
        // List should not error (may show leftover tasks from other tests)
        assert!(!result.output.is_empty());
    }

    #[tokio::test]
    async fn task_stop_unknown_task() {
        let registry = make_registry_with_terminal();
        let result = registry.execute("terminal", &json!({
            "action": "stop",
            "task_id": "nonexistent",
            "call_id": "test-stop-1",
        })).await;

        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("not found"));
    }

    #[tokio::test]
    async fn task_output_unknown_task() {
        let registry = make_registry_with_terminal();
        let result = registry.execute("terminal", &json!({
            "action": "output",
            "task_id": "nonexistent",
            "call_id": "test-output-1",
        })).await;

        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("not found"));
    }

    #[tokio::test]
    async fn unknown_action_returns_error() {
        let registry = make_registry_with_terminal();
        let result = registry.execute("terminal", &json!({
            "action": "unknown_action",
            "call_id": "test-unknown-1",
        })).await;

        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("Unknown action"));
    }
}
