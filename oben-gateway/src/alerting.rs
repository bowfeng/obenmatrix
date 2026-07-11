//! Gateway alerting - alert on gateway issues
//!
//! AlertManager handles alerting for gateway issues

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Alert severity levels
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum AlertSeverity {
    Critical,
    Warning,
    Info,
}

/// Alert types
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AlertType {
    PlatformFailure,
    HighLatency,
    RateLimitExceeded,
    CircuitOpen,
    MemoryHigh,
    DiskHigh,
    NetworkError,
    Unknown,
}

/// An alert about a gateway issue
#[derive(Clone, Debug)]
pub struct Alert {
    pub alert_type: AlertType,
    pub severity: AlertSeverity,
    pub platform: Option<String>,
    pub message: String,
    pub timestamp: Instant,
    pub id: String,
}

impl Alert {
    pub fn new(alert_type: AlertType, message: &str) -> Self {
        let now = Instant::now();
        let alert_id = format!("alert_{}_{}", alert_type as u8, now.elapsed().as_secs());
        Self {
            alert_type,
            severity: AlertSeverity::Warning,
            platform: None,
            message: message.to_string(),
            timestamp: now,
            id: alert_id,
        }
    }

    pub fn with_platform(mut self, platform: &str) -> Self {
        self.platform = Some(platform.to_string());
        self
    }

    /// Set severity
    pub fn set_severity(&mut self, severity: AlertSeverity) {
        self.severity = severity;
    }

    /// Check if alert is critical
    pub fn is_critical(&self) -> bool {
        self.severity == AlertSeverity::Critical
    }

    /// Check if alert is warning
    pub fn is_warning(&self) -> bool {
        self.severity == AlertSeverity::Warning
    }
}

/// Alert manager
pub struct AlertManager {
    /// Active alerts
    active_alerts: HashMap<String, Alert>,
    /// Alert history
    history: Vec<Alert>,
    /// Alert config
    config: AlertConfig,
}

/// Alert configuration
#[derive(Clone, Debug)]
pub struct AlertConfig {
    /// Max alerts to keep in history
    pub max_history: usize,
    /// Alert cooldown period
    pub cooldown_period: Duration,
}

impl AlertConfig {
    /// Create new config
    pub fn new() -> Self {
        Self {
            max_history: 100,
            cooldown_period: Duration::from_secs(60),
        }
    }

    /// Set max history
    pub fn with_max_history(mut self, max: usize) -> Self {
        self.max_history = max;
        self
    }

    /// Set cooldown period
    pub fn with_cooldown_period(mut self, period: Duration) -> Self {
        self.cooldown_period = period;
        self
    }
}

impl Default for AlertConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl AlertManager {
    /// Create new alert manager
    pub fn new() -> Self {
        Self {
            active_alerts: HashMap::new(),
            history: Vec::new(),
            config: AlertConfig::new(),
        }
    }

    /// Create with config
    pub fn with_config(config: AlertConfig) -> Self {
        Self {
            active_alerts: HashMap::new(),
            history: Vec::new(),
            config,
        }
    }

    /// Create a new alert
    pub fn create_alert(&mut self, alert_type: AlertType, message: &str) -> String {
        let now = Instant::now();
        let id = format!("alert_{}_{}", alert_type as u8, now.elapsed().as_secs());
        let alert = Alert::new(alert_type, message);
        self.active_alerts.insert(id.clone(), alert);
        id
    }

    /// Create a platform-specific alert
    pub fn create_platform_alert(&mut self, platform: &str, alert_type: AlertType, message: &str) -> String {
        let id = self.create_alert(alert_type, message);
        if let Some(alert) = self.active_alerts.get_mut(&id) {
            alert.platform = Some(platform.to_string());
        }
        id
    }

    /// Resolve an alert
    pub fn resolve_alert(&mut self, id: &str) -> bool {
        if self.active_alerts.remove(id).is_some() {
            self.history.push(Alert {
                message: "Resolved".to_string(),
                timestamp: Instant::now(),
                id: id.to_string(),
                ..Default::default()
            });
            true
        } else {
            false
        }
    }

    /// Get active alerts
    pub fn active_alerts(&self) -> Vec<&Alert> {
        self.active_alerts.values().collect()
    }

    /// Get alerts for a platform
    pub fn platform_alerts(&self, platform: &str) -> Vec<&Alert> {
        self.active_alerts
            .values()
            .filter(|a| a.platform.as_deref() == Some(platform))
            .collect()
    }

    /// Get critical alerts
    pub fn critical_alerts(&self) -> Vec<&Alert> {
        self.active_alerts
            .values()
            .filter(|a| a.is_critical())
            .collect()
    }

    /// Get warning alerts
    pub fn warning_alerts(&self) -> Vec<&Alert> {
        self.active_alerts
            .values()
            .filter(|a| a.is_warning())
            .collect()
    }

    /// Clear all active alerts
    pub fn clear_active(&mut self) {
        self.active_alerts.clear();
    }

    /// Clear history
    pub fn clear_history(&mut self) {
        self.history.clear();
    }

    /// Get history
    pub fn history(&self) -> &[Alert] {
        &self.history
    }

    /// Get total alert count
    pub fn alert_count(&self) -> usize {
        self.active_alerts.len() + self.history.len()
    }
}

impl Default for Alert {
    fn default() -> Self {
        Self {
            alert_type: AlertType::Unknown,
            severity: AlertSeverity::Info,
            platform: None,
            message: String::new(),
            timestamp: Instant::now(),
            id: String::new(),
        }
    }
}

impl Alert {
    /// Set alert ID
    pub fn with_id(mut self, id: &str) -> Self {
        self.id = id.to_string();
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_alert() {
        let mut manager = AlertManager::new();
        let id = manager.create_alert(AlertType::PlatformFailure, "Platform failed");
        assert!(manager.active_alerts.contains_key(&id));
    }

    #[test]
    fn test_platform_alert() {
        let mut manager = AlertManager::new();
        manager.create_platform_alert("telegram", AlertType::PlatformFailure, "Telegram failed");
        
        let alerts = manager.platform_alerts("telegram");
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].platform, Some("telegram".to_string()));
    }

    #[test]
    fn test_resolve_alert() {
        let mut manager = AlertManager::new();
        let id = manager.create_alert(AlertType::PlatformFailure, "Platform failed");
        assert!(manager.resolve_alert(&id));
        assert!(!manager.active_alerts.contains_key(&id));
    }

    #[test]
    fn test_critical_alerts() {
        let mut manager = AlertManager::new();
        let id = manager.create_alert(AlertType::PlatformFailure, "Critical failure");
        if let Some(alert) = manager.active_alerts.get_mut(&id) {
            alert.set_severity(AlertSeverity::Critical);
        }
        
        let critical = manager.critical_alerts();
        assert_eq!(critical.len(), 1);
    }
}
