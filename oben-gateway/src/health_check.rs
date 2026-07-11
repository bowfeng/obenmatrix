//! Gateway health check - health monitoring for gateway components
//!
//! HealthChecker monitors the health of gateway components and platforms

use std::collections::HashMap;

use crate::platform::PlatformAdapter;

/// Health status of a component
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HealthStatus {
    /// Component is healthy
    Healthy,
    /// Component is unhealthy
    Unhealthy { reason: String },
    /// Component is unknown (not yet checked)
    Unknown,
}

/// Health check result
#[derive(Clone, Debug)]
pub struct HealthCheckResult {
    /// Component name
    pub component: String,
    /// Health status
    pub status: HealthStatus,
    /// Timestamp of check
    pub timestamp: u64,
    /// Response time in milliseconds
    pub response_time_ms: u64,
}

impl HealthCheckResult {
    /// Create a new health check result
    pub fn new(component: &str, status: HealthStatus, response_time_ms: u64) -> Self {
        Self {
            component: component.to_string(),
            status,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            response_time_ms,
        }
    }

    /// Check if the component is healthy
    pub fn is_healthy(&self) -> bool {
        matches!(self.status, HealthStatus::Healthy)
    }
}

/// Health checker for gateway components
pub struct HealthChecker {
    /// Results of health checks
    results: HashMap<String, HealthCheckResult>,
    /// Timeout for health checks in milliseconds
    timeout_ms: u64,
}

impl HealthChecker {
    /// Create new health checker with default timeout (5000ms)
    pub fn new() -> Self {
        Self {
            results: HashMap::new(),
            timeout_ms: 5000,
        }
    }

    /// Create new health checker with custom timeout
    pub fn with_timeout(timeout_ms: u64) -> Self {
        Self {
            results: HashMap::new(),
            timeout_ms,
        }
    }

    /// Get the timeout in milliseconds
    pub fn timeout_ms(&self) -> u64 {
        self.timeout_ms
    }

    /// Check health of a platform adapter
    pub async fn check_platform_adapter(
        &mut self,
        platform: &str,
        adapter: &mut (dyn PlatformAdapter + Send + Sync),
    ) -> HealthCheckResult {
        let start = std::time::SystemTime::now();

        let result = tokio::time::timeout(
            std::time::Duration::from_millis(self.timeout_ms),
            adapter.health_check(),
        )
        .await;

        let response_time = start
            .duration_since(std::time::SystemTime::now())
            .map(|d| d.as_millis() as u64)
            .unwrap_or(self.timeout_ms);

        let status = match result {
            Ok(health) => {
                if health {
                    HealthStatus::Healthy
                } else {
                    HealthStatus::Unhealthy {
                        reason: "Health check returned false".to_string(),
                    }
                }
            }
            Err(_) => HealthStatus::Unhealthy {
                reason: "Health check timed out".to_string(),
            },
        };

        let result = HealthCheckResult::new(platform, status, response_time);
        self.results.insert(platform.to_string(), result.clone());
        result
    }

    /// Check health of all platforms
    pub async fn check_all<'a>(
        &'a mut self,
        adapters: &mut HashMap<String, Box<dyn PlatformAdapter + Send + Sync>>,
    ) -> HashMap<String, HealthCheckResult> {
        let mut results = HashMap::new();

        for (platform, adapter) in adapters.iter_mut() {
            let result = self.check_platform_adapter(platform, adapter.as_mut()).await;
            results.insert(platform.clone(), result);
        }

        results
    }

    /// Get health status of a specific component
    pub fn get(&self, component: &str) -> Option<&HealthCheckResult> {
        self.results.get(component)
    }

    /// Get all health check results
    pub fn all_results(&self) -> &HashMap<String, HealthCheckResult> {
        &self.results
    }

    /// Get all healthy components
    pub fn healthy_components(&self) -> Vec<&String> {
        self.results
            .iter()
            .filter(|(_, result)| result.is_healthy())
            .map(|(name, _)| name)
            .collect()
    }

    /// Get all unhealthy components
    pub fn unhealthy_components(&self) -> Vec<(&String, &HealthCheckResult)> {
        self.results
            .iter()
            .filter(|(_, result)| !result.is_healthy())
            .collect()
    }

    /// Check if all components are healthy
    pub fn are_all_healthy(&self) -> bool {
        self.results.values().all(HealthCheckResult::is_healthy)
    }
}

impl Default for HealthChecker {
    fn default() -> Self {
        Self::new()
    }
}

/// Health check handler - callable for health endpoints
pub struct HealthCheckHandler {
    checker: HealthChecker,
}

impl HealthCheckHandler {
    /// Create new handler
    pub fn new() -> Self {
        Self {
            checker: HealthChecker::new(),
        }
    }

    /// Perform health check and return summary
    pub async fn check(&mut self) -> HealthCheckSummary {
        let healthy = self.checker.healthy_components();
        let unhealthy = self.checker.unhealthy_components();

        HealthCheckSummary {
            overall: if unhealthy.is_empty() {
                HealthStatus::Healthy
            } else {
                HealthStatus::Unhealthy {
                    reason: format!("{} components unhealthy", unhealthy.len()),
                }
            },
            healthy_count: healthy.len(),
            unhealthy_count: unhealthy.len(),
            components: self.checker.all_results().clone(),
        }
    }

    /// Get the underlying checker
    pub fn checker(&self) -> &HealthChecker {
        &self.checker
    }

    /// Get mutable reference to checker
    pub fn checker_mut(&mut self) -> &mut HealthChecker {
        &mut self.checker
    }
}

impl Default for HealthCheckHandler {
    fn default() -> Self {
        Self::new()
    }
}

/// Health check summary
#[derive(Clone, Debug)]
pub struct HealthCheckSummary {
    /// Overall health status
    pub overall: HealthStatus,
    /// Number of healthy components
    pub healthy_count: usize,
    /// Number of unhealthy components
    pub unhealthy_count: usize,
    /// Detailed results per component
    pub components: HashMap<String, HealthCheckResult>,
}

impl HealthCheckSummary {
    /// Check if overall health is good
    pub fn is_healthy(&self) -> bool {
        self.overall == HealthStatus::Healthy
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::{OutgoingMessage, PlatformAdapter};

    #[derive(Clone)]
    struct TestAdapter {
        name: String,
        should_pass: bool,
    }

    #[async_trait::async_trait]
    impl PlatformAdapter for TestAdapter {
        fn name(&self) -> &str {
            &self.name
        }

        async fn listen(&mut self) -> anyhow::Result<()> {
            Ok(())
        }

        async fn stop(&mut self) {}

        async fn send(&self, _msg: OutgoingMessage) -> anyhow::Result<()> {
            Ok(())
        }

        async fn health_check(&self) -> bool {
            self.should_pass
        }
    }

    #[tokio::test]
    async fn test_health_checker_healthy() {
        let mut checker = HealthChecker::new();
        let mut adapter = TestAdapter {
            name: "telegram".to_string(),
            should_pass: true,
        };

        let result = checker.check_platform_adapter("telegram", &mut adapter).await;
        assert_eq!(result.status, HealthStatus::Healthy);
        assert!(result.is_healthy());
    }

    #[tokio::test]
    async fn test_health_checker_unhealthy() {
        let mut checker = HealthChecker::new();
        let mut adapter = TestAdapter {
            name: "discord".to_string(),
            should_pass: false,
        };

        let result = checker.check_platform_adapter("discord", &mut adapter).await;
        assert_eq!(result.status, HealthStatus::Unhealthy { reason: "Health check returned false".to_string() });
        assert!(!result.is_healthy());
    }

    #[tokio::test]
    async fn test_health_checker_timeout() {
        let mut checker = HealthChecker::with_timeout(100);
        
        // Create a mock adapter that takes too long
        struct SlowAdapter {
            name: String,
        }

        #[async_trait::async_trait]
        impl PlatformAdapter for SlowAdapter {
            fn name(&self) -> &str { &self.name }

            async fn listen(&mut self) -> anyhow::Result<()> { Ok(()) }
            async fn stop(&mut self) {}
            async fn send(&self, _msg: OutgoingMessage) -> anyhow::Result<()> { Ok(()) }
            
            async fn health_check(&self) -> bool {
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                true
            }
        }

        let mut adapter = SlowAdapter { name: "slow".to_string() };
        let result = checker.check_platform_adapter("slow", &mut adapter).await;
        
        assert_eq!(result.status, HealthStatus::Unhealthy { reason: "Health check timed out".to_string() });
        assert!(!result.is_healthy());
    }

    #[tokio::test]
    async fn test_health_check_handler() {
        let mut handler = HealthCheckHandler::new();
        
        // Add some adapters
        let mut adapters: HashMap<String, Box<dyn PlatformAdapter + Send + Sync>> = HashMap::new();
        
        let adapter1 = TestAdapter { name: "healthy".to_string(), should_pass: true };
        let adapter2 = TestAdapter { name: "unhealthy".to_string(), should_pass: false };
        
        adapters.insert("healthy".to_string(), Box::new(adapter1));
        adapters.insert("unhealthy".to_string(), Box::new(adapter2));
        
        let results = handler.checker_mut().check_all(&mut adapters).await;
        
        assert!(results.get("healthy").unwrap().is_healthy());
        assert!(!results.get("unhealthy").unwrap().is_healthy());
    }

    #[test]
    fn test_health_summary() {
        let summary = HealthCheckSummary {
            overall: HealthStatus::Healthy,
            healthy_count: 2,
            unhealthy_count: 0,
            components: HashMap::new(),
        };

        assert!(summary.is_healthy());
        assert_eq!(summary.healthy_count, 2);
    }
}
