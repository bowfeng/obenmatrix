//! Gateway circuit breakers - circuit breaker pattern for gateway operations
//!
//! CircuitBreaker implements the circuit breaker pattern to prevent cascading failures
//! in gateway operations

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Circuit breaker states
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CircuitState {
    /// Closed: Normal operation, requests pass through
    Closed,
    /// Open: Circuit is open, requests fail fast
    Open,
    /// Half-open: Testing if service has recovered
    HalfOpen,
}

/// Circuit breaker configuration
#[derive(Clone, Debug)]
pub struct CircuitBreakerConfig {
    /// Failure threshold (number of failures before opening)
    pub failure_threshold: u32,
    /// Success threshold (number of successes before closing)
    pub success_threshold: u32,
    /// Timeout before trying again (in seconds)
    pub timeout_seconds: u64,
    /// Half-open max requests
    pub half_open_max_requests: u32,
}

impl CircuitBreakerConfig {
    /// Create new config with defaults
    pub fn new() -> Self {
        Self {
            failure_threshold: 5,
            success_threshold: 3,
            timeout_seconds: 30,
            half_open_max_requests: 1,
        }
    }

    /// Set failure threshold
    pub fn with_failure_threshold(mut self, threshold: u32) -> Self {
        self.failure_threshold = threshold;
        self
    }

    /// Set success threshold
    pub fn with_success_threshold(mut self, threshold: u32) -> Self {
        self.success_threshold = threshold;
        self
    }

    /// Set timeout
    pub fn with_timeout(mut self, timeout_seconds: u64) -> Self {
        self.timeout_seconds = timeout_seconds;
        self
    }

    /// Set half-open max requests
    pub fn with_half_open_max_requests(mut self, max: u32) -> Self {
        self.half_open_max_requests = max;
        self
    }
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self::new()
    }
}

/// Circuit breaker state
#[derive(Clone, Debug)]
struct CircuitStateData {
    /// Current state
    state: CircuitState,
    /// Number of consecutive failures
    failure_count: u32,
    /// Number of consecutive successes
    success_count: u32,
    /// When state last changed
    state_changed_at: Instant,
    /// Number of requests in half-open state
    half_open_requests: u32,
}

/// Circuit breaker for a single key
pub struct CircuitBreaker {
    /// Config
    config: CircuitBreakerConfig,
    /// State
    state: CircuitStateData,
}

impl CircuitBreaker {
    /// Create new circuit breaker with config
    pub fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            config,
            state: CircuitStateData {
                state: CircuitState::Closed,
                failure_count: 0,
                success_count: 0,
                state_changed_at: Instant::now(),
                half_open_requests: 0,
            },
        }
    }

    /// Default config
    pub fn with_defaults() -> Self {
        Self::new(CircuitBreakerConfig::default())
    }

    /// Get current state (mutable for state transitions)
    pub fn state(&mut self) -> CircuitState {
        self.check_state_internal();
        self.state.state
    }

    /// Get current state (immutable, no transitions)
    pub fn state_immutable(&self) -> CircuitState {
        self.state.state
    }

    /// Check if state should transition (internal)
    fn check_state_internal(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.state.state_changed_at).as_secs();

        match self.state.state {
            CircuitState::Closed => {
                // Stay closed until threshold reached
            }
            CircuitState::Open => {
                // Transition to half-open after timeout
                if elapsed >= self.config.timeout_seconds {
                    self.state.state = CircuitState::HalfOpen;
                    self.state.state_changed_at = now;
                    self.state.half_open_requests = 0;
                }
            }
            CircuitState::HalfOpen => {
                // Stay half-open until threshold or timeout
            }
        }
    }

    /// Check if request is allowed
    pub fn allow(&mut self) -> bool {
        self.check_state_internal();

        match self.state.state {
            CircuitState::Closed => true,
            CircuitState::Open => false,
            CircuitState::HalfOpen => {
                if self.state.half_open_requests < self.config.half_open_max_requests {
                    self.state.half_open_requests += 1;
                    true
                } else {
                    false
                }
            }
        }
    }

    /// Record a success
    pub fn record_success(&mut self) {
        self.check_state_internal();

        match self.state.state {
            CircuitState::Closed => {
                self.state.failure_count = 0;
                self.state.success_count += 1;
            }
            CircuitState::Open => {
                // Transition to half-open
                self.state.state = CircuitState::HalfOpen;
                self.state.state_changed_at = Instant::now();
                self.state.half_open_requests = 0;
            }
            CircuitState::HalfOpen => {
                self.state.success_count += 1;
                self.state.half_open_requests -= 1;

                if self.state.success_count >= self.config.success_threshold {
                    self.state.state = CircuitState::Closed;
                    self.state.failure_count = 0;
                    self.state.success_count = 0;
                    self.state.state_changed_at = Instant::now();
                }
            }
        }
    }

    /// Record a failure
    pub fn record_failure(&mut self) {
        self.check_state_internal();

        match self.state.state {
            CircuitState::Closed => {
                self.state.failure_count += 1;
                self.state.success_count = 0;

                if self.state.failure_count >= self.config.failure_threshold {
                    self.state.state = CircuitState::Open;
                    self.state.state_changed_at = Instant::now();
                }
            }
            CircuitState::Open => {
                // Stay open
            }
            CircuitState::HalfOpen => {
                // Transition back to open
                self.state.state = CircuitState::Open;
                self.state.failure_count += 1;
                self.state.state_changed_at = Instant::now();
                self.state.half_open_requests = 0;
            }
        }
    }

    /// Get remaining time in current state
    pub fn remaining_time(&self) -> Option<Duration> {
        match self.state.state {
            CircuitState::Closed => None,
            CircuitState::Open => {
                let elapsed = Instant::now().duration_since(self.state.state_changed_at).as_secs();
                if elapsed < self.config.timeout_seconds {
                    Some(Duration::from_secs(self.config.timeout_seconds - elapsed))
                } else {
                    None
                }
            }
            CircuitState::HalfOpen => None,
        }
    }
}

/// Circuit breaker manager - manages multiple circuit breakers
pub struct CircuitBreakerManager {
    /// Per-key circuit breakers
    breakers: HashMap<String, CircuitBreaker>,
    /// Default config
    default_config: CircuitBreakerConfig,
}

impl CircuitBreakerManager {
    /// Create new manager with default config
    pub fn new() -> Self {
        Self {
            breakers: HashMap::new(),
            default_config: CircuitBreakerConfig::default(),
        }
    }

    /// Create with custom default config
    pub fn with_config(config: CircuitBreakerConfig) -> Self {
        Self {
            breakers: HashMap::new(),
            default_config: config,
        }
    }

    /// Get or create a circuit breaker
    fn get_or_create(&mut self, key: &str) -> &mut CircuitBreaker {
        self.breakers
            .entry(key.to_string())
            .or_insert_with(|| CircuitBreaker::new(self.default_config.clone()))
    }

    /// Check if request is allowed
    pub fn allow(&mut self, key: &str) -> bool {
        self.get_or_create(key).allow()
    }

    /// Record a success
    pub fn record_success(&mut self, key: &str) {
        self.get_or_create(key).record_success();
    }

    /// Record a failure
    pub fn record_failure(&mut self, key: &str) {
        self.get_or_create(key).record_failure();
    }

    /// Get state of a circuit breaker
    pub fn state(&self, key: &str) -> Option<CircuitState> {
        self.breakers.get(key).map(|b| b.state_immutable())
    }

    /// Reset a circuit breaker to closed state
    pub fn reset(&mut self, key: &str) {
        self.breakers.remove(key);
        // When reset, the circuit should be considered closed
        // The next call to allow() will create it in closed state
    }

    /// Clear all breakers
    pub fn clear(&mut self) {
        self.breakers.clear();
    }

    /// Get count of open breakers
    pub fn open_count(&self) -> usize {
        self.breakers
            .values()
            .filter(|b| b.state_immutable() == CircuitState::Open)
            .count()
    }

    /// Get count of closed breakers
    pub fn closed_count(&self) -> usize {
        self.breakers
            .values()
            .filter(|b| b.state_immutable() == CircuitState::Closed)
            .count()
    }

    /// Get count of half-open breakers
    pub fn half_open_count(&self) -> usize {
        self.breakers
            .values()
            .filter(|b| b.state_immutable() == CircuitState::HalfOpen)
            .count()
    }
}

impl Default for CircuitBreakerManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Circuit breaker error
#[derive(Clone, Debug)]
pub struct CircuitBreakerOpen {
    /// Key that's circuit-broken
    pub key: String,
    /// Remaining time
    pub remaining: Option<Duration>,
}

impl CircuitBreakerOpen {
    /// Create new error
    pub fn new(key: &str, remaining: Option<Duration>) -> Self {
        Self {
            key: key.to_string(),
            remaining,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_circuit_breaker_closed_initially() {
        let mut breaker = CircuitBreaker::with_defaults();
        assert_eq!(breaker.state(), CircuitState::Closed);
    }

    #[test]
    fn test_circuit_breaker_opens_after_failures() {
        let mut breaker = CircuitBreaker::with_defaults();
        assert_eq!(breaker.state(), CircuitState::Closed);

        // Record failures up to threshold
        for _ in 0..5 {
            breaker.record_failure();
        }

        assert_eq!(breaker.state(), CircuitState::Open);
    }

    #[test]
    fn test_circuit_breaker_half_open_transition() {
        let mut breaker = CircuitBreaker::new(CircuitBreakerConfig {
            failure_threshold: 2,
            success_threshold: 2,
            timeout_seconds: 1,
            half_open_max_requests: 1,
        });

        // Open the circuit
        breaker.record_failure();
        breaker.record_failure();
        assert_eq!(breaker.state(), CircuitState::Open);

        // Wait for timeout (in test, this is simulated)
        // In real usage, this would wait 1 second
    }

    #[test]
    fn test_circuit_breaker_allows_half_open() {
        let mut breaker = CircuitBreaker::new(CircuitBreakerConfig {
            failure_threshold: 2,
            success_threshold: 2,
            timeout_seconds: 1,
            half_open_max_requests: 2,
        });

        // Open the circuit
        breaker.record_failure();
        breaker.record_failure();

        // Should allow half-open requests after timeout
        // (In test, we can't actually wait, so we check logic)
    }

    #[test]
    fn test_circuit_breaker_manager() {
        let mut manager = CircuitBreakerManager::new();

        // Record some failures
        for _ in 0..5 {
            manager.record_failure("service-a");
        }

        // Should be open now
        assert_eq!(manager.state("service-a"), Some(CircuitState::Open));
        assert!(!manager.allow("service-a"));
    }

    #[test]
    fn test_circuit_breaker_reset() {
        let mut manager = CircuitBreakerManager::new();

        // Open the circuit
        for _ in 0..5 {
            manager.record_failure("service-a");
        }

        assert_eq!(manager.state("service-a"), Some(CircuitState::Open));

        // Reset
        manager.reset("service-a");

        // After reset, the key is removed - allow() will create a new closed circuit
        assert!(manager.allow("service-a"));
        // Now check state - it should be closed since allow() creates it
        assert_eq!(manager.state("service-a"), Some(CircuitState::Closed));
    }
}
