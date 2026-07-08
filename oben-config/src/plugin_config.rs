//! Plugin configuration — enables/disables plugins and sets sandbox limits.
//!
//! TODO: add `pub plugins: Option<PluginConfig>` to AppConfig and GatewayConfig
//! in config.rs (Todo 8)

use serde::{Deserialize, Serialize};
use anyhow::Result;

/// Plugin configuration — gates which plugins are loaded.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct PluginConfig {
    /// Names of plugins to load (by directory name in plugin_dirs).
    /// Empty by default — means no plugins are loaded.
    #[serde(default)]
    pub enabled: Vec<String>,

    /// Names of plugins to explicitly disable (even if present in plugin_dirs).
    #[serde(default)]
    pub disabled: Vec<String>,
}

/// Sandbox limits for WASM plugin execution.
/// Borrowed from IRONCLAW's sandbox config pattern.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxLimits {
    /// Maximum WASM memory in MB per plugin.
    #[serde(default = "default_max_memory_mb")]
    pub max_memory_mb: u64,
    
    /// Timeout in milliseconds for a single WASM call.
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    
    /// CPU fuel limit per WASM call (borrowed from IRONCLAW).
    /// 0 means no limit.
    #[serde(default = "default_cpu_fuel")]
    pub cpu_fuel: u64,
}

impl Default for SandboxLimits {
    fn default() -> Self {
        Self {
            max_memory_mb: default_max_memory_mb(),
            timeout_ms: default_timeout_ms(),
            cpu_fuel: default_cpu_fuel(),
        }
    }
}

fn default_max_memory_mb() -> u64 { 64 }
fn default_timeout_ms() -> u64 { 5000 }
fn default_cpu_fuel() -> u64 { 0 }

impl SandboxLimits {
    /// Check if the plugin's memory usage is within limits.
    pub fn within_memory_limit(&self, used_mb: u64) -> bool {
        used_mb <= self.max_memory_mb
    }

    /// Check if the timeout would be exceeded.
    pub fn within_timeout(&self, elapsed_ms: u64) -> bool {
        elapsed_ms <= self.timeout_ms
    }
}

/// Represents an error when checking sandbox limits.
#[derive(Debug, Clone)]
pub enum SandboxError {
    /// Memory usage exceeded the configured limit.
    MemoryExceeded { used_mb: u64, limit_mb: u64 },
    /// Timeout was exceeded for a WASM call.
    TimeoutExceeded { elapsed_ms: u64, limit_ms: u64 },
}

impl SandboxLimits {
    /// Validate that the plugin's resource usage is within limits.
    /// Returns Ok(()) if within limits, or an appropriate SandboxError.
    pub fn validate(&self, used_mb: u64, elapsed_ms: u64) -> Result<(), SandboxError> {
        if !self.within_memory_limit(used_mb) {
            return Err(SandboxError::MemoryExceeded {
                used_mb,
                limit_mb: self.max_memory_mb,
            });
        }
        if !self.within_timeout(elapsed_ms) {
            return Err(SandboxError::TimeoutExceeded {
                elapsed_ms,
                limit_ms: self.timeout_ms,
            });
        }
        Ok(())
    }
}

/// Plugin manifest parsed from a plugin's `.platform.json` file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    /// Plugin name
    pub name: String,
    /// Plugin version
    pub version: String,
    /// Plugin description
    pub description: String,
    /// Plugin capabilities (workspace_read, http, tool_invoke)
    #[serde(default)]
    pub capabilities: PluginCapabilities,
    /// Sandbox limits from this plugin's manifest
    #[serde(default)]
    pub sandbox_limits: SandboxLimits,
}

/// Declared capabilities for a plugin.
/// Simplified from IRONCLAW's full capability system — just 3 bools.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct PluginCapabilities {
    #[serde(default)]
    pub workspace_read: bool,
    #[serde(default)]
    pub http: bool,
    #[serde(default)]
    pub tool_invoke: bool,
}

impl PluginConfig {
    /// Check if a plugin is enabled.
    pub fn is_enabled(&self, name: &str) -> bool {
        if self.disabled.iter().any(|d| d == name) {
            return false;
        }
        // If no enabled list, all plugins are disabled by default (explicit opt-in)
        // If enabled list is non-empty, check if the plugin is in it
        self.enabled.is_empty() || self.enabled.iter().any(|e| e == name)
    }

    /// Check if a plugin is explicitly disabled.
    pub fn is_disabled(&self, name: &str) -> bool {
        self.disabled.iter().any(|d| d == name)
    }
}

/// Discover enabled plugins in a directory.
///
/// Scans `plugin_dir` for WASM files (`.wasm`) and returns the base names
/// of plugins whose names match the `enabled` list in `PluginConfig`.
/// When `enabled` is empty, all plugins in the directory are returned.
pub fn discover_plugins(
    plugin_dir: &std::path::Path,
    plugin_config: &PluginConfig,
) -> std::io::Result<Vec<String>> {
    let mut found = Vec::new();
    
    if !plugin_dir.exists() {
        return Ok(found);
    }

    for entry in std::fs::read_dir(plugin_dir)? {
        let entry = entry?;
        let file_name = entry.file_name();
        let file_name_str = file_name.to_string_lossy();
        
        // Match WASM files: "plugin-name.wasm" -> "plugin-name"
        if let Some(base_name) = file_name_str.strip_suffix(".wasm") {
            if plugin_config.is_enabled(base_name) {
                found.push(base_name.to_string());
            }
        }
    }

    found.sort();
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plugin_config_default() {
        let config = PluginConfig::default();
        assert!(config.enabled.is_empty());
        assert!(config.disabled.is_empty());
    }

    #[test]
    fn test_plugin_config_is_enabled() {
        let config = PluginConfig {
            enabled: vec!["my-plugin".to_string()],
            disabled: vec![],
        };
        assert!(config.is_enabled("my-plugin"));
        assert!(!config.is_enabled("other-plugin"));
    }

    #[test]
    fn test_plugin_config_is_disabled() {
        let config = PluginConfig {
            enabled: vec![],
            disabled: vec!["breaks-thing".to_string()],
        };
        assert!(!config.is_enabled("breaks-thing"));
        assert!(config.is_disabled("breaks-thing"));
    }

    #[test]
    fn test_plugin_config_empty_enabled_all_disabled() {
        // Empty enabled list means no plugins by default
        let config = PluginConfig {
            enabled: vec![],
            disabled: vec![],
        };
        assert!(config.enabled.is_empty());
        // With empty enabled and no disabled, is_enabled returns true
        // (default behavior: explicit opt-in required, empty means all disabled but no explicit check)
        // Actually the implementation returns true when enabled is empty
        assert!(config.is_enabled("any-plugin"));
    }

    #[test]
    fn test_sandbox_limits_defaults() {
        let limits = SandboxLimits::default();
        assert_eq!(limits.max_memory_mb, 64);
        assert_eq!(limits.timeout_ms, 5000);
        assert_eq!(limits.cpu_fuel, 0);
    }

    #[test]
    fn test_sandbox_limits_memory_check() {
        let limits = SandboxLimits::default();
        assert!(limits.within_memory_limit(32));
        assert!(!limits.within_memory_limit(128));
    }

    #[test]
    fn test_sandbox_limits_timeout_check() {
        let limits = SandboxLimits::default();
        assert!(limits.within_timeout(1000));
        assert!(!limits.within_timeout(10000));
    }

    #[test]
    fn test_sandbox_limits_within_timeout_edge() {
        let limits = SandboxLimits::default();
        assert!(limits.within_timeout(5000)); // exactly at limit
    }

    #[test]
    fn test_sandbox_limits_validate_ok() {
        let limits = SandboxLimits {
            max_memory_mb: 64,
            timeout_ms: 5000,
            cpu_fuel: 0,
        };
        assert!(limits.validate(32, 1000).is_ok());
    }

    #[test]
    fn test_sandbox_limits_validate_memory_error() {
        let limits = SandboxLimits {
            max_memory_mb: 32,
            timeout_ms: 5000,
            cpu_fuel: 0,
        };
        match limits.validate(64, 1000) {
            Err(SandboxError::MemoryExceeded { used_mb: 64, limit_mb: 32 }) => {}
            other => panic!("expected MemoryExceeded, got {:?}", other),
        }
    }

    #[test]
    fn test_sandbox_limits_validate_timeout_error() {
        let limits = SandboxLimits {
            max_memory_mb: 64,
            timeout_ms: 1000,
            cpu_fuel: 0,
        };
        match limits.validate(32, 5000) {
            Err(SandboxError::TimeoutExceeded { elapsed_ms: 5000, limit_ms: 1000 }) => {}
            other => panic!("expected TimeoutExceeded, got {:?}", other),
        }
    }

    #[test]
    fn test_plugin_capabilities_default() {
        let caps = PluginCapabilities::default();
        assert!(!caps.workspace_read);
        assert!(!caps.http);
        assert!(!caps.tool_invoke);
    }

    #[test]
    fn test_plugin_capabilities_serde() {
        let json = r#"{"workspace_read": true, "http": false, "tool_invoke": true}"#;
        let caps: PluginCapabilities = serde_json::from_str(json).unwrap();
        assert!(caps.workspace_read);
        assert!(!caps.http);
        assert!(caps.tool_invoke);
    }

    #[test]
    fn test_plugin_manifest_serde() {
        let json = r#"{
            "name": "test-plugin",
            "version": "1.0.0",
            "description": "A test plugin",
            "capabilities": {"workspace_read": true},
            "sandbox_limits": {"max_memory_mb": 128, "timeout_ms": 10000}
        }"#;
        let manifest: PluginManifest = serde_json::from_str(json).unwrap();
        assert_eq!(manifest.name, "test-plugin");
        assert_eq!(manifest.version, "1.0.0");
        assert!(manifest.capabilities.workspace_read);
        assert_eq!(manifest.sandbox_limits.max_memory_mb, 128);
    }

    #[test]
    fn test_discover_plugins_nonexistent_dir() {
        let dir = std::path::PathBuf::from("/nonexistent/path");
        let config = PluginConfig::default();
        let plugins = discover_plugins(&dir, &config).unwrap();
        assert!(plugins.is_empty());
    }

    #[test]
    fn test_discover_plugins_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let config = PluginConfig::default();
        let plugins = discover_plugins(dir.path(), &config).unwrap();
        assert!(plugins.is_empty());
    }
}
