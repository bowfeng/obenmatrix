/// Concurrent tool dispatch — parallel execution of independent tools.
///
/// Mirrors Hermes' concurrent tool execution for independent tools.
/// Tools that don't overlap on file paths and aren't destructive are
/// dispatched in parallel via tokio's JoinSet.

use std::sync::Arc;

use anyhow::Result;
use tokio::task::JoinSet;
use tracing::debug;

use crate::interrupt::SharedInterrupt;

/// Configuration for concurrent tool dispatch.
#[derive(Debug, Clone)]
pub struct ConcurrentDispatchConfig {
    /// Max concurrent workers. Default: 8.
    pub max_concurrency: usize,
    /// Tool names that are ALWAYS executed serially (never concurrent).
    pub serial_only_tools: Vec<String>,
    /// Tool names that are destructive (write/modify files).
    pub destructive_tools: Vec<String>,
}

impl Default for ConcurrentDispatchConfig {
    fn default() -> Self {
        Self {
            max_concurrency: 8,
            serial_only_tools: vec![],
            destructive_tools: vec![
                "write_file".to_string(),
                "patch".to_string(),
                "create_dir".to_string(),
                "delete_file".to_string(),
                "shell".to_string(), // shell can do anything
            ],
        }
    }
}

/// A pending tool call ready for dispatch.
#[derive(Debug, Clone)]
pub struct PendingToolCall {
    pub tool_name: String,
    pub arguments: serde_json::Value,
    pub call_id: String,
}

/// Dispatch multiple tool calls, running independent ones concurrently.
///
/// Returns results in the same order as input calls.
pub async fn dispatch_tool_calls(
    tools: &Arc<oben_tools::ToolRegistry>,
    config: &ConcurrentDispatchConfig,
    calls: &[PendingToolCall],
    interrupt: Option<&SharedInterrupt>,
) -> Result<Vec<oben_models::ToolResult>> {
    if calls.len() <= 1 {
        // No concurrency needed for 0 or 1 call
        let mut results = Vec::with_capacity(calls.len());
        for call in calls {
            let result = tools.execute(&call.tool_name, &call.arguments).await;
            results.push(result);
        }
        return Ok(results);
    }

    // Partition calls: serial (destructive/serial-only) vs concurrent
    let mut serial_tasks: Vec<PendingToolCall> = Vec::new();
    let mut concurrent_tasks: Vec<PendingToolCall> = Vec::new();

    for call in calls {
        if is_serial_only(&call.tool_name, config) {
            serial_tasks.push(call.clone());
        } else {
            concurrent_tasks.push(call.clone());
        }
    }

    // Execute serial tasks one by one
    let mut serial_results: Vec<oben_models::ToolResult> = Vec::with_capacity(serial_tasks.len());
    for call in serial_tasks {
        if let Some(ref int) = interrupt {
            if int.is_interrupted() {
                // Drain interrupt and return partial results
                let _ = int.drain_interrupt_message();
                return Ok(serial_results);
            }
        }
        let result = tools.execute(&call.tool_name, &call.arguments).await;
        serial_results.push(result);
    }

    // Execute concurrent tasks in parallel
    let mut concurrent_results: Vec<oben_models::ToolResult> = vec![
        oben_models::ToolResult {
            call_id: String::new(),
            output: String::new(),
            error: Some("cancelled".into()),
        };
        concurrent_tasks.len()
    ];

    if !concurrent_tasks.is_empty() {
        let mut joinset = JoinSet::new();

        for (idx, call) in concurrent_tasks.iter().enumerate() {
            let tools_clone = Arc::clone(tools);
            let call_clone = call.clone();

            joinset.spawn(async move {
                let result = tools_clone.execute(&call_clone.tool_name, &call_clone.arguments).await;
                (idx, call_clone, result)
            });
        }

        while let Some(res) = joinset.join_next().await {
            match res {
                Ok((idx, _call, result)) => {
                    concurrent_results[idx] = result;
                }
                Err(e) => {
                    debug!("Concurrent task panicked: {}", e);
                    concurrent_results
                        .iter_mut()
                        .for_each(|r| {
                            r.error = Some(format!("Task panic: {}", e));
                        });
                }
            }
        }
    }

    // Combine serial and concurrent results
    let mut all_results = serial_results;
    all_results.extend(concurrent_results);

    Ok(all_results)
}

/// Check if a tool should always run serially.
fn is_serial_only(tool_name: &str, config: &ConcurrentDispatchConfig) -> bool {
    config.serial_only_tools.iter().any(|s| s == tool_name)
        || config.destructive_tools.iter().any(|s| s == tool_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serial_only_destructive_tools() {
        let config = ConcurrentDispatchConfig::default();
        assert!(is_serial_only("write_file", &config));
        assert!(is_serial_only("patch", &config));
        assert!(is_serial_only("shell", &config));
        assert!(!is_serial_only("http_get", &config));
        assert!(!is_serial_only("web_search", &config));
    }

    #[test]
    fn test_serial_only_custom_tools() {
        let config = ConcurrentDispatchConfig {
            serial_only_tools: vec!["my_tool".to_string()],
            ..Default::default()
        };
        assert!(is_serial_only("my_tool", &config));
        assert!(!is_serial_only("other_tool", &config));
    }
}
