/// Central messaging routing service.
///
/// Dispatcher manages per-session messaging channels and routes incoming
/// messages from platform adapters to the correct coordinator task.
use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{mpsc, Mutex};
use tracing::{info, warn};

use crate::coordinator::GatewayCoordinator;
use crate::platform::IncomingMessage;
use crate::router::ResponseRouter;

use oben_agent::{Agent, AgentBuilder};
use oben_config::AppConfig;
use oben_tools::ToolRegistry;

/// A user's message channel wrapping the sender.
pub struct UserChannel {
    sender: mpsc::Sender<IncomingMessage>,
}

/// Central dispatch service — routes incoming platform messages to coordinator tasks.
pub struct Dispatcher {
    app_config: Arc<AppConfig>,
    tools: Arc<ToolRegistry>,
    session_map: Mutex<HashMap<String, UserChannel>>, // session_key → sender
    response_router: Arc<ResponseRouter>,
}

impl Dispatcher {
    /// Create a new Dispatcher with the given configuration and tool registry.
    pub fn new(
        app_config: AppConfig,
        tools: Arc<ToolRegistry>,
        response_router: Arc<ResponseRouter>,
    ) -> Self {
        Self {
            app_config: Arc::new(app_config),
            tools,
            session_map: Mutex::new(HashMap::new()),
            response_router,
        }
    }

    /// Route an incoming message to the correct coordinator task.
    ///
    /// Computes a session key from the platform, user_id, and thread_id.
    /// If the session already exists, sends the message through the existing channel.
    /// Otherwise, creates a new channel, registers the sender, and spawns a
    /// coordinator task to process the message stream.
    pub async fn dispatch(&self, msg: IncomingMessage) -> Result<(), String> {
        let session_key = format!(
            "{}/{}/{}",
            msg.platform,
            msg.user_id,
            msg.thread_id.as_deref().unwrap_or("global")
        );

        let mut session_map = self.session_map.lock().await;

        if let Some(channel) = session_map.get(&session_key) {
            // Session already exists — try to send via the established channel
            match channel.sender.try_send(msg) {
                Ok(()) => Ok(()),
                Err(e) => {
                    warn!(
                        "Session channel full — dropping message for session {}",
                        session_key
                    );
                    Err(format!("Session channel full: {e}"))
                }
            }
        } else {
            // Create a new channel and spawn a coordinator task
            let (tx, rx) = mpsc::channel(64);
            
            if let Err(e) = tx.try_send(msg.clone()) {
                warn!(session_key = %session_key, error = %e, "Failed to send message to new session");
                return Err(format!("Failed to send to new session: {e}"));
            }
            
            session_map.insert(
                session_key.clone(),
                UserChannel { sender: tx },
            );
            
            self.spawn_coordinator_task(
                session_key,
                rx,
                msg.platform.clone(),
                msg.user_id.clone(),
                msg.thread_id.clone(),
            );
            Ok(())
        }
    }

    /// Spawn a coordinator task that runs the agent conversation loop for a session.
    ///
    /// Creates a GatewayCoordinator and Agent, then runs them together.
    /// Also starts a response routing loop to send agent replies back to the platform.
    fn spawn_coordinator_task(
        &self,
        session_key: String,
        msg_rx: mpsc::Receiver<IncomingMessage>,
        platform: String,
        user_id: String,
        thread_id: Option<String>,
    ) -> tokio::task::JoinHandle<()> {
        let app_config = self.app_config.clone();
        let tools = self.tools.clone();
        let response_router = self.response_router.clone();

        tokio::spawn(async move {
            info!(session_key = %session_key, "Coordinator task started");

            let (response_tx, mut response_rx) = mpsc::channel::<crate::coordinator::ResponseMessage>(128);

            // Concurrently route responses back to the platform
            let response_router_task = {
                let response_router = response_router.clone();
                tokio::spawn(async move {
                    while let Some(resp) = response_rx.recv().await {
                        info!(reply_len = resp.content.len(), "Routing response to platform");
                        if let Err(e) = response_router.dispatch_response(resp).await {
                            warn!(error = %e, "Failed to dispatch response to platform");
                        }
                    }
                })
            };

            // Build the coordinator with the response channel
            let coordinator = GatewayCoordinator::new(
                msg_rx,
                Arc::new(std::sync::Mutex::new(Some(response_tx))),
                platform,
                user_id,
                thread_id,
            );

            // Build the agent with defaults
            let system_prompt = oben_config::defaults::default_system_prompt();
            let agent = match AgentBuilder::new()
                .with_config((*app_config).clone())
                .with_system_prompt(system_prompt)
                .with_tools(tools)
                .build()
                .await
            {
                Ok(agent) => agent,
                Err(e) => {
                    warn!(error = %e, "Failed to create agent for session");
                    return;
                }
            };

            let agent = Arc::new(Mutex::new(agent));
            let result = Agent::run(agent, coordinator).await;

            // coordinator ended — response_tx is dropped, response_rx loop will exit
            if let Err(e) = &result {
                tracing::error!(error = %e, session_key = %session_key, "Agent run ended with error");
            } else {
                info!(session_key = %session_key, "Agent conversation loop completed");
            }
            // Wait for response router to finish
            if let Err(e) = response_router_task.await {
                warn!(session_key = %session_key, "response_router task join failed: {e}");
            }
        })
    }
}
