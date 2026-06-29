/// The response message sent from a coordinator back to the dispatcher
/// for routing to the appropriate platform adapter.
#[derive(Debug, Clone)]
pub struct ResponseMessage {
    /// Unique session key identifying the conversation (platform:user_id/thread_id).
    pub session_key: String,
    /// The response content to forward to the platform.
    pub content: String,
}

/// GatewayCoordinator — implements ConversationCoordinator for the multi-platform
/// message routing system. Blocks on the incoming message channel for each turn,
/// then sends the agent's response back to the dispatcher via the response channel.
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::platform::IncomingMessage;

use oben_agent::coordinator::{ConversationCoordinator, ConversationResult};

/// Core coordinator that bridges the agent's conversation loop with platform
/// message routing. Each dispatcher session gets one GatewayCoordinator, which
/// waits on a message channel for user input and sends responses back.
pub struct GatewayCoordinator {
    msg_rx: mpsc::Receiver<IncomingMessage>,
    response_tx: Arc<std::sync::Mutex<Option<mpsc::Sender<ResponseMessage>>>>,
    /// Stored for routing responses back to the correct platform session.
    platform: String,
    user_id: String,
    thread_id: Option<String>,
}

impl GatewayCoordinator {
    /// Create a new GatewayCoordinator with message routing metadata.
    ///
    /// The platform, user_id, and thread_id are used to construct the session_key
    /// that routes responses back to the correct platform adapter.
    pub fn new(
        msg_rx: mpsc::Receiver<IncomingMessage>,
        response_tx: Arc<std::sync::Mutex<Option<mpsc::Sender<ResponseMessage>>>>,
        platform: String,
        user_id: String,
        thread_id: Option<String>,
    ) -> Self {
        Self {
            msg_rx,
            response_tx,
            platform,
            user_id,
            thread_id,
        }
    }
}

#[async_trait]
impl ConversationCoordinator for GatewayCoordinator {
    /// No-op. Platform coordinators don't need pre-turn hooks.
    fn on_loop_start(&mut self) {}

    /// Wait for the next incoming message. Returns None when the channel is closed
    /// (user disconnected or session ended).
    async fn next_turn(&mut self) -> Option<String> {
        self.msg_rx.recv().await.map(|msg| msg.content)
    }

    /// Send the response back to the dispatcher for platform routing.
    /// Returns true to continue the conversation loop, false to exit.
    fn on_turn_complete(&mut self, response: &str, _msg_count: usize, success: bool) -> bool {
        if success {
            let thread = self.thread_id.as_deref().unwrap_or("global");
            let session_key = format!("{}/{}/{}", self.platform, self.user_id, thread);
            let response_msg = ResponseMessage {
                session_key,
                content: response.to_string(),
            };

            // Lock the mutex and try to send the response back to dispatcher
            if let Some(tx_guard) = self.response_tx.lock().unwrap().as_ref() {
                let _ = tx_guard.try_send(response_msg);
            }
        }

        success // Continue loop on success, exit on failure
    }

    /// Log the conversation outcome when the loop ends.
    fn on_loop_end(&mut self, outcome: &ConversationResult) {
        match outcome {
            ConversationResult::Exit => {
                tracing::info!("Gateway conversation loop ended (user exit)");
            }
            ConversationResult::BudgetExhausted => {
                tracing::warn!("Gateway conversation loop ended (budget exhausted)");
            }
            ConversationResult::Interrupted => {
                tracing::info!("Gateway conversation loop ended (interrupted)");
            }
            ConversationResult::GoalDone => {
                tracing::info!("Gateway conversation loop ended (goal done)");
            }
            ConversationResult::Error(ref msg) => {
                tracing::error!("Gateway conversation loop ended (error: {msg})");
            }
        }
    }
}
