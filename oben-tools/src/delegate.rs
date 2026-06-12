/// Delegate tool — subagent delegation (single + batch mode).
///
/// Maps to `tools/delegate_tool.py`.
///
/// The delegate tool is a special case: it doesn't execute a local command
/// like `read_file` or `terminal`. Instead, it spawns a **new `Agent` instance**
/// with its own `SessionManager` pointing at a child session in the shared database.
///
/// Supports two modes:
/// - **Single**: provide `goal` (+ optional context, role)
/// - **Batch**: provide `tasks` array [{goal, context, role}, ...]
///
/// Architecture:
/// - The delegate tool holds a `SpawnFn` closure
/// - The delegate tool takes `task_name` + `goal` (single) or `tasks` (batch)
/// - Returns subagent result(s) as tool output JSON
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Semaphore;

use crate::registry::{SpawnFn, Tool, ToolRegistry};
use oben_models::{Tool as ToolDef, ToolResult};
use tracing::{debug, info, warn};

/// Truncate a goal/context description for log lines to avoid spam.
fn log_short(s: &str, max: usize) -> String {
    let trimmed = s.trim();
    let trimmed_len = trimmed.chars().count();
    if trimmed_len <= max {
        trimmed.to_string()
    } else {
        let half = max / 2;
        let left: String = trimmed.chars().take(half).collect();
        let right: String = trimmed.chars().skip(trimmed_len - half).collect();
        format!("{}.{}.", left, right)
    }
}

/// A single subagent task in a batch delegation call.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DelegateTask {
    /// The goal the subagent should accomplish.
    pub goal: String,
    /// Optional additional context for the subagent.
    #[serde(default)]
    pub context: Option<String>,
    /// Role: "leaf" (default, cannot delegate further) or "orchestrator".
    #[serde(default)]
    pub role: Option<String>,
}

/// Batch result entry — one per subagent task.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DelegateTaskResult {
    /// 0-indexed task position in the tasks array.
    pub task_index: usize,
    /// Short descriptive label from the goal (truncated to 40 chars).
    pub label: String,
    /// Whether the subagent completed successfully.
    pub status: String,
    /// The child agent's final response (truncated to 500 chars).
    pub summary: String,
    /// Optional error message if the subagent failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Number of API calls the child made.
    pub api_calls: u32,
    /// How long the child ran in seconds.
    pub duration_seconds: f64,
    /// The model the child used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The child's session ID.
    pub session_id: String,
    /// The parent session ID that spawned this child.
    pub parent_session_id: String,
    /// Role: "leaf" or "orchestrator".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Depth of this subagent in the delegate tree.
    pub depth: usize,
}

/// Tool definition for `delegate_task`.
///
/// Supports two modes:
/// - Single: `goal` (+ optional `context`, `role`)
/// - Batch: `tasks` array [{goal, context, role}, ...]
pub fn tool_def() -> ToolDef {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "goal": {
                "type": "string",
                "description": "The goal the subagent should accomplish. Be specific and self-contained."
            },
            "context": {
                "type": "string",
                "description": "Optional additional context for the subagent (file paths, conventions, constraints)."
            },
            "task_name": {
                "type": "string",
                "description": "A short descriptive name for tracking progress."
            },
            "role": {
                "type": "string",
                "enum": ["leaf", "orchestrator"],
                "description": "Role of the child agent: 'leaf' (default, cannot delegate further) or 'orchestrator' (can spawn subagents)."
            },
            "tasks": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "goal": {"type": "string", "description": "Task goal"},
                        "context": {"type": "string", "description": "Task-specific context"},
                        "role": {
                            "type": "string",
                            "enum": ["leaf", "orchestrator"],
                            "description": "Per-task role override. See top-level 'role'."
                        }
                    },
                    "required": ["goal"]
                },
                "description": "Batch mode: tasks to run in parallel. Each gets its own subagent with isolated context."
            }
        },
        "required": []
    });

    ToolDef::builder(
        "delegate_task",
        "Spawn one or more subagents in isolated contexts. Supports two modes:\n\
         1. Single: provide `goal` (+ optional `context`, `role`)\n\
         2. Batch: provide `tasks` array [{goal, context, role}, ...] for parallel execution\n\n\
         Subagents get their own conversation history and session—intermediate tool calls\n\
         never enter your context. Only the final summary is returned.\n\n\
         USE delegate_task when:\n\
         - The task requires multiple reasoning steps or research (debugging, code review)\n\
         - A task would flood your context with intermediate data\n\
         - You have 2+ independent subtasks that can run in parallel (batch mode)\n\n\
         DO NOT use delegate_task when:\n\
         - It's a single mechanical step — do it directly\n\
         - It requires user interaction — subagents cannot call `clarify`\n\
         - The task is trivial or too broad (aim for 2-5 minutes per subagent)\n\
         - Work must outlive the current turn — use cronjob instead\n\n\
         Subagents run SYNCHRONOUSLY inside your turn. You block until they complete.",
    )
    .json_schema(schema)
    .build()
}

/// A delegating tool that spawns a child agent.
///
/// The `DelegateTool` holds:
/// - The spawn function closure
/// - Child and parent session IDs for tracking
pub struct DelegateTool {
    /// The spawn function closure.
    /// Takes (parent_session_id, goal, depth, role) and returns a JoinHandle.
    spawn_fn: SpawnFn,
    /// Session ID for parent (for display/tracking). Filled from args during execute.
    parent_session_id: String,
    /// Maximum number of subagent tasks running concurrently.
    max_concurrent_tasks: usize,
}

impl DelegateTool {
    pub fn new(spawn_fn: SpawnFn, max_concurrent_tasks: usize) -> Box<dyn Tool> {
        Box::new(Self {
            spawn_fn,
            parent_session_id: String::new(),
            max_concurrent_tasks,
        })
    }

    /// Execute a single subagent delegation.
    async fn execute_single(
        &self,
        parent_id: &str,
        goal: &str,
        _context: &str,
        role: &str,
        depth: usize,
    ) -> String {
        info!(
            "delegate: spawn single child parent_session_id={} depth={role} goal={}",
            parent_id, goal
        );
        let start = std::time::Instant::now();
        let join_handle = (self.spawn_fn)(parent_id.to_string(), goal.to_string(), depth, role);

        match join_handle.await {
            Ok(Ok(sr)) => {
                info!(
                    "delegate: single task completed in {:0.2}s task_index={} status={}",
                    start.elapsed().as_secs_f64(),
                    sr.depth,
                    sr.status
                );
                if let Ok(json) = serde_json::to_string_pretty(&sr) {
                    return json;
                }
                warn!(
                    "delegate: single failed to serialize result in {:0.2}s",
                    start.elapsed().as_secs_f64()
                );
                format!("Result serialized: {:?}", sr.status)
            }
            Ok(Err(e)) => {
                warn!(
                    "delegate: single task failed in {:0.2}s child_session_id={}: {}",
                    start.elapsed().as_secs_f64(),
                    "unknown",
                    e
                );
                return format!("Subagent execution failed: {e}");
            }
            Err(e) => {
                warn!(
                    "delegate: single task panicked in {:0.2}s parent_session_id={}: {}",
                    start.elapsed().as_secs_f64(),
                    parent_id,
                    e
                );
                return format!("Subagent task panicked: {e}");
            }
        }
    }

    /// Execute batch subagent delegations concurrently.
    ///
    /// Two-phase approach:
    /// 1. Validate tasks and collect errors for malformed entries
    /// 2. Spawn valid tasks, await all concurrently, merge results
    async fn execute_batch(
        &self,
        parent_id: &str,
        top_level_role: String,
        base_depth: usize,
        tasks: &[Value],
        call_id: &str,
    ) -> ToolResult {
        // Phase 1: validate & extract — separate valid tasks from parsing errors
        struct TaskSpec {
            index: usize,
            goal: String,
            role: String,
        }

        let mut error_results: Vec<DelegateTaskResult> = Vec::new();
        let mut valid_tasks: Vec<TaskSpec> = Vec::new();

        for (i, task_val) in tasks.iter().enumerate() {
            match task_val.as_object() {
                Some(obj) => match obj.get("goal").and_then(|v| v.as_str()) {
                    Some(g) if !g.trim().is_empty() => {
                        let task_role = obj
                            .get("role")
                            .and_then(|v| v.as_str())
                            .unwrap_or(&top_level_role);
                        debug!(
                            "delegate: batch task[{i}] valid goal={}, role={}",
                            log_short(g, 100),
                            task_role
                        );
                        valid_tasks.push(TaskSpec {
                            index: i,
                            goal: g.to_string(),
                            role: task_role.to_string(),
                        });
                    }
                    _ => {
                        let task_num = i + 1;
                        warn!("delegate: batch task[{i}] missing or empty goal");
                        error_results.push(DelegateTaskResult {
                            task_index: i,
                            label: format!("Task {task_num}"),
                            status: "error".into(),
                            summary: String::new(),
                            error: Some("Task is missing or empty 'goal'".into()),
                            api_calls: 0,
                            duration_seconds: 0.0,
                            model: None,
                            session_id: String::new(),
                            parent_session_id: parent_id.to_string(),
                            role: Some("leaf".into()),
                            depth: base_depth + 1,
                        });
                    }
                },
                None => {
                    let task_num = i + 1;
                    error_results.push(DelegateTaskResult {
                        task_index: i,
                        label: format!("Task {task_num}"),
                        status: "error".into(),
                        summary: String::new(),
                        error: Some("Task entry is not an object".into()),
                        api_calls: 0,
                        duration_seconds: 0.0,
                        model: None,
                        session_id: String::new(),
                        parent_session_id: parent_id.to_string(),
                        role: Some("leaf".into()),
                        depth: base_depth + 1,
                    });
                }
            }
        }

        // Phase 2: spawn valid tasks concurrently with concurrency limit
        let mut handles: Vec<(usize, tokio::task::JoinHandle<DelegateTaskResult>)> =
            Vec::with_capacity(valid_tasks.len());

        info!(
            "delegate: batch spawning {} valid tasks, {} parse errors, max_concurrent={}",
            valid_tasks.len(),
            error_results.len(),
            self.max_concurrent_tasks
        );

        let semaphore = Arc::new(Semaphore::new(self.max_concurrent_tasks));

        for spec in valid_tasks {
            let spawn_fn = Arc::clone(&self.spawn_fn);
            let parent_id = parent_id.to_string();
            let task_label = spec.goal.chars().take(40).collect::<String>();
            let task_role = spec.role.clone();
            let depth = base_depth + 1;
            let goal = spec.goal.clone();
            let idx = spec.index;
            let permit = semaphore.clone();

            info!(
                "delegate: batch spawning task[{idx}] label={label} role={role} depth={depth}",
                label = task_label,
                role = task_role
            );
            let goal_short = log_short(&goal, 80);
            debug!("delegate: batch task[{idx}] goal={}", goal_short);

            let handle = tokio::spawn(async move {
                let _permit = permit.acquire().await.unwrap();
                debug!("delegate: batch task[{idx}] spawned label={}", task_label);
                let join_handle = (spawn_fn)(parent_id.clone(), goal, depth, &task_role);

                match join_handle.await {
                    Ok(Ok(sr)) => DelegateTaskResult {
                        task_index: idx,
                        label: task_label,
                        status: sr.status.clone(),
                        summary: sr.summary,
                        error: None,
                        api_calls: sr.api_calls,
                        duration_seconds: sr.duration_seconds,
                        model: sr.model,
                        session_id: sr.session_id,
                        parent_session_id: parent_id,
                        role: Some(task_role),
                        depth,
                    },
                    Ok(Err(e)) => DelegateTaskResult {
                        task_index: idx,
                        label: task_label,
                        status: "error".into(),
                        summary: String::new(),
                        error: Some(format!("SubagentError: {e}")),
                        api_calls: 0,
                        duration_seconds: 0.0,
                        model: None,
                        session_id: String::new(),
                        parent_session_id: parent_id,
                        role: Some(task_role),
                        depth,
                    },
                    Err(e) => DelegateTaskResult {
                        task_index: idx,
                        label: task_label,
                        status: "error".into(),
                        summary: String::new(),
                        error: Some(format!("Task panicked: {e}")),
                        api_calls: 0,
                        duration_seconds: 0.0,
                        model: None,
                        session_id: String::new(),
                        parent_session_id: parent_id,
                        role: Some(task_role),
                        depth,
                    },
                }
            });
            handles.push((idx, handle));
        }

        // Collect spawned results
        for (_idx, handle) in handles {
            match handle.await {
                Ok(result) => error_results.push(result),
                Err(e) => {
                    let task_num = _idx + 1;
                    error_results.push(DelegateTaskResult {
                        task_index: _idx,
                        label: format!("Task {task_num}"),
                        status: "error".into(),
                        summary: String::new(),
                        error: Some(format!("JoinError: {e}")),
                        api_calls: 0,
                        duration_seconds: 0.0,
                        model: None,
                        session_id: String::new(),
                        parent_session_id: parent_id.to_string(),
                        role: None,
                        depth: base_depth + 1,
                    });
                }
            }
        }

        // Sort by task_index for deterministic output
        error_results.sort_by_key(|r| r.task_index);

        let output = match serde_json::to_string_pretty(&error_results) {
            Ok(json) => json,
            Err(e) => {
                return ToolResult {
                    call_id: call_id.to_string(),
                    output: format!("Failed to serialize results: {e}"),
                    error: Some(format!("SerializationError: {e}")),
                };
            }
        };

        ToolResult {
            call_id: call_id.to_string(),
            output,
            error: None,
        }
    }
}

#[async_trait::async_trait]
impl Tool for DelegateTool {
    fn name(&self) -> &str {
        "delegate_task"
    }

    fn description(&self) -> &str {
        "Delegate a task to a subagent. The subagent gets its own session, \
         fresh context, and can use tools to accomplish the goal. Returns \
         a summary of the result."
    }

    /// Validate args: must have either `goal` (single) or `tasks` (batch).
    fn validate(&self, args: &Value) -> Result<(), anyhow::Error> {
        let has_goal = args
            .get("goal")
            .and_then(|v| v.as_str())
            .map(|s| !s.is_empty())
            .unwrap_or(false);
        let has_tasks = args
            .get("tasks")
            .and_then(|v| v.as_array())
            .map(|a| !a.is_empty())
            .unwrap_or(false);

        debug!(
            "delegate: validate has_goal={} has_tasks={}",
            has_goal, has_tasks
        );
        if has_goal {
            if let Some(g) = args.get("goal").and_then(|v| v.as_str()) {
                debug!(
                    "delegate: validate goal={} context={} role={}",
                    g,
                    args.get("context").and_then(|v| v.as_str()).unwrap_or(""),
                    args.get("role").and_then(|v| v.as_str()).unwrap_or("")
                );
            }
        }
        if has_tasks {
            if let Some(tasks) = args.get("tasks").and_then(|v| v.as_array()) {
                debug!("delegate: validate {} tasks to process", tasks.len());
            }
        }

        if !has_goal && !has_tasks {
            return Err(anyhow::anyhow!(
                "Provide either 'goal' (single task) or 'tasks' (batch array)"
            ));
        }

        if has_tasks {
            let tasks = args
                .get("tasks")
                .and_then(|v| v.as_array())
                .ok_or_else(|| anyhow::anyhow!("tasks must be a JSON array of task objects"))?;

            for (i, task) in tasks.iter().enumerate() {
                let task_obj = task
                    .as_object()
                    .ok_or_else(|| anyhow::anyhow!("tasks[{i}] must be an object"))?;
                let goal = task_obj
                    .get("goal")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("tasks[{i}] is missing 'goal'"))?;
                if goal.trim().is_empty() {
                    return Err(anyhow::anyhow!("tasks[{i}] 'goal' must not be empty"));
                }
            }
        }

        Ok(())
    }

    /// Execute the delegate: spawn child agent(s) and await their results.
    ///
    /// Supports two modes:
    /// - **Single**: one subagent spawned for the `goal`
    /// - **Batch**: multiple subagents spawned concurrently for each `tasks` entry
    async fn execute(&self, args: &Value) -> ToolResult {
        let parent_id = args
            .get("parent_session_id")
            .and_then(|v| v.as_str())
            .unwrap_or(&self.parent_session_id)
            .to_string();
        let top_level_role = args
            .get("role")
            .and_then(|v| v.as_str())
            .unwrap_or("leaf")
            .to_string();
        let base_depth = args
            .get("agent_depth")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        let call_id = args
            .get("call_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if let Some(n) = args.get("tasks").and_then(|v| v.as_array()) {
            info!(
                "delegate: batch mode START, {} tasks, parent_session_id={} role={} depth={}",
                n.len(),
                parent_id,
                top_level_role,
                base_depth
            );
            self.execute_batch(&parent_id, top_level_role, base_depth, n, &call_id)
                .await
        } else {
            let goal = args
                .get("goal")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_default();
            let context = args.get("context").and_then(|v| v.as_str()).unwrap_or("");
            let goal_short = log_short(&goal, 100);
            info!(
                "delegate: single mode START, parent_session_id={} goal={} role={} depth={}",
                parent_id, goal_short, top_level_role, base_depth
            );
            let result = self
                .execute_single(&parent_id, &goal, context, &top_level_role, base_depth)
                .await;

            let _output_short = log_short(&result, 200);
            info!(
                "delegate: single mode COMPLETE, parent_session_id={} result_len={} goal={}",
                parent_id,
                result.len(),
                goal_short
            );

            ToolResult {
                call_id,
                output: result,
                error: None,
            }
        }
    }

    fn clone_tool(&self) -> Box<dyn Tool> {
        Box::new(Self {
            spawn_fn: Arc::clone(&self.spawn_fn),
            parent_session_id: self.parent_session_id.clone(),
            max_concurrent_tasks: self.max_concurrent_tasks,
        })
    }
}

/// Register the delegate tool with the registry.
///
/// The `delegate_task` tool must be registered AFTER the parent agent's
/// `SpawnFn` has been set up, so it has the proper closure to spawn children.
pub fn register(registry: &mut ToolRegistry) {
    // Placeholder registration — the real handler is set by the parent tool.
    let spawn_fn: SpawnFn = Arc::new(|_parent_id, _goal, _depth, _role| {
        tokio::spawn(async { Err(anyhow::anyhow!("delegate_task not yet configured")) })
    });
    let tool = DelegateTool::new(spawn_fn, 3);
    registry.register_with_def(tool, tool_def());
}

// ── Unit tests ─────────────────────────────────────────────────────────

/// Helper: build a SpawnFn that returns a pre-constructed SubagentResult.
fn mock_spawn_fn(sr: crate::registry::SubagentResult) -> SpawnFn {
    Arc::new(move |_pid, _goal, _depth, _role| {
        let sr = sr.clone();
        tokio::spawn(async move { Ok(sr) })
    })
}

/// Helper: build a SpawnFn that returns an error.
fn error_spawn_fn(msg: String) -> SpawnFn {
    Arc::new(move |_pid, _goal, _depth, _role| {
        let msg = msg.clone();
        tokio::spawn(async move { Err(anyhow::anyhow!("{}", msg)) })
    })
}

/// Given: Valid single-task args with goal
/// When: DelegateTool.validate is called
/// Then: Returns Ok(())
#[tokio::test]
async fn test_validate_single_valid() {
    let spawn = mock_spawn_fn(crate::registry::SubagentResult {
        status: "completed".into(),
        ..Default::default()
    });
    let tool = DelegateTool::new(spawn, 3);
    let args = serde_json::json!({"goal": "research schema"});
    assert!(tool.validate(&args).is_ok());
}

/// Given: Valid batch args with tasks array
/// When: DelegateTool.validate is called
/// Then: Returns Ok(())
#[tokio::test]
async fn test_validate_batch_valid() {
    let spawn = mock_spawn_fn(crate::registry::SubagentResult {
        status: "completed".into(),
        ..Default::default()
    });
    let tool = DelegateTool::new(spawn, 3);
    let args = serde_json::json!({
        "tasks": [
            {"goal": "task 1"},
            {"goal": "task 2", "role": "orchestrator"}
        ]
    });
    assert!(tool.validate(&args).is_ok());
}

/// Given: Args with neither goal nor tasks
/// When: DelegateTool.validate is called
/// Then: Returns Err with descriptive message
#[tokio::test]
async fn test_validate_neither_goal_nor_tasks() {
    let spawn = mock_spawn_fn(crate::registry::SubagentResult {
        status: "completed".into(),
        ..Default::default()
    });
    let tool = DelegateTool::new(spawn, 3);
    let args = serde_json::json!({"context": "extra info"});
    let result = tool.validate(&args);
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Provide either 'goal'"));
}

/// Given: Batch with task missing goal
/// When: DelegateTool.validate is called
/// Then: Returns Err about missing goal
#[tokio::test]
async fn test_validate_batch_missing_goal() {
    let spawn = mock_spawn_fn(crate::registry::SubagentResult {
        status: "completed".into(),
        ..Default::default()
    });
    let tool = DelegateTool::new(spawn, 3);
    let args = serde_json::json!({"tasks": [{"context": "no goal here"}]});
    let result = tool.validate(&args);
    assert!(result.is_err());
}

/// Given: Batch with task having empty goal
/// When: DelegateTool.validate is called
/// Then: Returns Err about empty goal
#[tokio::test]
async fn test_validate_batch_empty_goal() {
    let spawn = mock_spawn_fn(crate::registry::SubagentResult {
        status: "completed".into(),
        ..Default::default()
    });
    let tool = DelegateTool::new(spawn, 3);
    let args = serde_json::json!({"tasks": [{"goal": ""}]});
    let result = tool.validate(&args);
    assert!(result.is_err());
}

/// Given: Batch with non-object task entry
/// When: DelegateTool.validate is called
/// Then: Returns Err about task not being an object
#[tokio::test]
async fn test_validate_batch_non_object_task() {
    let spawn = mock_spawn_fn(crate::registry::SubagentResult {
        status: "completed".into(),
        ..Default::default()
    });
    let tool = DelegateTool::new(spawn, 3);
    let args = serde_json::json!({"tasks": ["not-an-object"]});
    let result = tool.validate(&args);
    assert!(result.is_err());
}

/// Given: Valid single-task args
/// When: DelegateTool.execute is called with goal
/// Then: Returns ToolResult with JSON-serialized SubagentResult
#[tokio::test]
async fn test_execute_single_returns_result() {
    let expect_sr = crate::registry::SubagentResult {
        status: "completed".into(),
        summary: "child done".into(),
        api_calls: 42,
        duration_seconds: 5.5,
        model: Some("mock/1".into()),
        session_id: "child-1".into(),
        parent_session_id: "parent-1".into(),
        role: Some("leaf".into()),
        depth: 1,
        ..Default::default()
    };
    let spawn = mock_spawn_fn(expect_sr.clone());
    let tool = DelegateTool::new(spawn, 3);
    let args = serde_json::json!({
        "goal": "research schema",
        "parent_session_id": "parent-1",
        "call_id": "call-1"
    });
    let result = tool.execute(&args).await;

    assert!(result.error.is_none());
    let parsed: crate::registry::SubagentResult =
        serde_json::from_str(&result.output).expect("output is valid JSON");
    assert_eq!(parsed.status, "completed");
    assert_eq!(parsed.summary, "child done");
    assert_eq!(parsed.api_calls, 42);
    assert!(!parsed.session_id.is_empty());
}

/// Given: Single task with context and optional fields
/// When: DelegateTool.execute is called
/// Then: SubagentResult is serialized with depth tracking
#[tokio::test]
async fn test_execute_single_with_context() {
    let spawn = mock_spawn_fn(crate::registry::SubagentResult {
        status: "completed".into(),
        summary: "summary".into(),
        session_id: "child-2".into(),
        parent_session_id: "parent-2".into(),
        role: Some("orchestrator".into()),
        depth: 1,
        ..Default::default()
    });
    let tool = DelegateTool::new(spawn, 3);
    let args = serde_json::json!({
        "goal": "benchmark",
        "context": "use postgres",
        "role": "orchestrator",
        "agent_depth": 0,
        "parent_session_id": "parent-2",
        "call_id": "call-2"
    });
    let result = tool.execute(&args).await;

    assert!(result.error.is_none());
    let parsed: crate::registry::SubagentResult =
        serde_json::from_str(&result.output).expect("output is valid JSON");
    assert_eq!(parsed.role, Some("orchestrator".into()));
    assert_eq!(parsed.depth, 1);
}

/// Given: Batch with 2 valid tasks
/// When: DelegateTool.execute is called
/// Then: Returns ToolResult with array of 2 DelegateTaskResult
#[tokio::test]
async fn test_execute_batch_two_tasks() {
    let spawn = mock_spawn_fn(crate::registry::SubagentResult {
        status: "completed".into(),
        summary: "batch result".into(),
        session_id: "batch-task-00".into(),
        role: Some("leaf".into()),
        depth: 1,
        ..Default::default()
    });
    let tool = DelegateTool::new(spawn, 3);
    let args = serde_json::json!({
        "tasks": [
            {"goal": "task A"},
            {"goal": "task B", "role": "orchestrator"}
        ],
        "parent_session_id": "parent-batch",
        "call_id": "call-batch",
        "agent_depth": 0,
        "role": "leaf"
    });
    let result = tool.execute(&args).await;

    assert!(result.error.is_none());
    let parsed: Vec<DelegateTaskResult> =
        serde_json::from_str(&result.output).expect("output is valid JSON array");
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0].task_index, 0);
    assert_eq!(parsed[0].label, "task A");
    assert_eq!(parsed[0].status, "completed");
    assert_eq!(parsed[1].task_index, 1);
    assert_eq!(parsed[1].label, "task B");
    assert_eq!(parsed[1].role, Some("orchestrator".into()));
    assert_eq!(parsed[1].depth, 1);
}

/// Given: Batch with 1 valid + 1 invalid task
/// When: DelegateTool.execute is called
/// Then: Returns 2 results — one success, one error for missing goal
#[tokio::test]
async fn test_execute_batch_mixed_valid_invalid() {
    let spawn = mock_spawn_fn(crate::registry::SubagentResult {
        status: "completed".into(),
        summary: "ok".into(),
        session_id: "batch-task-00".into(),
        role: Some("leaf".into()),
        depth: 1,
        ..Default::default()
    });
    let tool = DelegateTool::new(spawn, 3);
    let args = serde_json::json!({
        "tasks": [
            {"goal": "valid task"},
            {"context": "no goal"}
        ],
        "parent_session_id": "parent-mixed",
        "call_id": "call-mixed",
        "agent_depth": 0
    });
    let result = tool.execute(&args).await;

    assert!(result.error.is_none());
    let parsed: Vec<DelegateTaskResult> =
        serde_json::from_str(&result.output).expect("output is valid JSON array");
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0].status, "completed");
    assert_eq!(parsed[1].status, "error");
    assert!(parsed[1]
        .error
        .as_ref()
        .unwrap()
        .contains("missing or empty 'goal'"));
}

/// Given: Batch with tasks that have non-object entries
/// When: DelegateTool.execute is called
/// Then: All tasks produce error entries
#[tokio::test]
async fn test_execute_batch_non_object_entries() {
    let tool = DelegateTool::new(
        mock_spawn_fn(crate::registry::SubagentResult {
            status: "completed".into(),
            session_id: "skip".into(),
            ..Default::default()
        }),
        3,
    );
    let args = serde_json::json!({
        "tasks": ["string-value", 42, null],
        "parent_session_id": "parent",
        "call_id": "call-id",
        "agent_depth": 0
    });
    let result = tool.execute(&args).await;

    let parsed: Vec<DelegateTaskResult> =
        serde_json::from_str(&result.output).expect("output is valid JSON array");
    assert_eq!(parsed.len(), 3);
    for r in &parsed {
        assert_eq!(r.status, "error");
        assert!(r.error.as_ref().unwrap().contains("not an object"));
    }
}

/// Given: Empty tasks array
/// When: DelegateTool.validate is called
/// Then: Returns Err about providing goal or tasks
#[tokio::test]
async fn test_validate_empty_tasks_array() {
    let spawn = mock_spawn_fn(crate::registry::SubagentResult {
        status: "completed".into(),
        ..Default::default()
    });
    let tool = DelegateTool::new(spawn, 3);
    let args = serde_json::json!({"tasks": []});
    let result = tool.validate(&args);
    assert!(result.is_err());
}

/// Given: tool_def() definition
/// When: Schema is checked
/// Then: It includes both goal and tasks parameters
#[test]
fn test_tool_def_has_tasks_param() {
    let def = tool_def();
    match &def.parameters {
        oben_models::ToolParameters::JsonSchema { schema } => {
            let props = schema.get("properties").unwrap().as_object().unwrap();
            assert!(props.contains_key("goal"));
            assert!(props.contains_key("tasks"));
            assert!(props.contains_key("context"));
            assert!(props.contains_key("role"));
        }
        _ => panic!("expected json schema"),
    }
}

/// Given: tool_def() description
/// When: Description is inspected
/// Then: It mentions both single and batch modes
#[test]
fn test_tool_def_mentions_batch_mode() {
    let def = tool_def();
    assert!(def.description.contains("Batch"));
    assert!(def.description.contains("goal"));
}
