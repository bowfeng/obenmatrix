//! Gateway diagnostics - diagnostic info for gateway components
//!
//! DiagnosticInfo provides diagnostic information about gateway operation

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::platform::PlatformStatus;

/// Diagnostic information about a platform
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlatformDiagnostic {
    /// Platform name
    pub platform: String,
    /// Current status
    pub status: PlatformStatus,
    /// Start time (Unix epoch)
    pub started_at: Option<u64>,
    /// Last activity (Unix epoch)
    pub last_activity: Option<u64>,
    /// Error message if any
    pub error: Option<String>,
}

impl PlatformDiagnostic {
    /// Create new platform diagnostic
    pub fn new(platform: &str) -> Self {
        Self {
            platform: platform.to_string(),
            status: PlatformStatus::Idle,
            started_at: None,
            last_activity: None,
            error: None,
        }
    }

    /// Set the status
    pub fn with_status(mut self, status: PlatformStatus) -> Self {
        self.status = status;
        self
    }

    /// Set the start time
    pub fn with_start_time(mut self, time: u64) -> Self {
        self.started_at = Some(time);
        self
    }

    /// Set the error
    pub fn with_error(mut self, error: &str) -> Self {
        self.error = Some(error.to_string());
        self
    }
}

/// Gateway diagnostic information
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GatewayDiagnostic {
    /// Uptime in seconds
    pub uptime_seconds: u64,
    /// Start time (Unix epoch)
    pub start_time: u64,
    /// Number of platforms
    pub platform_count: usize,
    /// Platform diagnostics
    pub platforms: Vec<PlatformDiagnostic>,
    /// System memory usage (MB)
    pub memory_mb: Option<u64>,
    /// Total messages processed
    pub total_messages: u64,
    /// Total messages failed
    pub total_failed: u64,
}

impl GatewayDiagnostic {
    /// Create new diagnostic
    pub fn new() -> Self {
        Self {
            uptime_seconds: 0,
            start_time: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            platform_count: 0,
            platforms: Vec::new(),
            memory_mb: None,
            total_messages: 0,
            total_failed: 0,
        }
    }

    /// Update uptime
    pub fn update_uptime(&mut self) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        self.uptime_seconds = now - self.start_time;
    }

    /// Set memory usage
    pub fn with_memory(mut self, mb: u64) -> Self {
        self.memory_mb = Some(mb);
        self
    }

    /// Add a platform diagnostic
    pub fn add_platform(&mut self, diag: PlatformDiagnostic) {
        self.platforms.push(diag);
    }

    /// Serialize to JSON
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string())
    }
}

/// Diagnostic provider
#[derive(Serialize, Deserialize)]
pub struct DiagnosticProvider {
    diagnostic: GatewayDiagnostic,
}

impl DiagnosticProvider {
    /// Create new provider
    pub fn new() -> Self {
        Self {
            diagnostic: GatewayDiagnostic::new(),
        }
    }

    /// Get diagnostic info
    pub fn diagnostic(&self) -> &GatewayDiagnostic {
        &self.diagnostic
    }

    /// Get mutable diagnostic
    pub fn diagnostic_mut(&mut self) -> &mut GatewayDiagnostic {
        &mut self.diagnostic
    }

    /// Collect current diagnostic info
    pub fn collect(&mut self) -> GatewayDiagnostic {
        self.diagnostic.update_uptime();
        self.diagnostic.clone()
    }
}

impl Default for DiagnosticProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_platform_diagnostic() {
        let diag = PlatformDiagnostic::new("telegram")
            .with_status(PlatformStatus::Running)
            .with_start_time(1234567890);
        
        assert_eq!(diag.platform, "telegram");
        assert_eq!(diag.status, PlatformStatus::Running);
        assert_eq!(diag.started_at, Some(1234567890));
    }

    #[test]
    fn test_gateway_diagnostic() {
        let mut diag = GatewayDiagnostic::new();
        diag.add_platform(PlatformDiagnostic::new("telegram"));
        diag.add_platform(PlatformDiagnostic::new("discord"));
        
        assert_eq!(diag.platforms.len(), 2);
        assert!(diag.uptime_seconds >= 0);
    }

    #[test]
    fn test_diagnostic_provider() {
        let mut provider = DiagnosticProvider::new();
        let diag = provider.collect();
        
        assert_eq!(diag.platform_count, 0);
        assert!(diag.uptime_seconds >= 0);
    }

    #[test]
    fn test_diagnostic_serialization() {
        let mut diag = GatewayDiagnostic::new();
        diag.add_platform(PlatformDiagnostic::new("telegram"));
        
        let json = diag.to_json();
        assert!(json.contains("telegram"));
    }
}
