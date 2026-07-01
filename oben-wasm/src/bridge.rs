use std::sync::{Arc, Mutex};

use anyhow::Result;
use async_trait::async_trait;

use oben_platform_sdk::{OutgoingMessage, PlatformAdapter, PlatformStatus, PlatformInfo};

use crate::loader::PlatformPluginConfig;
use crate::runtime::PreparedComponent;

/// Internal adapter state managed under a mutex.
#[derive(Clone, Copy, PartialEq, Eq)]
enum AdapterState {
    Stopped,
    Starting,
    Running,
}

/// Bridge between WASM component and PlatformAdapter trait.
#[allow(dead_code)]
pub struct WasmPlatformAdapter {
    name: String,
    config: PlatformPluginConfig,
    component: Arc<PreparedComponent>,
    state: Arc<Mutex<AdapterState>>,
    started_at: Arc<Mutex<Option<String>>>,
}

impl WasmPlatformAdapter {
    pub fn new(
        name: String,
        config: PlatformPluginConfig,
        component: Arc<PreparedComponent>,
    ) -> Self {
        Self {
            name,
            config,
            component,
            state: Arc::new(Mutex::new(AdapterState::Stopped)),
            started_at: Arc::new(Mutex::new(None)),
        }
    }

    /// Get current platform info for health introspection.
    pub fn info(&self) -> PlatformInfo {
        let state = self.state.lock().unwrap();
        let started_at = self.started_at.lock().unwrap().clone();
        let status = match *state {
            AdapterState::Stopped => PlatformStatus::Idle,
            AdapterState::Starting => PlatformStatus::Connecting,
            AdapterState::Running => PlatformStatus::Running,
        };
        PlatformInfo {
            name: self.name.clone(),
            status,
            started_at,
            error: None,
        }
    }
}

#[async_trait]
impl PlatformAdapter for WasmPlatformAdapter {
    fn name(&self) -> &str {
        &self.name
    }

    async fn listen(&mut self) -> Result<()> {
        {
            let mut state = self.state.lock().unwrap();
            *state = AdapterState::Starting;
        }
        {
            let mut started_at = self.started_at.lock().unwrap();
            *started_at = Some(chrono::Utc::now().to_rfc3339());
        }

        tracing::info!(name = %self.name, "WASM adapter listen started");

        {
            let mut state = self.state.lock().unwrap();
            *state = AdapterState::Running;
        }

        // Keep the adapter alive until explicitly stopped.
        loop {
            {
                let state = self.state.lock().unwrap();
                if *state == AdapterState::Stopped {
                    break;
                }
            }
            tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
        }

        Ok(())
    }

    async fn stop(&mut self) {
        *self.state.lock().unwrap() = AdapterState::Stopped;
        tracing::info!(name = %self.name, "WASM adapter stopped");
    }

    async fn send(&self, msg: OutgoingMessage) -> Result<()> {
        // TODO: Send message through WASM host interface
        tracing::info!(
            name = %self.name,
            to = %msg.user_id,
            "Queueing message for WASM adapter send"
        );
        Ok(())
    }

    async fn health_check(&self) -> bool {
        let state = self.state.lock().unwrap();
        *state == AdapterState::Running
    }
}
