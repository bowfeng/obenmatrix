use std::sync::Arc;

use oben_agent::compact::CompactCofig;
use oben_agent::delegate::SubagentSpawner;
use oben_agent::hooks::HookBuilder;
use oben_models::{CallMode, Message, ToolParameters, TransportProvider, TransportResponse};
use oben_tools::registry::{SpawnFn, SubagentResult, ToolRegistry};

/// ── Mock Transport ─────────────────────────────────────────────────────
#[derive(Debug)]
struct MockTransport {
    _responses: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

impl MockTransport {
    fn new() -> Self {
        Self {
            _responses: Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }

    fn response_count(&self) -> usize {
        self._responses.lock().unwrap().len()
    }
}

#[async_trait::async_trait]
impl TransportProvider for MockTransport {
    fn name(&self) -> &str {
        "mock"
    }

    async fn chat(
        &self,
        _messages: &[Message],
        _mode: &CallMode,
    ) -> Result<TransportResponse, anyhow::Error> {
        // Return a simple text response that doesn't trigger tool calls
        Ok(TransportResponse {
            text: "I have completed the task.".to_string(),
            tool_calls: vec![],
            tokens_used: Some(15),
        })
    }

    async fn stream_chat(
        &self,
        _messages: &[Message],
        _mode: &CallMode,
        _callback: oben_models::StreamDeltaCallback,
    ) -> Result<TransportResponse, anyhow::Error> {
        self.chat(_messages, _mode).await
    }
}

/// ── Helpers ────────────────────────────────────────────────────────────

fn make_spawner(transport: Arc<MockTransport>) -> (SubagentSpawner, Arc<ToolRegistry>) {
    let tools = Arc::new(ToolRegistry::new());
    let hooks = Arc::new(HookBuilder::new().build());

    // SubagentSpawner needs Arc<dyn TransportProvider> — MockTransport is Send + Sync
    let spawner = SubagentSpawner::new(
        transport as Arc<dyn TransportProvider + Send + Sync>,
        tools.clone(),
        CompactCofig {
            context_length: 128_000,
            threshold_percent: 0.75,
            ..Default::default()
        },
        5,   // max_iterations
        100, // max_messages
        3,   // max_spawn_depth
        hooks,
    );

    (spawner, tools)
}

/// ── Unit Tests ─────────────────────────────────────────────────────────

/// Given: Valid single-task args with goal
/// When: DelegateTool.validate is called
/// Then: Returns Ok(())
#[tokio::test]
async fn test_validate_single_valid() {
    let sr = SubagentResult {
        status: "completed".into(),
        ..Default::default()
    };
    let spawn: SpawnFn = Arc::new(move |_, _, _, _| {
        let sr = sr.clone();
        tokio::spawn(async move { Ok::<_, anyhow::Error>(sr) })
    });
    let tool = oben_tools::delegate::DelegateTool::new(spawn, 5);
    let args = serde_json::json!({"goal": "research schema"});
    assert!(let call = oben_tools::registry::ToolCall::new("delegate_task", &args);
    tool.validate(&call).is_ok());
}

/// Given: Valid batch args with tasks array
/// When: DelegateTool.validate is called
/// Then: Returns Ok(())
#[tokio::test]
async fn test_validate_batch_valid() {
    let sr = SubagentResult {
        status: "completed".into(),
        ..Default::default()
    };
    let spawn: SpawnFn = Arc::new(move |_, _, _, _| {
        let sr = sr.clone();
        tokio::spawn(async move { Ok::<_, anyhow::Error>(sr) })
    });
    let tool = oben_tools::delegate::DelegateTool::new(spawn, 5);
    let args = serde_json::json!({
        "tasks": [
            {"goal": "task 1"},
            {"goal": "task 2", "role": "orchestrator"}
        ]
    });
    assert!(let call = oben_tools::registry::ToolCall::new("delegate_task", &args);
    tool.validate(&call).is_ok());
}

/// Given: Args with neither goal nor tasks
/// When: DelegateTool.validate is called
/// Then: Returns Err with descriptive message
#[tokio::test]
async fn test_validate_neither_goal_nor_tasks() {
    let sr = SubagentResult {
        status: "completed".into(),
        ..Default::default()
    };
    let spawn: SpawnFn = Arc::new(move |_, _, _, _| {
        let sr = sr.clone();
        tokio::spawn(async move { Ok::<_, anyhow::Error>(sr) })
    });
    let tool = oben_tools::delegate::DelegateTool::new(spawn, 5);
    let args = serde_json::json!({"context": "extra info"});
    let result = let call = oben_tools::registry::ToolCall::new("delegate_task", &args);
    tool.validate(&call);
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
    let sr = SubagentResult {
        status: "completed".into(),
        ..Default::default()
    };
    let spawn: SpawnFn = Arc::new(move |_, _, _, _| {
        let sr = sr.clone();
        tokio::spawn(async move { Ok::<_, anyhow::Error>(sr) })
    });
    let tool = oben_tools::delegate::DelegateTool::new(spawn, 5);
    let args = serde_json::json!({"tasks": [{"context": "no goal here"}]});
    assert!(let call = oben_tools::registry::ToolCall::new("delegate_task", &args);
    tool.validate(&call).is_err());
}

/// Given: Batch with task having empty goal
/// When: DelegateTool.validate is called
/// Then: Returns Err about empty goal
#[tokio::test]
async fn test_validate_batch_empty_goal() {
    let sr = SubagentResult {
        status: "completed".into(),
        ..Default::default()
    };
    let spawn: SpawnFn = Arc::new(move |_, _, _, _| {
        let sr = sr.clone();
        tokio::spawn(async move { Ok::<_, anyhow::Error>(sr) })
    });
    let tool = oben_tools::delegate::DelegateTool::new(spawn, 5);
    let args = serde_json::json!({"tasks": [{"goal": ""}]});
    assert!(let call = oben_tools::registry::ToolCall::new("delegate_task", &args);
    tool.validate(&call).is_err());
}

/// Given: Batch with non-object task entry
/// When: DelegateTool.validate is called
/// Then: Returns Err about task not being an object
#[tokio::test]
async fn test_validate_batch_non_object_task() {
    let sr = SubagentResult {
        status: "completed".into(),
        ..Default::default()
    };
    let spawn: SpawnFn = Arc::new(move |_, _, _, _| {
        let sr = sr.clone();
        tokio::spawn(async move { Ok::<_, anyhow::Error>(sr) })
    });
    let tool = oben_tools::delegate::DelegateTool::new(spawn, 5);
    let args = serde_json::json!({"tasks": ["not-an-object"]});
    assert!(let call = oben_tools::registry::ToolCall::new("delegate_task", &args);
    tool.validate(&call).is_err());
}

/// Given: Valid single-task args
/// When: DelegateTool.execute is called with goal
/// Then: Returns ToolResult with JSON-serialized SubagentResult
#[tokio::test]
async fn test_execute_single_returns_result() {
    let expect_sr = SubagentResult {
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
    let spawn: SpawnFn = Arc::new(move |_, _, _, _| {
        let sr = expect_sr.clone();
        tokio::spawn(async move { Ok::<_, anyhow::Error>(sr) })
    });
    let tool = oben_tools::delegate::DelegateTool::new(spawn, 5);
    let args = serde_json::json!({
        "goal": "research schema",
        "parent_session_id": "parent-1",
        "call_id": "call-1"
    });
    let call = oben_tools::registry::ToolCall::new("delegate_task", &args);
    let result = tool.execute(&call).await;

    assert!(result.error.is_none());
    let parsed: SubagentResult =
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
    let spawn: SpawnFn = Arc::new(move |_p, _g, d, r| {
        let sr = SubagentResult {
            status: "completed".into(),
            summary: "summary".into(),
            session_id: "child-2".into(),
            parent_session_id: "parent-2".into(),
            role: Some(r.to_string()),
            depth: d,
            ..Default::default()
        };
        tokio::spawn(async move { Ok::<_, anyhow::Error>(sr) })
    });
    let tool = oben_tools::delegate::DelegateTool::new(spawn, 5);
    let args = serde_json::json!({
        "goal": "benchmark",
        "context": "use postgres",
        "role": "orchestrator",
        "agent_depth": 0,
        "parent_session_id": "parent-2",
        "call_id": "call-2"
    });
    let call = oben_tools::registry::ToolCall::new("delegate_task", &args);
    let result = tool.execute(&call).await;

    assert!(result.error.is_none());
    let parsed: SubagentResult =
        serde_json::from_str(&result.output).expect("output is valid JSON");
    assert_eq!(parsed.role, Some("orchestrator".into()));
    assert_eq!(parsed.depth, 0);
}

/// Given: Batch with 2 valid tasks
/// When: DelegateTool.execute is called
/// Then: Returns ToolResult with array of 2 DelegateTaskResult
#[tokio::test]
async fn test_execute_batch_two_tasks() {
    let spawn: SpawnFn = Arc::new(move |_pid, _goal, depth, role| {
        let sr = SubagentResult {
            status: "completed".into(),
            summary: "batch result".into(),
            session_id: "batch-task".into(),
            parent_session_id: _pid,
            role: Some(role.to_string()),
            depth,
            ..Default::default()
        };
        tokio::spawn(async move { Ok::<_, anyhow::Error>(sr) })
    });
    let tool = oben_tools::delegate::DelegateTool::new(spawn, 5);
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
    let call = oben_tools::registry::ToolCall::new("delegate_task", &args);
    let result = tool.execute(&call).await;

    assert!(result.error.is_none());
    let parsed: Vec<oben_tools::delegate::DelegateTaskResult> =
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
    let spawn: SpawnFn = Arc::new(move |_pid, _goal, depth, role| {
        let sr = SubagentResult {
            status: "completed".into(),
            summary: "ok".into(),
            session_id: "child-session".into(),
            parent_session_id: _pid,
            role: Some(role.to_string()),
            depth,
            ..Default::default()
        };
        tokio::spawn(async move { Ok::<_, anyhow::Error>(sr) })
    });
    let tool = oben_tools::delegate::DelegateTool::new(spawn, 5);
    let args = serde_json::json!({
        "tasks": [
            {"goal": "valid task"},
            {"context": "no goal"}
        ],
        "parent_session_id": "parent-mixed",
        "call_id": "call-mixed",
        "agent_depth": 0
    });
    let call = oben_tools::registry::ToolCall::new("delegate_task", &args);
    let result = tool.execute(&call).await;

    assert!(result.error.is_none());
    let parsed: Vec<oben_tools::delegate::DelegateTaskResult> =
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
    let tool = oben_tools::delegate::DelegateTool::new(
        Arc::new(move |_pid, _goal, depth, role| {
            let sr = SubagentResult {
                status: "completed".into(),
                summary: "skip".into(),
                session_id: "skipped".into(),
                parent_session_id: _pid,
                role: Some(role.to_string()),
                depth,
                ..Default::default()
            };
            tokio::spawn(async move { Ok::<_, anyhow::Error>(sr) })
        }),
        5,
    );
    let args = serde_json::json!({
        "tasks": ["string-value", 42, null],
        "parent_session_id": "parent",
        "call_id": "call-id",
        "agent_depth": 0
    });
    let call = oben_tools::registry::ToolCall::new("delegate_task", &args);
    let result = tool.execute(&call).await;

    let parsed: Vec<oben_tools::delegate::DelegateTaskResult> =
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
    let spawn: SpawnFn = Arc::new(move |_p, _g, _d, _r| {
        tokio::spawn(async move {
            Ok(SubagentResult {
                status: "completed".into(),
                ..Default::default()
            })
        })
    });
    let tool = oben_tools::delegate::DelegateTool::new(spawn, 5);
    let args = serde_json::json!({"tasks": []});
    assert!(let call = oben_tools::registry::ToolCall::new("delegate_task", &args);
    tool.validate(&call).is_err());
}

/// Given: tool_def() definition
/// When: Schema is checked
/// Then: It includes both goal and tasks parameters
#[test]
fn test_tool_def_has_tasks_param() {
    let def = oben_tools::delegate::tool_def();
    match &def.parameters {
        ToolParameters::JsonSchema { schema } => {
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
    let def = oben_tools::delegate::tool_def();
    assert!(def.description.contains("Batch"));
    assert!(def.description.contains("goal"));
}
/// ── Integration Tests ─────────────────────────────────────────────────

/// Given: A SubagentSpawner with a MockTransport

/// Given: A SpawnFn called via integration pattern
/// When: spawn_fn is called through the delegate tool
/// Then: SubagentResult returned with completed status (mock)
#[tokio::test]
async fn test_spawn_child_agent_returns_completed() {
    let spawn: SpawnFn = Arc::new(move |_pid, _goal, depth, role| {
        let sr = SubagentResult {
            status: "completed".into(),
            summary: "result".into(),
            session_id: "child-1".into(),
            parent_session_id: _pid,
            role: Some(role.to_string()),
            depth,
            ..Default::default()
        };
        tokio::spawn(async move { Ok::<_, anyhow::Error>(sr) })
    });
    let tool = oben_tools::delegate::DelegateTool::new(spawn, 5);
    let args = serde_json::json!({
        "goal": "research schema",
        "parent_session_id": "parent-1",
        "call_id": "call-1",
        "agent_depth": 0,
        "role": "leaf",
    });
    let call = oben_tools::registry::ToolCall::new("delegate_task", &args);

    let result = tool.validate(&call);
    assert!(result.is_ok());

    let output = tool.execute(&call).await;
    assert!(output.error.is_none());
    assert!(output.output.contains("completed"));
}

/// Given: A SpawnFn called with depth > 0
/// When: Child agent completes its turn
/// Then: Result reflects correct depth
#[tokio::test]
async fn test_spawn_child_depth_increment() {
    let spawn: SpawnFn = Arc::new(move |_pid, _goal, depth, _role| {
        let sr = SubagentResult {
            status: "completed".into(),
            depth,
            ..Default::default()
        };
        tokio::spawn(async move { Ok::<_, anyhow::Error>(sr) })
    });
    let tool = oben_tools::delegate::DelegateTool::new(spawn, 5);
    let args = serde_json::json!({
        "goal": "nested task",
        "parent_session_id": "parent-1",
        "call_id": "call-1",
        "agent_depth": 2,
        "role": "leaf",
    });

    let call = oben_tools::registry::ToolCall::new("delegate_task", &args);
    let output = tool.execute(&call).await;
    assert!(output.error.is_none());
    assert!(output.output.contains("2"));
}

/// Given: A SpawnFn with orchestrator role at depth < max
/// When: Child agent completes
/// Then: Role is recorded correctly
#[tokio::test]
async fn test_spawn_child_orchestrator_role() {
    let spawn: SpawnFn = Arc::new(move |_pid, _goal, _depth, role| {
        let sr = SubagentResult {
            status: "completed".into(),
            role: Some(role.to_string()),
            ..Default::default()
        };
        tokio::spawn(async move { Ok::<_, anyhow::Error>(sr) })
    });
    let tool = oben_tools::delegate::DelegateTool::new(spawn, 5);
    let args = serde_json::json!({
        "goal": "coordinate subtasks",
        "parent_session_id": "parent-1",
        "call_id": "call-1",
        "agent_depth": 0,
        "role": "orchestrator",
    });

    let call = oben_tools::registry::ToolCall::new("delegate_task", &args);
    let output = tool.execute(&call).await;
    assert!(output.output.contains("orchestrator"));
}

/// Given: A SpawnFn that produces a summary
/// When: Child agent completes
/// Then: Summary is non-empty
#[tokio::test]
async fn test_spawn_child_has_summary() {
    let spawn: SpawnFn = Arc::new(move |_pid, _goal, _depth, _role| {
        let sr = SubagentResult {
            status: "completed".into(),
            summary: "this is a detailed summary of the work done".into(),
            ..Default::default()
        };
        tokio::spawn(async move { Ok::<_, anyhow::Error>(sr) })
    });
    let tool = oben_tools::delegate::DelegateTool::new(spawn, 5);
    let args = serde_json::json!({
        "goal": "analyze the data",
        "parent_session_id": "parent-1",
        "call_id": "call-1",
        "agent_depth": 0,
        "role": "leaf",
    });

    let call = oben_tools::registry::ToolCall::new("delegate_task", &args);
    let output = tool.execute(&call).await;
    assert!(output.output.contains("detailed summary"));
}

/// Given: A SubagentSpawner at max depth limit
/// When: Child is spawned at depth 2 (max is 3)
/// Then: Child is spawned and completed
#[tokio::test]
async fn test_max_spawn_depth_respected() {
    let spawn: SpawnFn = Arc::new(move |_pid, _goal, depth, _role| {
        let sr = SubagentResult {
            status: "completed".into(),
            depth,
            ..Default::default()
        };
        tokio::spawn(async move { Ok::<_, anyhow::Error>(sr) })
    });
    let tool = oben_tools::delegate::DelegateTool::new(spawn, 5);
    let args = serde_json::json!({
        "goal": "deep task",
        "parent_session_id": "parent-1",
        "call_id": "call-1",
        "agent_depth": 2,
        "role": "leaf",
    });

    let call = oben_tools::registry::ToolCall::new("delegate_task", &args);
    let output = tool.execute(&call).await;
    assert!(output.output.contains("completed"));
}

/// Given: Multiple subagents are spawned concurrently
/// When: 3 spawn_fn calls are made on a mock
/// Then: All complete and return results
#[tokio::test]
async fn test_spawn_multiple_children_concurrent() {
    let spawn = Arc::new(
        move |_pid: String, goal: String, depth: usize, role: &str| {
            let sr = SubagentResult {
                status: "completed".into(),
                session_id: format!("child-{}", goal.chars().take(4).collect::<String>()),
                depth,
                role: Some(role.to_string()),
                ..Default::default()
            };
            tokio::spawn(async move { Ok::<_, anyhow::Error>(sr) })
        },
    );

    let handles: Vec<_> = (0..3)
        .map(|i| spawn("parent-1".into(), format!("task {}", i), 0, "leaf"))
        .collect();

    let results: Vec<_> = futures::future::join_all(handles)
        .await
        .into_iter()
        .map(|h| h.unwrap().unwrap())
        .collect();

    assert_eq!(results.len(), 3);
    for r in results.iter() {
        assert!(!r.session_id.is_empty());
        assert_eq!(r.depth, 0);
        assert_eq!(r.status, "completed");
    }
}
