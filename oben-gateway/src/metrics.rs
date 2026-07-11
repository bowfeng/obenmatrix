//! Gateway metrics - track performance metrics
//!
//! GatewayMetrics tracks various metrics about gateway operation including:
//! - Message rates
//! - Latency statistics
//! - Platform-specific metrics

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Per-platform metrics
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct PlatformMetrics {
    /// Total messages received
    pub messages_received: u64,
    /// Total messages sent
    pub messages_sent: u64,
    /// Total messages failed
    pub messages_failed: u64,
    /// Total bytes received
    pub bytes_received: u64,
    /// Total bytes sent
    pub bytes_sent: u64,
    /// Last message timestamp
    pub last_message: Option<u64>,
}

impl PlatformMetrics {
    /// Create new platform metrics
    pub fn new() -> Self {
        Self {
            messages_received: 0,
            messages_sent: 0,
            messages_failed: 0,
            bytes_received: 0,
            bytes_sent: 0,
            last_message: None,
        }
    }

    /// Record a received message
    pub fn record_received(&mut self, bytes: u64) {
        self.messages_received += 1;
        self.bytes_received += bytes;
        self.last_message = Some(std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs());
    }

    /// Record a sent message
    pub fn record_sent(&mut self, bytes: u64) {
        self.messages_sent += 1;
        self.bytes_sent += bytes;
        self.last_message = Some(std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs());
    }

    /// Record a failed message
    pub fn record_failed(&mut self) {
        self.messages_failed += 1;
    }
}

/// Overall gateway metrics
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct GatewayMetrics {
    /// Per-platform metrics
    pub platforms: HashMap<String, PlatformMetrics>,
    /// Total messages processed
    pub total_messages: u64,
    /// Total messages failed
    pub total_failed: u64,
    /// Average latency in milliseconds (moving average)
    pub avg_latency_ms: f64,
    /// Peak latency in milliseconds
    pub peak_latency_ms: u64,
    /// Uptime in seconds
    pub uptime_seconds: u64,
    /// Start time
    pub start_time: u64,
}

impl GatewayMetrics {
    /// Create new gateway metrics
    pub fn new() -> Self {
        Self {
            platforms: HashMap::new(),
            total_messages: 0,
            total_failed: 0,
            avg_latency_ms: 0.0,
            peak_latency_ms: 0,
            uptime_seconds: 0,
            start_time: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
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

    /// Record a message for a platform
    pub fn record_message(&mut self, platform: &str, bytes: u64) {
        self.total_messages += 1;
        self.platforms
            .entry(platform.to_string())
            .or_default()
            .record_received(bytes);
    }

    /// Record a failed message
    pub fn record_failure(&mut self, platform: &str) {
        self.total_failed += 1;
        self.platforms
            .entry(platform.to_string())
            .or_default()
            .record_failed();
    }

    /// Record a sent message
    pub fn record_sent(&mut self, platform: &str, bytes: u64) {
        self.platforms
            .entry(platform.to_string())
            .or_default()
            .record_sent(bytes);
    }

    /// Record latency
    pub fn record_latency(&mut self, latency_ms: u64) {
        // Simple moving average
        if self.total_messages > 0 {
            let count = self.total_messages as f64;
            self.avg_latency_ms =
                (self.avg_latency_ms * count + latency_ms as f64) / (count + 1.0);
        } else {
            self.avg_latency_ms = latency_ms as f64;
        }

        if latency_ms > self.peak_latency_ms {
            self.peak_latency_ms = latency_ms;
        }
    }

    /// Get metrics for a specific platform
    pub fn platform_metrics(&self, platform: &str) -> Option<&PlatformMetrics> {
        self.platforms.get(platform)
    }

    /// Get all platform names
    pub fn platform_names(&self) -> Vec<&String> {
        self.platforms.keys().collect()
    }

    /// Serialize metrics to JSON
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string())
    }
}

/// Metrics recorder - convenience wrapper
pub struct MetricsRecorder {
    metrics: GatewayMetrics,
}

impl MetricsRecorder {
    /// Create new recorder
    pub fn new() -> Self {
        Self {
            metrics: GatewayMetrics::new(),
        }
    }

    /// Get immutable metrics reference
    pub fn metrics(&self) -> &GatewayMetrics {
        &self.metrics
    }

    /// Get mutable metrics reference
    pub fn metrics_mut(&mut self) -> &mut GatewayMetrics {
        &mut self.metrics
    }

    /// Record a received message
    pub fn record_received(&mut self, platform: &str, bytes: u64) {
        self.metrics.record_message(platform, bytes);
    }

    /// Record a sent message
    pub fn record_sent(&mut self, platform: &str, bytes: u64) {
        self.metrics.record_sent(platform, bytes);
    }

    /// Record a failed message
    pub fn record_failed(&mut self, platform: &str) {
        self.metrics.record_failure(platform);
    }

    /// Record a latency measurement
    pub fn record_latency(&mut self, latency_ms: u64) {
        self.metrics.record_latency(latency_ms);
    }

    /// Update uptime
    pub fn update_uptime(&mut self) {
        self.metrics.update_uptime();
    }
}

impl Default for MetricsRecorder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_platform_metrics() {
        let mut metrics = PlatformMetrics::new();
        
        metrics.record_received(100);
        assert_eq!(metrics.messages_received, 1);
        assert_eq!(metrics.bytes_received, 100);
        
        metrics.record_sent(50);
        assert_eq!(metrics.messages_sent, 1);
        assert_eq!(metrics.bytes_sent, 50);
        
        metrics.record_failed();
        assert_eq!(metrics.messages_failed, 1);
    }

    #[test]
    fn test_gateway_metrics() {
        let mut metrics = GatewayMetrics::new();
        
        metrics.record_message("telegram", 100);
        assert_eq!(metrics.total_messages, 1);
        assert_eq!(metrics.platform_metrics("telegram").unwrap().messages_received, 1);
        
        metrics.record_failure("telegram");
        assert_eq!(metrics.total_failed, 1);
        
        metrics.record_sent("telegram", 200);
        assert_eq!(metrics.platform_metrics("telegram").unwrap().messages_sent, 1);
    }

    #[test]
    fn test_metrics_recorder() {
        let mut recorder = MetricsRecorder::new();
        
        recorder.record_received("telegram", 100);
        recorder.record_sent("telegram", 50);
        recorder.record_latency(100);
        
        assert_eq!(recorder.metrics().total_messages, 1);
        assert_eq!(recorder.metrics().platform_metrics("telegram").unwrap().messages_received, 1);
        let avg_latency = recorder.metrics().avg_latency_ms;
        assert!(avg_latency > 0.0, "avg_latency_ms should be positive");
    }

    #[test]
    fn test_metrics_serialization() {
        let mut metrics = GatewayMetrics::new();
        metrics.record_message("telegram", 100);
        metrics.record_latency(50);
        
        let json = metrics.to_json();
        assert!(json.contains("telegram"));
    }

    #[test]
    fn test_metrics_collection() {
        let mut metrics = GatewayMetrics::new();
        
        for i in 0..10 {
            metrics.record_message("telegram", 100 + i * 10);
            metrics.record_latency(i * 10);
        }
        
        assert_eq!(metrics.total_messages, 10);
        assert!(metrics.peak_latency_ms > 0);
    }

    #[test]
    fn test_metrics_aggregation() {
        let mut metrics = GatewayMetrics::new();
        
        metrics.record_message("telegram", 100);
        metrics.record_message("discord", 200);
        metrics.record_message("telegram", 300);
        
        let telegram_metrics = metrics.platform_metrics("telegram").unwrap();
        assert_eq!(telegram_metrics.messages_received, 2);
        assert_eq!(telegram_metrics.bytes_received, 400);
    }

    #[test]
    fn test_metrics_export_json() {
        let mut metrics = GatewayMetrics::new();
        metrics.record_message("telegram", 100);
        
        let json = metrics.to_json();
        assert!(json.contains("total_messages"));
        assert!(json.contains("telegram"));
    }
}
