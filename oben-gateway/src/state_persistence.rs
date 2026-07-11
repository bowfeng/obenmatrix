//! Gateway state persistence - persists gateway state to disk
//!
//! GatewayState tracks the current state of the gateway including:
//! - Platform statuses
//! - Message statistics
//! - Last activity timestamps
//! - Configuration state

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

use crate::platform::PlatformStatus;

/// Gateway state that persists across restarts
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct GatewayState {
    /// Map of platform name to platform status
    pub platforms: HashMap<String, PlatformStatus>,
    /// Total messages processed
    pub messages_processed: u64,
    /// Total messages failed
    pub messages_failed: u64,
    /// Last activity timestamp (Unix epoch)
    pub last_activity: u64,
    /// Gateway startup timestamp
    pub startup_time: u64,
}

impl GatewayState {
    /// Create a new gateway state
    pub fn new() -> Self {
        Self {
            platforms: HashMap::new(),
            messages_processed: 0,
            messages_failed: 0,
            last_activity: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            startup_time: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        }
    }

    /// Update the last activity timestamp
    pub fn record_activity(&mut self) {
        self.last_activity = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
    }

    /// Increment the message processed counter
    pub fn increment_processed(&mut self) {
        self.messages_processed += 1;
    }

    /// Increment the message failed counter
    pub fn increment_failed(&mut self) {
        self.messages_failed += 1;
    }

    /// Set the status for a platform
    pub fn set_platform_status(&mut self, platform: String, status: PlatformStatus) {
        self.platforms.insert(platform, status);
        self.record_activity();
    }

    /// Get the status of a platform
    pub fn get_platform_status(&self, platform: &str) -> Option<&PlatformStatus> {
        self.platforms.get(platform)
    }

    /// Serialize state to JSON string
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self).map_err(|e| anyhow::anyhow!(e))
    }

    /// Deserialize state from JSON string
    pub fn from_json(json: &str) -> Result<Self> {
        serde_json::from_str(json).map_err(|e| anyhow::anyhow!(e))
    }

    /// Save state to a file
    pub fn save_to_file<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let json = self.to_json()?;
        std::fs::write(path, json).map_err(|e| anyhow::anyhow!(e))
    }

    /// Load state from a file
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let json = std::fs::read_to_string(path).map_err(|e| anyhow::anyhow!(e))?;
        Self::from_json(&json)
    }
}

/// GatewayStatePersister handles persistence operations
pub struct GatewayStatePersister {
    state: GatewayState,
    base_path: String,
}

impl GatewayStatePersister {
    /// Create a new persister
    pub fn new(base_path: &str) -> Self {
        Self {
            state: GatewayState::new(),
            base_path: base_path.to_string(),
        }
    }

    /// Get the current state
    pub fn state(&self) -> &GatewayState {
        &self.state
    }

    /// Get mutable access to state
    pub fn state_mut(&mut self) -> &mut GatewayState {
        &mut self.state
    }

    /// Load state from file if it exists
    pub fn load(&mut self) -> Result<()> {
        let state_path = format!("{}/state.json", self.base_path);
        if Path::new(&state_path).exists() {
            self.state = GatewayState::load_from_file(&state_path)?;
        }
        Ok(())
    }

    /// Save state to file
    pub fn save(&self) -> Result<()> {
        let state_path = format!("{}/state.json", self.base_path);
        self.state.save_to_file(&state_path)
    }

    /// Update a platform status and save
    pub fn update_platform_status(&mut self, platform: String, status: PlatformStatus) -> Result<()> {
        self.state.set_platform_status(platform, status);
        self.save()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gateway_state_new() {
        let state = GatewayState::new();
        assert_eq!(state.messages_processed, 0);
        assert_eq!(state.messages_failed, 0);
        assert!(state.platforms.is_empty());
        assert!(state.last_activity > 0);
        assert!(state.startup_time > 0);
    }

    #[test]
    fn test_gateway_state_record_activity() {
        let mut state = GatewayState::new();
        let before = state.last_activity;
        // Wait at least 1 second to ensure timestamp changes
        std::thread::sleep(std::time::Duration::from_secs(1));
        state.record_activity();
        assert!(state.last_activity >= before);
    }

    #[test]
    fn test_gateway_state_increment_counters() {
        let mut state = GatewayState::new();
        state.increment_processed();
        state.increment_processed();
        state.increment_failed();
        assert_eq!(state.messages_processed, 2);
        assert_eq!(state.messages_failed, 1);
    }

    #[test]
    fn test_gateway_state_platform_status() {
        let mut state = GatewayState::new();
        state.set_platform_status(
            "telegram".to_string(),
            PlatformStatus::Running,
        );
        assert_eq!(
            state.get_platform_status("telegram"),
            Some(&PlatformStatus::Running)
        );
        assert_eq!(state.get_platform_status("discord"), None);
    }

    #[test]
    fn test_gateway_state_serialization() {
        let mut state = GatewayState::new();
        state.set_platform_status("telegram".to_string(), PlatformStatus::Running);
        state.increment_processed();

        let json = state.to_json().unwrap();
        let restored = GatewayState::from_json(&json).unwrap();

        assert_eq!(restored.messages_processed, state.messages_processed);
        assert_eq!(
            restored.platforms.get("telegram"),
            state.platforms.get("telegram")
        );
    }

    #[test]
    fn test_gateway_state_persistence() -> Result<()> {
        use std::fs;
        use std::path::Path;

        let temp_dir = std::env::temp_dir();
        let state_path = temp_dir.join("oben-gateway-test-state.json");

        let mut state = GatewayState::new();
        state.set_platform_status("telegram".to_string(), PlatformStatus::Running);
        state.save_to_file(&state_path)?;

        let loaded = GatewayState::load_from_file(&state_path)?;
        assert_eq!(loaded.platforms.get("telegram"), state.platforms.get("telegram"));

        // Clean up
        if Path::exists(&state_path) {
            fs::remove_file(&state_path)?;
        }
        Ok(())
    }

    #[test]
    fn test_gateway_state_persister() -> Result<()> {
        use std::fs;

        let temp_dir = std::env::temp_dir();
        let base_path = temp_dir.join("oben-gateway-test-persister-12345");
        let base_path_str = base_path.to_str().unwrap();

        // Create the directory if it doesn't exist
        std::fs::create_dir_all(&base_path).ok();

        let mut persister = GatewayStatePersister::new(base_path_str);
        persister.state_mut().set_platform_status(
            "discord".to_string(),
            PlatformStatus::Running,
        );
        persister.save()?;

        // Load a new persister
        let mut new_persister = GatewayStatePersister::new(base_path_str);
        new_persister.load()?;
        assert_eq!(
            new_persister.state().platforms.get("discord"),
            persister.state().platforms.get("discord")
        );

        // Clean up
        let state_path = base_path.join("state.json");
        if state_path.exists() {
            fs::remove_file(&state_path)?;
        }
        Ok(())
    }
}
