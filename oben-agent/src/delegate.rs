/// Subagent delegation — spawns child agents with isolated sessions.
///
/// Maps to `tools/delegate_tool.py`.
///
/// Architecture (MVP):
/// - `delegate_task()` is the entry point
/// - Creates a child `Agent` with:
///   - Fresh `SessionManager` (shared DB, uses child session ID)
///   - Shared `Arc<Transport>` (safe, session-scoped)
///   - Shared `Arc<ToolRegistry>` (safe, filtered by delegate tool)
///   - Fresh `Box<dyn ContextWindowManager>`
/// - Runs the child agent via `Agent::turn` (single-turn with interrupt support)
/// - Returns `SubagentResult` with summary, metadata
use std::sync::Arc;

use tracing::{debug, info, warn};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::interrupt::{InterruptState, SharedInterrupt};

/// Result of executing a child agent delegation run.
///
/// Maps to the child's `run_conversation()` result in hermes-agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentResult {
    /// Whether the child completed successfully.
    pub status: SubagentStatus,
    /// The child's final response (truncated to 500 chars by delegate tool).
    pub summary: String,
    /// Number of API calls the child made.
    pub api_calls: u32,
    /// How long the child ran (monotonic seconds).
    pub duration_seconds: f64,
    /// The model the child used.
    pub model: Option<String>,
    /// The child's session ID in the shared database.
    pub session_id: String,
    /// The parent session ID that spawned this child.
    pub parent_session_id: String,
    /// Optional exit reason (for parity).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_reason: Option<String>,
}

/// Status of a child agent delegation run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SubagentStatus {
    /// Child completed normally.
    Completed,
    /// Child was interrupted by the user (Ctrl+C, /interrupt).
    Interrupted,
    /// Child hit max iterations.
    MaxIterations,
    /// Child failed due to an error (API error, timeout, etc.).
    Error(String),
}

impl SubagentResult {
    /// Create a success result.
    pub fn success(summary: String, api_calls: u32, duration: f64, model: Option<String>) -> Self {
        Self {
            status: SubagentStatus::Completed,
            summary: summary.chars().take(500).collect(),
            api_calls,
            duration_seconds: duration,
            model,
            session_id: String::new(),
            parent_session_id: String::new(),
            exit_reason: None,
        }
    }

    /// Create an interrupted result.
    pub fn interrupted(session_id: String, parent_session_id: String) -> Self {
        Self {
            status: SubagentStatus::Interrupted,
            summary: String::new(),
            api_calls: 0,
            duration_seconds: 0.0,
            model: None,
            session_id,
            parent_session_id,
            exit_reason: Some("interrupted".into()),
        }
    }

    /// Create an error result.
    pub fn error(msg: String, session_id: String, parent_session_id: String) -> Self {
        Self {
            status: SubagentStatus::Error(msg.clone()),
            summary: String::new(),
            api_calls: 0,
            duration_seconds: 0.0,
            model: None,
            session_id,
            parent_session_id,
            exit_reason: Some(msg),
        }
    }
}

/// Spawns a child agent to execute a delegated task.
///
/// This is the core logic — not a tool handler. The delegate tool
/// (in `oben-tools`) will wrap this and present it as a callable tool.
///
/// Returns immediately with a `JoinHandle` that the parent can await
/// or wait for interruption. The child runs the full conversation loop
/// (LLM call → tools → repeat until no more tool calls).
///
/// **Resource model:**
/// - Shared: `Arc<Transport>`, `Arc<ToolRegistry>`
/// - Fresh: `Box<dyn ContextWindowManager>`, `Arc<InterruptState>`, `SessionManager`
pub struct Subagent {
    /// The child's session ID.
    pub session_id: String,
    /// The parent session ID.
    pub parent_session_id: String,
    /// The goal the child is executing.
    pub goal: String,
    /// Orchestrator role of the subagent: "leaf" or "orchestrator".
    pub role: String,
    /// Nesting depth: 0 for parent, increments with each delegate call.
    pub depth: usize,
    /// Whether this subagent can further delegate (role=orchestrator && depth < max).
    pub can_delegate: bool,
    /// Thread-safe interrupt state for this subagent, shared with coordinator.
    pub interrupt_state: SharedInterrupt,
    /// Handle to retrieve the result.
    handle: tokio::task::JoinHandle<Result<SubagentResult>>,
}

impl Subagent {
    /// Get a reference to the session ID (read-only view of the handle).
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Get a reference to the parent session ID.
    pub fn parent_session_id(&self) -> &str {
        &self.parent_session_id
    }

    /// Get the goal of this subagent.
    pub fn goal(&self) -> &str {
        &self.goal
    }

    /// Await the subagent result.
    pub async fn result(self) -> Result<SubagentResult> {
        self.handle
            .await
            .map_err(|e| anyhow::anyhow!("Subagent task panicked: {e}"))?
    }

    /// Check if the subagent has completed.
    pub fn is_done(&self) -> bool {
        self.handle.is_finished()
    }

    /// Interrupt this subagent gracefully.
    ///
    /// Fires `request_interrupt()` on the subagent's shared `InterruptState`,
    /// which is polled by `TurnExecutor` during each tool-call iteration.
    pub fn interrupt(&self, message: Option<String>) {
        self.interrupt_state.request_interrupt(message);
    }
}

/// Spawn configuration builder — creates the `Subagent` with all necessary
/// resources.
///
/// This is the glue between `oben-agent` (Agent) and `oben-tools` (delegate tool).
///
/// The `SubagentSpawner::spawn()` method uses the `session_id` to create a child
/// `SessionManager` that opens the shared database and switches to that session.
#[derive(Clone)]
pub struct SubagentSpawner {
    /// Parent's transport for shared LLM API access.
    transport: Arc<dyn oben_models::providers::TransportProvider + Send + Sync>,
    /// Parent's tool registry (parent decides what subset child gets).
    tools: Arc<oben_tools::ToolRegistry>,
    /// Parent's full config — inherited by child agents for consistency.
    config: oben_config::AppConfig,
    /// Context config for child ContextWindowManager creation.
    context_config: crate::compact::CompactCofig,
    /// Max iterations per child.
    max_iterations: usize,
    /// Max messages per child.
    max_messages: usize,
    /// Maximum delegation depth. When child depth >= this value, child cannot delegate further.
    max_spawn_depth: usize,
    /// Shared hook engine so child agents fire the same hooks as the parent.
    hooks: Arc<super::hooks::HookEngine>,
}

impl SubagentSpawner {
    /// Create a new spawner.
    pub fn new(
        transport: Arc<dyn oben_models::providers::TransportProvider + Send + Sync>,
        tools: Arc<oben_tools::ToolRegistry>,
        config: oben_config::AppConfig,
        context_config: crate::compact::CompactCofig,
        max_iterations: usize,
        max_messages: usize,
        max_spawn_depth: usize,
        hooks: Arc<super::hooks::HookEngine>,
    ) -> Self {
        Self {
            transport,
            tools,
            config,
            context_config,
            max_iterations,
            max_messages,
            max_spawn_depth,
            hooks,
        }
    }

    /// Spawn a child agent.
    ///
    /// The child gets its own `SessionManager` (shared DB) pointed at the
    /// `child_session_id`, a fresh `ContextWindowManager`, and its own `Arc<InterruptState>`.
    /// The interrupt state is created **before** spawning the async task so the
    /// parent coordinator can wire it to the interruption hub.
    pub fn spawn(
        &self,
        child_session_id: String,
        parent_session_id: String,
        goal: String,
        system_prompt: String,
        depth: usize,
    ) -> Subagent {
        let transport = Arc::clone(&self.transport);
        let tools = Arc::clone(&self.tools);
        let config = self.config.clone();
        let _context_config = self.context_config.clone();
        let _max_iterations = self.max_iterations;
        let _max_messages = self.max_messages;
        let max_spawn_depth = self.max_spawn_depth;
        let child_session_id_clone = child_session_id.clone();
        let shared_hooks = Arc::clone(&self.hooks);

        // Create interrupt state OUTSIDE the async block so it is accessible
        // to the parent coordinator for subagent tree interrupt propagation.
        let interrupt_state = Arc::new(InterruptState::new());
        let interrupt_state_for_spawn = Arc::clone(&interrupt_state);

        let goal_clone = goal.clone();
        let handle = tokio::spawn(async move {
            let goal = goal_clone.clone();
            let start = std::time::Instant::now();
            let model_name = transport.name().to_owned();

            // Create child SessionManager - uses default isolation
            let sm = oben_sessions::DBSessionManager::new_with_agent(Some("default"))
                .map_err(|e| anyhow::anyhow!("Failed to create child session manager: {e}"))?;
            let child_sm: Arc<std::sync::Mutex<oben_sessions::DBSessionManager>> =
                Arc::new(std::sync::Mutex::new(sm));

            // Initialize and switch to child session
            {
                let mut mgmt = child_sm.lock().unwrap();
                if let Err(e) = mgmt.init() {
                    return Err(anyhow::anyhow!(
                        "Failed to initialize child session DB: {e}"
                    ));
                }
                if let Err(e) = mgmt.switch_session(&child_session_id_clone) {
                    return Err(anyhow::anyhow!("Failed to switch to child session: {e}"));
                }
                if let Err(e) = mgmt.load(Some(&child_session_id_clone)) {
                    return Err(anyhow::anyhow!("Failed to load child session: {e}"));
                }
            }

            // Create child ContextWindowManager (fresh engine, no need to call on_session_start)
            // Note: Child ContextWindowManager created by Agent::new below, not here.
            // (Previously passed to on_session_start, which was removed)

            // Build child agent — this is the core of delegate tool:
            // a full `Agent` with fresh context but shared transport/tools/parent-config.
            let mut child_agent = crate::AgentBuilder::new()
                .with_config(config)
                .with_system_prompt(system_prompt)
                .with_tools(tools)
                .with_hooks(shared_hooks)
                .build()
                .await
                .map_err(|e| {
                    anyhow::anyhow!("Failed to create child agent: {e}")
                })?;

            // Execute the child's turn with the goal as the first message input
            let result = child_agent
                .turn(&goal, false, Some(Arc::clone(&interrupt_state_for_spawn)))
                .await;

            let (summary, api_calls) = match result {
                Ok(text) => {
                    let _duration = start.elapsed().as_secs_f64();
                    (text, 0)
                }
                Err(e) => (format!("Subagent error: {e}"), 0),
            };

            Ok(SubagentResult::success(
                summary,
                api_calls,
                start.elapsed().as_secs_f64(),
                Some(model_name.to_owned()),
            ))
        });

        Subagent {
            session_id: child_session_id,
            parent_session_id,
            goal,
            role: "orchestrator".into(),
            depth,
            can_delegate: depth < max_spawn_depth,
            interrupt_state,
            handle,
        }
    }
}

/// The spawn_fn closure that the delegate tool calls.
///
/// Type signature: `Fn(String, SpawnedSession) -> Subagent`
pub type SpawnFn = Arc<dyn FnMut(String, SpawnedSession) -> Subagent + Send + Sync>;

/// Metadata returned when spawning a child session for subagent delegation.
///
/// This is the same struct from `SessionManager::spawn_session_for_subagent()`.
/// The `SubagentSpawner::spawn()` method uses the `session_id` to create a child
/// `SessionManager` that opens the shared database and switches to that session.
#[derive(Clone, Debug)]
pub struct SpawnedSession {
    /// The newly created child session ID.
    pub session_id: String,
    /// The parent session ID that the child is linked to.
    pub parent_session_id: String,
}

/// Build the child toolset — exclude `delegate_task` when depth already reached the floor.
///
/// When `depth >= max_spawn_depth`, the child becomes a leaf and cannot delegate further.
fn build_child_toolset(
    parent_tools: &Arc<oben_tools::ToolRegistry>,
    depth: usize,
    max_depth: usize,
) -> Arc<oben_tools::ToolRegistry> {
    let blocked: &[&str] = if depth >= max_depth {
        &["delegate_task"]
    } else {
        &[]
    };

    if blocked.is_empty() {
        // No filtering needed — same ref
        Arc::clone(parent_tools)
    } else {
        // Create filtered clone with blocked tools excluded
        let filtered = parent_tools.filtered_clone(blocked);
        Arc::new(filtered)
    }
}

/// Build the child system prompt based on role and depth.
/// For orchestrator children, inject guidance about delegation tree depth.
fn build_child_system_prompt(
    parent_prompt: &str,
    child_depth: usize,
    max_spawn_depth: usize,
    role: &str,
    _goal: &str,
) -> String {
    let mut parts = Vec::new();

    // Use the full parent prompt as base (identity + tool guidance + all stable sections)
    parts.push(parent_prompt.to_string());

    // Add child-specific delegation context
    parts.push(format!(
        "## Delegation Context\n\n\
        You are a delegated subagent spawned by a parent agent to accomplish a task.\n\
        Your direct instructions come from your conversation history.\n\n\
        ## Delegation Role & Depth\n\n\
        Your role in the delegation tree: **{role}**.\n\
        Current depth: {child_depth}. Maximum allowed depth: {max_spawn_depth}."
    ));

    // Role-specific guidance (mirrors what hermes puts in ephemeral_system_prompt)
    match role {
        "orchestrator" => {
            if child_depth >= max_spawn_depth {
                let next_depth = child_depth + 1;
                parts.push(format!(
                    "\n## Orchestrator Depth Floor\n\n\
                    You are orchestrator at depth {child_depth}/{max_spawn_depth}. \
                    The depth floor prevents you from delegating further. \
                    Your children will be at depth {next_depth}, exceeding the limit.\n\
                    Act as a leaf: execute directly with available tools."
                ));
            } else {
                let child_depth_plus_1 = child_depth + 1;
                parts.push(format!(
                    "\n## Orchestrator Guidance\n\n\
                    You CAN spawn subagents via `delegate_task` for parallelization.\n\
                    You are at depth {child_depth}/{max_spawn_depth}. Your children will be at depth {child_depth_plus_1}.\n\
                    When children reach depth {max_spawn_depth}, they become leaves.\n\
                    Coordinate your children's results and synthesize a cohesive summary.\n\
                    Your summary must be actionable — not just a list of child results."
                ));
            }
        }
        _ => {
            parts.push("\n## Leaf Agent\n\nYou are a **leaf** subagent — you cannot delegate further. Execute your objective directly.".to_string());
        }
    }

    parts.join("\n\n")
}

/// Build the spawn_fn that the delegate tool calls.
///
/// The returned closure takes `(parent_session_id, goal, depth, role)`.
/// It creates a child session in the shared DB via a temporary SessionManager,
/// then builds the child agent and executes it.
///
/// This is the TUI layer's bridge between `oben-agent::SubagentSpawner` and `oben-tools::SpawnFn`.
pub fn build_spawn_fn_wrapper(
    spawner: SubagentSpawner,
    parent_system_prompt: String,
) -> oben_tools::registry::SpawnFn {
    // Take ownership of the spawner's Arcs by moving them into the closure.
    // These Arcs point to a SEPARATE ToolRegistry allocation (created in init_agent
    // via ToolRegistry::clone). So self.tools stays unique (refcount 1) and
    // Arc::get_mut(&mut self.tools) succeeds for delegate tool registration.
    let transport = spawner.transport;
    let tools = spawner.tools;
    let config = spawner.config;
    let context_config = spawner.context_config.clone();
    let max_iterations = spawner.max_iterations;
    let max_messages = spawner.max_messages;
    let max_spawn_depth = spawner.max_spawn_depth;
    let shared_hooks = spawner.hooks;

    Arc::new(
        move |parent_session_id: String, goal: String, depth: usize, role: &str| {
            info!(
                "delegate: spawn_fn_wrapper parent_session_id={} depth={} role={}",
                parent_session_id, depth, role
            );
            debug!(
                "delegate: spawn_fn_wrapper goal={}",
                &goal.chars().take(100).collect::<String>()
            );

            // Create interrupt state OUTSIDE the async block so it is accessible
            // to the parent coordinator for subagent tree interrupt propagation.
            let interrupt_state = Arc::new(InterruptState::new());

            let transport = Arc::clone(&transport);
            let tools = Arc::clone(&tools);
            let _context_config = context_config.clone();
            let _max_iterations = max_iterations;
            let _max_messages = max_messages;
            let _max_spawn_depth = max_spawn_depth;

            // Build child toolset: exclude delegate_task if depth >= max_spawn_depth
            let child_tool_registry = build_child_toolset(&tools, depth, max_spawn_depth);

            // Build child system prompt with orchestrator guidance if needed
            let child_system_prompt = build_child_system_prompt(
                &parent_system_prompt,
                depth,
                max_spawn_depth,
                role,
                &goal,
            );

            let parent_session_id_clone = parent_session_id.clone();
            let goal_clone = goal.clone();
            let role_clone = role.to_string();
            let config_for_child = config.clone();
            let shared_hooks_for_child = Arc::clone(&shared_hooks);

            info!(
                "delegate: spawning child parent_session_id={} depth={} role={}",
                parent_session_id_clone, depth, role_clone
            );
            let goal_short: String = goal_clone.chars().take(80).collect();
            debug!("delegate: child goal starts with: {}", goal_short);

            let is_clone = Arc::clone(&interrupt_state);
            tokio::spawn(async move {
                let start = std::time::Instant::now();

                // Create child session in shared DB using a temporary SessionManager
                info!(
                    "delegate: init_child_session_for_creation parent_session_id={}",
                    parent_session_id_clone
                );
                let temp_sm = oben_sessions::DBSessionManager::new_with_agent(Some("default"))
                    .map_err(|e| anyhow::anyhow!("Failed to create child session manager: {e}"));
                let spawned = match temp_sm {
                    Ok(mut sm) => {
                        if let Err(e) = sm.init() {
                            warn!("delegate: child init DB failed in {:0.2}s parent_session_id={}: {}", start.elapsed().as_secs_f64(), parent_session_id_clone, e);
                            return Err(anyhow::anyhow!(
                                "Failed to initialize child session DB: {e}"
                            ));
                        }
                        // Create child session directly to avoid spawn_session_for_subagent
                        // requiring an active parent session (it only needs active_session_id).
                        let child = sm.create_session(&goal_clone);
                        let child_id = child.id.clone();
                        // Set parent reference in both DB and in-memory session metadata
                        if let Err(e) =
                            sm.set_parent_session_id_for_child(&child_id, &parent_session_id_clone)
                        {
                            return Err(anyhow::anyhow!("Failed to set parent session ID: {e}"));
                        }
                        if let Some(s) = sm.session_mut(&child_id) {
                            s.metadata.parent_session_id = Some(parent_session_id_clone.clone());
                        }
                        Ok(SpawnedSession {
                            session_id: child_id,
                            parent_session_id: parent_session_id_clone.clone(),
                        })
                    }
                    Err(e) => Err(e),
                };
                let spawned = match spawned {
                    Ok(sp) => sp,
                    Err(e) => {
                        warn!("delegate: child session creation FAILED in {:0.2}s parent_session_id={}: {}", start.elapsed().as_secs_f64(), parent_session_id_clone, e);
                        return Err(e);
                    }
                };
                let child_session_id = spawned.session_id;

                // Create child SessionManager
                info!(
                    "delegate: init_child_session_manager child_session_id={}",
                    child_session_id
                );
                let sm = oben_sessions::DBSessionManager::new_with_agent(Some("default"))
                    .map_err(|e| anyhow::anyhow!("Failed to create child session manager: {e}"));

                let sm = match sm {
                    Ok(sm) => sm,
                    Err(e) => {
                        warn!(
                            "delegate: child agent FAILED in {:0.2}s child_session_id={}: {}",
                            start.elapsed().as_secs_f64(),
                            child_session_id,
                            e
                        );
                        return Err(e);
                    }
                };
                let child_sm: Arc<std::sync::Mutex<oben_sessions::DBSessionManager>> =
                    Arc::new(std::sync::Mutex::new(sm));

                // Initialize and switch to child session
                {
                    let mut mgmt = child_sm.lock().unwrap();
                    info!(
                        "delegate: init_child_db child_session_id={}",
                        child_session_id
                    );
                    if let Err(e) = mgmt.init() {
                        warn!(
                            "delegate: child init DB failed in {:0.2}s child_session_id={}: {}",
                            start.elapsed().as_secs_f64(),
                            child_session_id,
                            e
                        );
                        return Err(anyhow::anyhow!(
                            "Failed to initialize child session DB: {e}"
                        ));
                    }
                    if let Err(e) = mgmt.switch_session(&child_session_id) {
                        warn!("delegate: child switch session failed in {:0.2}s child_session_id={}: {}", start.elapsed().as_secs_f64(), child_session_id, e);
                        return Err(anyhow::anyhow!("Failed to switch to child session: {e}"));
                    }
                    if let Err(e) = mgmt.load(Some(&child_session_id)) {
                        warn!("delegate: child load session failed in {:0.2}s child_session_id={}: {}", start.elapsed().as_secs_f64(), child_session_id, e);
                        return Err(anyhow::anyhow!("Failed to load child session: {e}"));
                    }
                }
                info!(
                    "delegate: child DB and session initialized in {:0.2}s child_session_id={}",
                    start.elapsed().as_secs_f64(),
                    child_session_id
                );

                // Create child ContextWindowManager
                info!(
                    "delegate: init_child_context_window_manager child_session_id={}",
                    child_session_id
                );
                // Note: Child ContextWindowManager created by Agent::new below, not here.
                // (Previously passed to on_session_start, which was removed)

                info!("delegate: build_child_agent child_session_id={} depth={} role={} max_iterations={}", child_session_id, depth, role_clone, max_iterations);
                let mut child_agent = crate::AgentBuilder::new()
                    .with_config(config_for_child)
                    .with_system_prompt(child_system_prompt)
                    .with_tools(child_tool_registry)
                    .with_hooks(shared_hooks_for_child)
                    .build()
                    .await
                    .map_err(|e| {
                        warn!(
                            "delegate: build_child_agent FAILED in {:0.2}s child_session_id={}: {}",
                            start.elapsed().as_secs_f64(),
                            child_session_id,
                            e
                        );
                        anyhow::anyhow!("Failed to create child agent: {e}")
                    })?;

                // Execute the child's turn with the goal
                info!(
                    "delegate: child_agent_turn START child_session_id={} depth={}",
                    child_session_id, depth
                );
                let result = child_agent
                    .turn(&goal_clone, false, Some(Arc::clone(&is_clone)))
                    .await;

                let duration = start.elapsed().as_secs_f64();
                let (summary, _api_calls) = match result {
                    Ok(text) => {
                        let len = text.len();
                        let summary_short: String = text.chars().take(80).collect();
                        info!("delegate: child_agent_turn SUCCESS in {:0.2}s child_session_id={} len={} depth={}", duration, child_session_id, len, depth);
                        debug!("delegate: child result preview: {}", summary_short);
                        (text, 0u32)
                    }
                    Err(e) => {
                        warn!(
                            "delegate: child_agent_turn FAILED in {:0.2}s child_session_id={}: {}",
                            duration, child_session_id, e
                        );
                        (format!("Subagent error: {e}"), 0)
                    }
                };

                let _can_delegate = depth < max_spawn_depth && role_clone == "orchestrator";

                Ok(oben_tools::registry::SubagentResult {
                    status: "completed".into(),
                    summary: summary.chars().take(500).collect(),
                    api_calls: _api_calls,
                    duration_seconds: 0.0,
                    model: Some(transport.name().to_owned()),
                    session_id: child_session_id,
                    parent_session_id: parent_session_id_clone,
                    role: Some(role_clone.clone()),
                    depth,
                    exit_reason: None,
                })
            })
        },
    )
}

impl From<SubagentResult> for oben_tools::registry::SubagentResult {
    fn from(sr: SubagentResult) -> Self {
        Self {
            status: match sr.status {
                SubagentStatus::Completed => "completed".into(),
                SubagentStatus::Interrupted => "interrupted".into(),
                SubagentStatus::MaxIterations => "max_iterations".into(),
                SubagentStatus::Error(ref e) => format!("error:{e}"),
            },
            summary: sr.summary,
            api_calls: sr.api_calls,
            duration_seconds: sr.duration_seconds,
            model: sr.model,
            session_id: sr.session_id,
            parent_session_id: sr.parent_session_id,
            role: None,
            depth: 0,
            exit_reason: sr.exit_reason,
        }
    }
}
