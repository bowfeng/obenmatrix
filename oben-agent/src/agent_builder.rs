//! Agent builder — construct `Agent` with optional hooks sharing and richer errors.
//!
//! `AgentBuilder` wraps `Agent::new()` with a builder pattern that:
//! - Accepts an optional shared `Arc<HookEngine>` (enables subagent hook sharing)
//! - Wraps transport initialization in `anyhow::Context` for better error messages
//! - Provides a fluent `.with_*()` API culminating in `.build()`

use std::sync::Arc;

use anyhow::Context;
use tokio::sync::Mutex;

use crate::compact_context::BuiltinContextWindowManager;
use crate::hooks::HookEngine;
use crate::interrupt::InterruptState;
use crate::Agent;

use oben_config::AppConfig;
use oben_models::providers::TransportProvider;
use oben_tools::ToolRegistry;

/// Builder for [`Agent`].
///
/// Holds configuration fragments and assembles an `Agent` in
/// [`build`][Self::build].  If a shared `HookEngine` is provided via
/// `.with_hooks()` the builder re-uses it instead of creating a new one,
/// which is how subagents share hooks with the parent.
pub struct AgentBuilder {
    config: Option<AppConfig>,
    system_prompt: Option<String>,
    tools: Option<Arc<ToolRegistry>>,
    hooks: Option<Arc<HookEngine>>,
}

impl AgentBuilder {
    /// Create a new, empty `AgentBuilder`.
    ///
    /// All fields are `None` until populated by builder methods.
    pub fn new() -> Self {
        Self {
            config: None,
            system_prompt: None,
            tools: None,
            hooks: None,
        }
    }

    /// Set the application configuration.
    pub fn with_config(mut self, config: AppConfig) -> Self {
        self.config = Some(config);
        self
    }

    /// Set the system prompt.
    pub fn with_system_prompt(mut self, system_prompt: String) -> Self {
        self.system_prompt = Some(system_prompt);
        self
    }

    /// Set the tool registry.
    pub fn with_tools(mut self, tools: Arc<ToolRegistry>) -> Self {
        self.tools = Some(tools);
        self
    }

    /// Provide a shared `HookEngine` instead of creating a new one.
    ///
    /// This is the mechanism by which subagents share hooks with the
    /// parent agent.  If not provided, `build()` creates a default
    /// `HookEngine` from the config.
    pub fn with_hooks(mut self, hooks: Arc<HookEngine>) -> Self {
        self.hooks = Some(hooks);
        self
    }

    /// Build the [`Agent`].
    ///
    /// All required fields (`config`, `system_prompt`, `tools`) must be
    /// set before calling this method — missing fields produce descriptive
    /// errors rather than panics.
    ///
    /// Wraps transport initialization in [`anyhow::Context`] so transport
    /// failures surface a "connection failed" hint.
    pub async fn build(self) -> anyhow::Result<Agent> {
        let config = self
            .config
            .ok_or_else(|| anyhow::anyhow!("AgentBuilder: config is required — call .with_config()"))?;

        let system_prompt = self
            .system_prompt
            .ok_or_else(|| anyhow::anyhow!("AgentBuilder: system_prompt is required — call .with_system_prompt()"))?;

        let tools = self
            .tools
            .ok_or_else(|| anyhow::anyhow!("AgentBuilder: tools is required — call .with_tools()"))?;

        // Prepare tool metadata for transport initialization.
        let system_prompt_cloned = system_prompt.clone();
        let tools_for_transport: Vec<oben_models::ToolMeta> =
            tools.list_tools().iter().map(|t| t.clone()).collect();

        // Initialize transport — wrap in context for better error messages.
        let transport: Arc<dyn TransportProvider + Send + Sync> = above_transport(
            &config.model,
            &system_prompt_cloned,
            &tools_for_transport,
        )
        .context("connection failed — check your model config (endpoint, api_key, model)")?;

        let session_manager = Arc::new(Mutex::new(
            oben_sessions::SessionStore::new(config.session_store.clone())
                .context("failed to initialize session store")?,
        ));

        // Re-use provided hooks or build a new one.
        let hooks = match self.hooks {
            Some(hooks) => hooks,
            None => Arc::new(
                crate::hooks::HookBuilder::from_config(&config.hooks).build(),
            ),
        };

        let context_config = crate::compact::CompactCofig {
            context_length: config.context.context_length,
            threshold_percent: config.context.threshold_percent,
            ..crate::compact::CompactCofig::default()
        };

        let mut agent = Agent {
            transport,
            tools,
            context_window_manager: Box::new(
                BuiltinContextWindowManager::with_config(context_config),
            ),
            call_mode: None,
            session_manager,
            interrupt_state: Arc::new(InterruptState::new()),
            config,
            fallback_chain: None,
            system_prompt,
            hooks,
        };

        agent.eager_load_active_session().await;
        Ok(agent)
    }
}

/// Initialize the transport layer with error context.
fn above_transport(
    config: &oben_models::ProviderConfig,
    system_prompt: &str,
    tools: &[oben_models::ToolMeta],
) -> anyhow::Result<Arc<dyn TransportProvider + Send + Sync>> {
    use oben_transport::Transport;

    let t: Arc<dyn TransportProvider + Send + Sync> =
        Transport::from_config_with_tools_via_registry(config, system_prompt, tools);
    Ok(t)
}

impl Default for AgentBuilder {
    fn default() -> Self {
        Self::new()
    }
}
