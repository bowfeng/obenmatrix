//! Shared agent state — extracted from `App` to decouple agent lifecycle from rendering.
//!
//! `App` now owns only rendering state (panels, turn_state, toasts).
//! `SharedAgentState` owns the agent, tools, turn handle, and session data.
//! Both are passed via `Arc<Mutex<SharedAgentState>>` to the TUI coordinator
//! and event loop.

use std::sync::Arc;

use oben_agent::delegate::{build_spawn_fn_wrapper, SubagentSpawner};
use oben_agent::{hooks::SubagentLifecycleCallback, Agent, AgentBuilder, TurnState};
use oben_config::AppConfig;
use oben_tools::delegate::DelegateTool;
use oben_tools::ToolRegistry;
use parking_lot::Mutex as PlMutex;
use parking_lot::RwLock;
use anyhow::Result;
use crate::widgets::message_renderer::StyledLine;

/// Information about a subagent execution, aggregated from delegation results.
///
/// This struct mirrors the Hermes TUI SubagentAccordion model: each subagent
/// has sections for thinking, tool calls, progress/summary, and children.
/// Rendered as an expandable accordion in the details panel.
#[derive(Debug, Clone, Default)]
pub struct SubagentInfo {
    /// Unique identifier for this subagent (maps to delegation_id).
    pub delegation_id: u32,
    /// The goal statement for this subagent.
    pub goal: String,
    /// Overall status: idle, running, complete, error.
    pub status: String,
    /// Rendered thinking/reasoning lines (already styled).
    pub thinking: Option<Vec<StyledLine>>,
    /// Tool calls executed by this subagent.
    pub tool_calls: Vec<String>,
    /// Tool call previews (tool_name + first 30 chars of arguments).
    pub tool_call_previews: Vec<String>,
    /// Start and end timestamps.
    pub start_time: Option<String>,
    pub end_time: Option<String>,
    /// The parent session ID this subagent was spawned from.
    pub parent_session_id: String,
    /// Truncated summary/result text from the subagent.
    pub summary: String,
    /// Aggregate stats.
    pub stats: SubagentStats,
    /// Nested subagent children (grandchildren in delegation tree).
    pub children: Vec<SubagentInfo>,
}

/// Aggregate statistics for a subagent execution.
#[derive(Debug, Clone, Default)]
pub struct SubagentStats {
    /// Number of tool calls made.
    pub tool_count: usize,
    /// Approximate token count across all subagent messages.
    pub token_count: usize,
    /// Elapsed duration string (e.g. "3m", "45s").
    pub duration: String,
    /// Estimated cost in USD.
    pub cost_usd: f64,
}

/// Agent-owning state extracted from `App`.
///
/// This struct holds everything related to the agent lifecycle:
/// - The agent itself (`Arc<Mutex<Agent>>`)
/// - Tool registry and tool names
/// - Active session ID and turn handle
/// - `TurnState` for rendering callbacks
///
/// `App` retains only rendering state (panels, toasts, input handling).
pub struct SharedAgentState {
    /// Agent — None before initialization, always Some after.
    pub agent: Option<Arc<tokio::sync::Mutex<Agent>>>,
    /// Message count in the session before the current turn's user message
    /// is inserted. Used to truncate orphaned in-memory messages on abort.
    pub turn_message_count: usize,
    /// Handle for the currently running turn task (used to abort).
    pub turn_handle: Option<tokio::task::JoinHandle<()>>,
    /// Active session ID.
    pub session_id: Option<String>,
    /// Tool registry.
    pub tools: std::sync::Arc<ToolRegistry>,
    /// Names of all registered tools.
    pub tool_names: Vec<String>,
    /// Shared `Arc<Mutex<TurnState>>` — owned here because it's written by the
    /// agent's hook adapters during turns.
    pub turn_state: Arc<PlMutex<TurnState>>,
    /// Accumulated subagent info from the current turn, grouped by delegation_id.
    /// Populated during `run_conversation()` and cleared after the turn completes.
    pub subagents: Arc<RwLock<Vec<SubagentInfo>>>,
}

impl SharedAgentState {
    /// Initialize agent and return a new `SharedAgentState`.
    ///
    /// This builds the tool transport, registers the delegate tool,
    /// creates the `Agent`, and initializes `TurnState`.
    pub async fn init(config: &AppConfig) -> Result<Self> {
        let mut tools = ToolRegistry::new();
        oben_tools::discover_builtin_tools(&mut tools);

        let tool_names: Vec<String> = tools.list_tools().iter().map(|t| t.name.clone()).collect();

        let identity = oben_config::defaults::default_system_prompt();
        let skills_dirs: Vec<std::path::PathBuf> = vec![];
        let volatile = oben_agent::system_prompt::build_volatile_block(
            None,
            None,
            Some(&config.model.model),
        );
        let assembled = oben_agent::system_prompt::build_system_prompt(
            &identity,
            &tool_names,
            &skills_dirs,
            None,
            None,
            Some(&volatile),
        );

        // Build delegate tool transport before creating the agent (we need
        // exclusive access to self.tools for delegate registration).
        let delegate_transport =
            oben_transport::Transport::from_config_with_tools_via_registry(
                &config.model,
                &assembled.prompt,
                &tools
                    .list_tools()
                    .iter()
                    .map(|t| (*t).clone())
                    .collect::<Vec<oben_models::ToolMeta>>(),
            );

        let shared_hooks = Arc::new(
            oben_agent::hooks::HookBuilder::from_config(&config.hooks).build(),
        );

        let spawner = SubagentSpawner::new(
            Arc::new(delegate_transport),
            Arc::new(ToolRegistry::clone(&tools)),
            config.clone(),
            oben_agent::compact::CompactCofig {
                context_length: config.context.context_length,
                threshold_percent: config.context.threshold_percent,
                ..oben_agent::compact::CompactCofig::default()
            },
            config.max_iterations.unwrap_or(50),
            config.context.max_messages.unwrap_or(100),
            config.max_spawn_depth.unwrap_or(3),
            shared_hooks.clone(),
        );
        let spawn_fn = build_spawn_fn_wrapper(spawner, assembled.prompt.clone());
        let mut tools_for_reg = ToolRegistry::clone(&tools);
        tools_for_reg.register(DelegateTool::new(
            spawn_fn,
            config.max_concurrent_tasks.unwrap_or(5),
        ));

        let agent = Arc::new(tokio::sync::Mutex::new(
            AgentBuilder::new()
                .with_config(config.clone())
                .with_system_prompt(assembled.prompt.clone())
                .with_tools(Arc::new(tools_for_reg.clone()))
                .with_hooks(shared_hooks)
                .build()
                .await?,
        ));

        let session_id = agent.lock().await.context_window_manager().session_id();

        Ok(Self {
            agent: Some(agent),
            turn_message_count: 0,
            turn_handle: None,
            session_id,
            tools: Arc::new(tools_for_reg),
            tool_names,
            turn_state: Arc::new(PlMutex::new(TurnState::new())),
            subagents: Arc::new(RwLock::new(Vec::new())),
        })
    }

    /// Get the interrupt state handle from the running agent.
    pub async fn interrupt_handle(&self) -> Option<Arc<oben_agent::interrupt::InterruptState>> {
        self.agent.as_ref().map(|a| {
            Arc::clone(&a.blocking_lock().get_interrupt_state())
        })
    }

    /// Refresh session_id from the agent's CWM, since sessions are created lazily on first turn.
    pub async fn sync_session_id(&mut self) {
        if let Some(agent) = &self.agent {
            let id = agent.lock().await.context_window_manager().session_id();
            if id.as_deref() != self.session_id.as_deref() {
                self.session_id = id;
            }
        }
    }

    /// Check if agent is initialized.
    pub fn is_initialized(&self) -> bool {
        self.agent.is_some()
    }

    /// Run the clear command on the agent.
    pub async fn cmd_clear(&self) -> Result<()> {
        if let Some(agent) = &self.agent {
            let mut guard = agent.lock().await;
            guard.reset().await?;
        }
        Ok(())
    }

    /// Run the new session command on the agent.
    pub async fn cmd_new_session(&self) -> Result<String> {
        if let Some(agent) = &self.agent {
            let new_id = agent.lock().await.new_session().await?;
            Ok(new_id)
        } else {
            Err(anyhow::anyhow!("Agent not initialized"))
        }
    }

    /// Run the compact session command on the agent.
    pub async fn cmd_compact_session(&self) -> Result<()> {
        if let Some(agent) = &self.agent {
            agent.lock().await.compact_session().await;
        }
        Ok(())
    }

    pub async fn get_interrupt_state(&self) -> Option<Arc<oben_agent::interrupt::InterruptState>> {
        self.agent.as_ref().map(|a| Arc::clone(&a.blocking_lock().get_interrupt_state()))
    }

    pub fn active_session_name(&self) -> Option<String> {
        // No-op until ActiveSessionName is wired upstream
        self.agent.as_ref().map(|_| String::new())
    }

    /// Load session messages from storage and return them.
    pub async fn loaded_session_messages(&self) -> Vec<oben_models::Message> {
        if let Some(agent) = &self.agent {
            agent.lock().await.loaded_session_messages().await.unwrap_or_default()
        } else {
            Vec::new()
        }
    }

    pub async fn load_session_messages(&self, session_id: &str) -> Result<()> {
        if let Some(agent) = &self.agent {
            agent.lock().await.load_session_messages(session_id).await?;
        }
        Ok(())
    }

    pub async fn get_session_messages(&self, session_id: &str) -> Vec<oben_models::Message> {
        if let Some(agent) = &self.agent {
            agent.lock().await.get_session_messages(session_id).await.unwrap_or_default()
        } else {
            Vec::new()
        }
    }

    pub async fn switch_session_to(&self, session_id: &str) -> Result<()> {
        if let Some(agent) = &self.agent {
            agent.lock().await.switch_session_to(session_id).await?;
        }
        Ok(())
    }

    pub async fn list_sessions_full(&self) -> Vec<oben_models::Session> {
        if let Some(agent) = &self.agent {
            agent.lock().await.list_sessions_full().await
        } else {
            Vec::new()
        }
    }

    pub async fn init_session_manager(&self) -> Result<()> {
        if let Some(agent) = &self.agent {
            agent.lock().await.init_session_manager().await?;
        }
        Ok(())
    }

    pub async fn find_session_key(&self, _name: &str) -> Option<String> {
        None
    }

    pub fn steer(&self, _text: &str) -> bool {
        false
    }

    pub async fn compact_session(&self) -> oben_agent::compact::CompactOutcome {
        if let Some(agent) = &self.agent {
            agent.lock().await.compact_session().await
        } else {
            oben_agent::compact::CompactOutcome::AlreadyCompact
        }
    }

    /// Empty constructor for pre-init state.
    pub fn new_empty() -> Self {
        Self {
            agent: None,
            turn_message_count: 0,
            turn_handle: None,
            session_id: None,
            tools: Arc::new(ToolRegistry::new()),
            tool_names: Vec::new(),
            turn_state: Arc::new(PlMutex::new(TurnState::new())),
            subagents: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Get the raw agent Arc (for extraction into TuiCoordinator).
    pub fn agent_arc(&self) -> Option<&Arc<tokio::sync::Mutex<Agent>>> {
        self.agent.as_ref()
    }

    /// Get a snapshot of the current subagents for rendering.
    pub fn get_subagents(&self) -> Vec<SubagentInfo> {
        self.subagents.read().clone()
    }

    /// Clear subagent data after turn completes.
    pub fn clear_subagents(&self) {
        self.subagents.write().clear();
    }
}

impl std::fmt::Debug for SharedAgentState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedAgentState")
            .field("turn_message_count", &self.turn_message_count)
            .field("session_id", &self.session_id)
            .field("tool_count", &self.tool_names.len())
            .finish()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// SubagentCallback — concrete wrapper implementing SubagentLifecycleCallback
// ─────────────────────────────────────────────────────────────────────────────

/// A callback that bridges the agent's hook system to SharedAgentState.subagents.
///
/// This struct is `Arc`-shareable and holds a reference to the SharedAgentState
/// so that when the TUI hook adapter fires `on_tool_start`/`on_tool_complete`
/// for delegate_task, the resulting SubagentInfo is populated.
pub struct SubagentCallback {
    shared_state: std::sync::Arc<parking_lot::Mutex<SharedAgentState>>,
}

impl SubagentCallback {
    pub fn new(state: std::sync::Arc<parking_lot::Mutex<SharedAgentState>>) -> Self {
        Self { shared_state: state }
    }
}

impl SubagentLifecycleCallback for SubagentCallback {
    fn on_start(
        &self,
        delegation_id: u32,
        parent_session_id: &str,
        goal: &str,
    ) {
        let guard = self.shared_state.lock();
        let mut subs = guard.subagents.write();
        subs.push(SubagentInfo {
            delegation_id,
            goal: goal.to_string(),
            status: "running".into(),
            start_time: Some(format!("{}", chrono::Utc::now().format("%H:%M:%S"))),
            parent_session_id: parent_session_id.to_string(),
            ..Default::default()
        });
    }

    fn on_complete(
        &self,
        delegation_id: u32,
        result: &str,
        _status: &str,
        _tool_calls: Vec<oben_agent::hooks::SubagentToolInfo>,
    ) {
        let guard = self.shared_state.lock();
        let mut subs = guard.subagents.write();
        for sub in subs.iter_mut() {
            if sub.delegation_id == delegation_id {
                sub.status = "completed".into();
                sub.end_time = Some(format!("{}", chrono::Utc::now().format("%H:%M:%S")));
                sub.summary = result.chars().take(200).collect();
                break;
            }
        }
    }
}
