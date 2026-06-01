//! Application state and core logic.

use anyhow::Result;
use crate::panels::chat::ChatPanel;
use crate::panels::config::ConfigPanel;
use crate::panels::sessions::SessionsPanel;
use crate::panels::setup::SetupPanel;
use crate::panels::{Panel, PanelId};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::info;

use crate::event::EventBus;
use crate::history;
use oben_agent::{Agent, AgentCallbacks, AgentConfig};
use oben_config::AppConfig;
use oben_tools::ToolRegistry;

/// Payload carried by TurnDone completion event from spawned task.
pub(super) struct TurnCompletion {
    pub(super) success: bool,
    pub(super) session_name: Option<String>,
    pub(super) messages: Vec<oben_models::Message>,
}

pub struct App {
    pub running: bool,
    pub active_panel: PanelId,
    pub panels: HashMap<PanelId, Box<dyn Panel>>,
    pub status: String,
    pub config: AppConfig,
    /// Agent protected by TokioMutex — guard is Send, needed for spawn()
    /// where we hold the lock across .await in agent.turn().
    pub agent: Option<Arc<tokio::sync::Mutex<Agent>>>,
    pub turn_handle: Option<tokio::task::JoinHandle<()>>,
    pub session_id: Option<String>,
    pub tools: std::sync::Arc<ToolRegistry>,
    pub tool_names: Vec<String>,
    pub input_tx: Option<tokio::sync::mpsc::UnboundedSender<crate::TuiEvent>>,
    pub input_history: history::InputHistory,
    pub paste_mode: bool,
    /// Event bus — single event emission point, wraps TurnState internally.
    /// Agents and UI components emit events through here to keep UI logic
    /// testable independently from the Agent.
    pub event_bus: Arc<EventBus>,
}

impl App {
    /// Downcast the stored ChatPanel — reads.
    pub(super) fn get_chat(&self) -> Option<&ChatPanel> {
        self.panels
            .get(&PanelId::Chat)
            .and_then(|p| p.downcast_ref::<ChatPanel>())
    }

    /// Downcast the stored ChatPanel — mutates.
    pub(super) fn get_chat_mut(&mut self) -> Option<&mut ChatPanel> {
        self.panels
            .get_mut(&PanelId::Chat)
            .and_then(|p| p.downcast_mut::<ChatPanel>())
    }

    /// Downcast the stored SessionsPanel — reads.
    pub(super) fn get_sessions(&self) -> Option<&SessionsPanel> {
        self.panels
            .get(&PanelId::Sessions)
            .and_then(|p| p.downcast_ref::<SessionsPanel>())
    }
    /// Activate a panel and call its on_activate/on_deactivate hooks.
    pub async fn activate_panel(&mut self, panel: PanelId) {
        let old = self.active_panel;

        // Deactivate old panel
        if old != panel {
            if let Some(mut boxed_panel) = self.panels.remove(&old) {
                boxed_panel.on_deactivate(self).await;
                self.panels.insert(old, boxed_panel);
            }
        }

        self.active_panel = panel;

        // Activate new panel
        if let Some(mut boxed_panel) = self.panels.remove(&panel) {
            if panel == PanelId::Chat {
                if let Some(s) = boxed_panel.downcast_mut::<ChatPanel>() {
                    if let Some(agent) = &self.agent {
                        let guard = agent.lock().await;
                        let session_name = guard.active_session_name().map(|n| n.clone());
                        let messages = guard
                            .session_manager()
                            .active_session()
                            .map(|sess| sess.messages.clone())
                            .unwrap_or_default();
                        s.set_session_data(session_name, messages);
                    }
                }
            }
            boxed_panel.on_activate(self).await;
            self.panels.insert(panel, boxed_panel);
        }
    }

    pub fn new() -> Result<Self> {
        let config = AppConfig::load()?;
        let mut tools = ToolRegistry::new();
        oben_tools::discover_builtin_tools(&mut tools);
        let tool_names: Vec<String> = tools.list_tools().iter().map(|t| t.name.clone()).collect();
        Ok(Self {
            running: true,
            active_panel: PanelId::Chat,
            panels: HashMap::new(),
            status: String::new(),
            config,
            agent: None,
            turn_handle: None,
            session_id: None,
            tools: std::sync::Arc::new(tools),
            tool_names,
            input_tx: None,
            input_history: history::InputHistory::new(),
            paste_mode: false,
            event_bus: Arc::new(EventBus::new()),
        })
    }

    pub fn init_agent(&mut self) -> Result<()> {
        let identity = oben_config::defaults::default_system_prompt();
        let skills_dirs: Vec<std::path::PathBuf> = vec![];
        let volatile = oben_agent::system_prompt::build_volatile_block(
            None,
            None,
            Some(&self.config.model.model),
        );
        let assembled = oben_agent::system_prompt::build_system_prompt(
            &identity,
            &self.tool_names,
            &skills_dirs,
            None,
            None,
            Some(&volatile),
        );
        let transport = oben_transport::Transport::from_config_with_tools_via_registry(
            &self.config.model,
            &assembled.prompt,
            &self
                .tools
                .list_tools()
                .iter()
                .map(|t| (*t).clone())
                .collect::<Vec<oben_models::Tool>>(),
        );

        let event_bus = Arc::clone(&self.event_bus);

        let callbacks = AgentCallbacks {
            step: Some(Box::new(move |msg: &str| {
                info!("STEP: {}", msg);
            })),
            status: Some(Box::new(move |level: &str, msg: &str| {
                info!("STATUS [{}]: {}", level, msg);
            })),
            tool_start: {
                let eb = Arc::clone(&event_bus);
                Some(Box::new(move |tool_name: &str, args_json: &str| {
                    eb.on_tool_start("tool-id", tool_name, args_json);
                }))
            },
            tool_complete: {
                let eb = Arc::clone(&event_bus);
                Some(Box::new(
                    move |tool_name: &str, _args_json: &str, result: &str| {
                        eb.on_tool_complete("tool-id", tool_name, result);
                    },
                ))
            },
            stream_delta: {
                let eb = Arc::clone(&event_bus);
                Some(Box::new(move |text: &str| {
                    eb.on_stream_delta(text);
                }))
            },
            reasoning: {
                let eb = Arc::clone(&event_bus);
                Some(Box::new(move |text: &str| {
                    eb.on_reasoning(text);
                }))
            },
            ..Default::default()
        };

        self.agent = Some(Arc::new(tokio::sync::Mutex::new(Agent::new(AgentConfig {
            system_prompt: assembled.prompt,
            transport,
            tools: std::sync::Arc::clone(&self.tools),
            skills_dirs: vec![],
            max_iterations: self.config.max_iterations.unwrap_or(50),
            max_messages: self.config.context.max_messages.unwrap_or(100),
            context_config: oben_agent::CompactCofig::default(),
            fallback_models: vec![],
            callbacks,
            concurrent_dispatch_config: oben_agent::ConcurrentDispatchConfig::default(),
            nudge_config: None,
        })?)));

        Ok(())
    }

    pub fn begin_turn(&self) {
        self.event_bus.begin_turn();
    }

    pub fn finalize_turn(&self, outcome: &str) {
        self.event_bus.on_turn_completed(outcome);
    }

    pub async fn create_chat_panel(&mut self) -> Result<()> {
        self.panels.insert(
            PanelId::Chat,
            Box::new(ChatPanel::new(None, None)),
        );
        Ok(()) 
    }

    pub async fn create_sessions_panel(&mut self) -> Result<()> {
        self.panels
            .insert(PanelId::Sessions, Box::new(SessionsPanel::new_empty()));
        Ok(())
    }

    pub async fn handle_key(&mut self, key: crossterm::event::KeyEvent) {
        match key.code {
            crossterm::event::KeyCode::Char('c') if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => {
                if self.active_panel == PanelId::Chat
                    && self.get_chat().map(|cp| cp.message_state.selection_start.is_some()).unwrap_or(false)
                {
                    if let Some(chat) = self.get_chat_mut() {
                        chat.copy_selection_to_clipboard();
                    }
                    return;
                }
                self.running = false;
                return;
            }
            crossterm::event::KeyCode::Char('1') if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => {
                self.activate_panel(PanelId::Chat).await;
                return;
            }
            crossterm::event::KeyCode::Char('2') if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => {
                self.activate_panel(PanelId::Sessions).await;
                return;
            }
            crossterm::event::KeyCode::Char('3') if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => {
                self.activate_panel(PanelId::Config).await;
                return;
            }
            crossterm::event::KeyCode::Char('4') if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => {
                self.activate_panel(PanelId::Setup).await;
                return;
            }
            crossterm::event::KeyCode::F(1) => {
                self.activate_panel(PanelId::Chat).await;
                return;
            }
            crossterm::event::KeyCode::F(2) => {
                self.activate_panel(PanelId::Sessions).await;
                return;
            }
            crossterm::event::KeyCode::F(3) => {
                self.activate_panel(PanelId::Config).await;
                return;
            }
            crossterm::event::KeyCode::F(4) => {
                self.activate_panel(PanelId::Setup).await;
                return;
            }
            crossterm::event::KeyCode::Tab => {
                let n = 4usize;
                let next_idx = match self.active_panel {
                    PanelId::Chat => 0,
                    PanelId::Sessions => 1,
                    PanelId::Config => 2,
                    PanelId::Setup => 3,
                };
                let next = match (next_idx + 1) % n {
                    0 => PanelId::Chat,
                    1 => PanelId::Sessions,
                    2 => PanelId::Config,
                    3 => PanelId::Setup,
                    _ => unreachable!(),
                };
                self.activate_panel(next).await;
                return;
            }
            crossterm::event::KeyCode::Char('t') if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => {
                if let Some(chat) = self.get_chat_mut() {
                    chat.cycle_theme();
                }
            }
            _ => {}
        }
        if let Some(_panel) = self.panels.get_mut(&self.active_panel) {
            let panel_id = self.active_panel;
            if let Some(boxed_panel) = self.panels.remove(&panel_id) {
                let mut panel = boxed_panel;
                panel.handle_key(self, key);
                self.panels.insert(panel_id, panel);
            }
        }
    }

    pub fn create_config_panel(&mut self) {
        let yaml = serde_yaml::to_string(&self.config).unwrap_or_default();
        self.panels
            .insert(PanelId::Config, Box::new(ConfigPanel::new(yaml)));
    }

    pub fn create_setup_panel(&mut self) {
        let mut panel = SetupPanel::new();
        panel.set_config(AppConfig {
            model: self.config.model.clone(),
            temperature: self.config.temperature,
            max_tokens: self.config.max_tokens,
            max_iterations: self.config.max_iterations,
            tools: self.config.tools.clone(),
            skills: self.config.skills.clone(),
            gateway: self.config.gateway.clone(),
            display: self.config.display.clone(),
            context: self.config.context.clone(),
            providers: self.config.providers.clone(),
            custom_providers: self.config.custom_providers.clone(),
        });
        self.panels.insert(PanelId::Setup, Box::new(panel));
    }

    pub async fn init_panels(&mut self) -> Result<()> {
        self.create_chat_panel().await?;
        self.activate_panel(PanelId::Chat).await;
        self.create_sessions_panel().await?;
        self.create_config_panel();
        self.create_setup_panel();
        Ok(())
    }
}