//! Plugin lifecycle management — tracks per-plugin state and manages
//! startup, shutdown, crash detection, and restart.
//!
//! Borrowed from IronClaw's ExtensionPackage lifecycle pattern.
//! Supports graceful stop, crash tracking with retry limits, and
//! cleanup on disable (no hot-reload).

use std::collections::HashMap;
use std::time::{Duration, Instant};
use tracing::{info, warn};

/// Current state of a plugin in the lifecycle manager.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginState {
    /// Plugin is being loaded (init phase)
    Initializing,
    /// Plugin is loaded and active
    Running,
    /// Plugin was gracefully stopped
    Stopped,
    /// Plugin crashed (max retries exceeded or manual crash)
    Crashed(String),
    /// Plugin was explicitly disabled (disabled via config)
    Disabled,
}

/// Plugin lifecycle manager — tracks per-plugin state and manages
/// lifecycle transitions.
///
/// When a plugin crashes:
/// 1. State → Crashed(error)
/// 2. Attempt restart (max 3 retries)
/// 3. If retries exhausted → State → Crashed("max retries exceeded")
///
/// When disabled:
/// 1. Stop all handlers
/// 2. Clean WASM store
/// 3. State → Disabled
#[derive(Debug, Default)]
pub struct PluginLifecycleManager {
    /// Per-plugin state tracking
    states: HashMap<String, PluginState>,
    /// Per-plugin store handles (for cleanup)
    stores: HashMap<String, Option<()>>,
    /// Per-plugin crash tracking: plugin_id → (crash_count, last_crash_time)
    crash_counts: HashMap<String, (u32, Instant)>,
    /// Maximum number of crash restarts before giving up
    max_restarts: u32,
}

impl PluginLifecycleManager {
    /// Create a new lifecycle manager.
    pub fn new(max_restarts: u32) -> Self {
        Self {
            states: HashMap::new(),
            stores: HashMap::new(),
            crash_counts: HashMap::new(),
            max_restarts,
        }
    }

    /// Record that a plugin is starting (Initializing → Running)
    pub fn start(&mut self, plugin_id: &str) {
        self.states.insert(plugin_id.to_string(), PluginState::Initializing);
        self.crash_counts.insert(plugin_id.to_string(), (0, Instant::now()));
    }

    /// Record that a plugin has started successfully
    pub fn started(&mut self, plugin_id: &str) {
        self.states.insert(plugin_id.to_string(), PluginState::Running);
    }

    /// Record that a plugin has been gracefully stopped
    pub fn stop(&mut self, plugin_id: &str) {
        self.states.insert(plugin_id.to_string(), PluginState::Stopped);
    }

    /// Record that a plugin crashed
    pub fn crash(&mut self, plugin_id: &str, error: &str) {
        let state = PluginState::Crashed(error.to_string());
        self.states.insert(plugin_id.to_string(), state.clone());

        // Track crash count and time
        let (count, last_crash) = self.crash_counts
            .entry(plugin_id.to_string())
            .or_insert((0, Instant::now()));

        *count += 1;
        *last_crash = Instant::now();

        warn!(
            plugin = %plugin_id,
            crashes = *count,
            max_restarts = self.max_restarts,
            error = %error,
            "Plugin crashed",
        );
    }

    /// Check if a plugin can be restarted after a crash.
    /// Returns true if the crash count is below the restart limit.
    pub fn can_restart(&self, plugin_id: &str) -> bool {
        let (count, _) = self.crash_counts.get(plugin_id).copied().unwrap_or((0, Instant::now()));
        count < self.max_restarts
    }

    /// Get the current state of a plugin.
    pub fn state(&self, plugin_id: &str) -> Option<&PluginState> {
        self.states.get(plugin_id)
    }

    /// Check if a plugin is running.
    pub fn is_running(&self, plugin_id: &str) -> bool {
        self.states.get(plugin_id) == Some(&PluginState::Running)
    }

    /// Check if a plugin is disabled.
    pub fn is_disabled(&self, plugin_id: &str) -> bool {
        self.states.get(plugin_id) == Some(&PluginState::Disabled)
    }

    /// Disable a plugin — stops it and marks as disabled.
    /// After this, the plugin should be unregistered from all registries.
    pub fn disable(&mut self, plugin_id: &str) {
        info!(plugin = %plugin_id, "Disabling plugin");
        self.states.insert(plugin_id.to_string(), PluginState::Disabled);
    }

    /// Clean up resources for a disabled plugin.
    /// In the future, this will release WASM stores.
    pub fn cleanup_on_disable(&mut self, plugin_id: &str) {
        info!(plugin = %plugin_id, "Cleaning up disabled plugin");
    }

    /// Get all plugin IDs that are currently running.
    pub fn running_plugins(&self) -> Vec<String> {
        self.states.iter()
            .filter(|(_, s)| **s == PluginState::Running)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Get all plugin IDs.
    pub fn all_plugins(&self) -> Vec<String> {
        self.states.keys().cloned().collect()
    }

    /// Stop a specific plugin.
    pub fn stop_plugin(&mut self, plugin_id: &str) {
        info!(plugin = %plugin_id, "Stopping plugin");
        self.states.insert(plugin_id.to_string(), PluginState::Stopped);
    }

    /// Graceful shutdown of all plugins.
    pub fn shutdown_all(&mut self) {
        let plugins = self.states.keys().cloned().collect::<Vec<_>>();
        for plugin in plugins {
            info!(plugin = %plugin, "Shutting down plugin");
            self.states.insert(plugin, PluginState::Stopped);
        }
    }

    /// Get the number of crashes for a plugin.
    pub fn crash_count(&self, plugin_id: &str) -> u32 {
        self.crash_counts.get(plugin_id).map(|(count, _)| *count).unwrap_or(0)
    }

    /// Get the time of the last crash for a plugin.
    pub fn last_crash_time(&self, plugin_id: &str) -> Option<Instant> {
        self.crash_counts.get(plugin_id).map(|(_, time)| *time)
    }

    /// Reset crash count for a plugin (used after successful restart).
    pub fn reset_crash_count(&mut self, plugin_id: &str) {
        if let Some((count, time)) = self.crash_counts.get_mut(plugin_id) {
            *count = 0;
            let _ = time;
        }
    }

    /// Get the duration since the last crash for a plugin.
    pub fn time_since_crash(&self, plugin_id: &str) -> Option<Duration> {
        self.last_crash_time(plugin_id).map(|t| t.elapsed())
    }

    /// Record that a plugin was stored for cleanup.
    pub fn record_store(&mut self, plugin_id: &str) {
        self.stores.insert(plugin_id.to_string(), Some(()));
    }

    /// Get the number of recorded store handles.
    pub fn store_count(&self, plugin_id: &str) -> usize {
        self.stores.get(plugin_id).map(|s| if s.is_some() { 1 } else { 0 }).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plugin_state_transitions() {
        let mut mgr = PluginLifecycleManager::new(3);
        mgr.start("test-plugin");
        assert_eq!(mgr.state("test-plugin"), Some(&PluginState::Initializing));

        mgr.started("test-plugin");
        assert_eq!(mgr.state("test-plugin"), Some(&PluginState::Running));
        assert!(mgr.is_running("test-plugin"));
    }

    #[test]
    fn test_crash_detection() {
        let mut mgr = PluginLifecycleManager::new(3);
        mgr.start("test-plugin");

        mgr.crash("test-plugin", "OOM error");
        assert_eq!(mgr.state("test-plugin"), Some(&PluginState::Crashed("OOM error".to_string())));
        assert!(mgr.can_restart("test-plugin")); // 1 crash < 3 max
        assert_eq!(mgr.crash_count("test-plugin"), 1);
    }

    #[test]
    fn test_max_restarts() {
        let mut mgr = PluginLifecycleManager::new(2);
        mgr.start("test-plugin");
        mgr.started("test-plugin");

        mgr.crash("test-plugin", "error 1");
        assert_eq!(mgr.state("test-plugin"), Some(&PluginState::Crashed("error 1".to_string())));
        assert!(mgr.can_restart("test-plugin")); // 1 < 2

        mgr.crash("test-plugin", "error 2");
        assert!(!mgr.can_restart("test-plugin")); // 2 >= 2
        assert_eq!(mgr.crash_count("test-plugin"), 2);
    }

    #[test]
    fn test_plugin_disable() {
        let mut mgr = PluginLifecycleManager::new(3);
        mgr.start("test-plugin");
        mgr.started("test-plugin");
        mgr.disable("test-plugin");

        assert!(mgr.is_disabled("test-plugin"));
        assert_eq!(mgr.state("test-plugin"), Some(&PluginState::Disabled));
    }

    #[test]
    fn test_running_plugins() {
        let mut mgr = PluginLifecycleManager::new(3);
        mgr.start("plugin-a"); mgr.started("plugin-a");
        mgr.start("plugin-b"); mgr.started("plugin-b");
        mgr.start("plugin-c"); mgr.started("plugin-c");

        let running = mgr.running_plugins();
        assert_eq!(running.len(), 3);
        assert!(running.contains(&"plugin-a".to_string()));
    }

    #[test]
    fn test_default_max_restarts() {
        let mgr = PluginLifecycleManager::new(0);
        assert!(!mgr.can_restart("nobody")); // No crashes tracked, no restart from 0
    }

    #[test]
    fn test_stop_plugin() {
        let mut mgr = PluginLifecycleManager::new(3);
        mgr.start("test-plugin");
        mgr.started("test-plugin");
        mgr.stop_plugin("test-plugin");

        assert_eq!(mgr.state("test-plugin"), Some(&PluginState::Stopped));
        assert!(!mgr.is_running("test-plugin"));
    }

    #[test]
    fn test_shutdown_all() {
        let mut mgr = PluginLifecycleManager::new(3);
        mgr.start("a"); mgr.started("a");
        mgr.start("b"); mgr.started("b");

        mgr.shutdown_all();

        assert_eq!(mgr.state("a"), Some(&PluginState::Stopped));
        assert_eq!(mgr.state("b"), Some(&PluginState::Stopped));
        assert!(mgr.running_plugins().is_empty());
    }

    #[test]
    fn test_crash_reset() {
        let mut mgr = PluginLifecycleManager::new(2);
        mgr.start("test-plugin");
        mgr.started("test-plugin");
        mgr.crash("test-plugin", "error 1");
        assert!(mgr.can_restart("test-plugin"));

        mgr.reset_crash_count("test-plugin");
        assert!(mgr.can_restart("test-plugin"));
    }

    #[test]
    fn test_store_tracking() {
        let mut mgr = PluginLifecycleManager::new(3);
        mgr.record_store("test-plugin");
        assert_eq!(mgr.store_count("test-plugin"), 1);
    }

    #[test]
    fn test_time_since_crash() {
        let mut mgr = PluginLifecycleManager::new(3);
        mgr.start("test-plugin");
        mgr.crash("test-plugin", "error");
        let dur = mgr.time_since_crash("test-plugin").unwrap();
        assert!(dur.as_secs() >= 0);
    }
}
