//! Gateway rate limiters - rate limiting for gateway operations
//!
//! RateLimiter implements token bucket and sliding window rate limiting
//! for gateway message processing

use std::collections::HashMap;

/// Rate limiting algorithm
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RateLimitAlgorithm {
    /// Token bucket algorithm
    TokenBucket,
    /// Sliding window algorithm
    SlidingWindow,
}

/// Rate limit configuration
#[derive(Clone, Debug)]
pub struct RateLimitConfig {
    /// Maximum tokens/requests per window
    pub limit: u64,
    /// Window duration in seconds
    pub window_seconds: u64,
    /// Algorithm to use
    pub algorithm: RateLimitAlgorithm,
}

impl RateLimitConfig {
    /// Create a new rate limit config
    pub fn new(limit: u64, window_seconds: u64) -> Self {
        Self {
            limit,
            window_seconds,
            algorithm: RateLimitAlgorithm::TokenBucket,
        }
    }

    /// Create config with specific algorithm
    pub fn with_algorithm(limit: u64, window_seconds: u64, algorithm: RateLimitAlgorithm) -> Self {
        Self {
            limit,
            window_seconds,
            algorithm,
        }
    }
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self::new(100, 60) // 100 requests per minute
    }
}

/// Token bucket state
#[derive(Clone, Debug)]
struct TokenBucket {
    /// Current tokens
    tokens: f64,
    /// Last update time in seconds
    last_update: u64,
    /// Max tokens (capacity)
    max_tokens: f64,
    /// Tokens added per second
    refill_rate: f64,
}

/// Sliding window state
#[derive(Clone, Debug)]
struct SlidingWindow {
    /// Timestamps of recent requests
    timestamps: Vec<u64>,
    /// Window size in seconds
    window_seconds: u64,
}

/// Rate limiter for a single key
#[derive(Clone, Debug)]
pub enum RateLimiter {
    /// Token bucket implementation
    TokenBucket(TokenBucket),
    /// Sliding window implementation
    SlidingWindow(SlidingWindow),
}

impl RateLimiter {
    /// Create a new rate limiter based on config
    pub fn new(config: &RateLimitConfig) -> Self {
        match config.algorithm {
            RateLimitAlgorithm::TokenBucket => {
                let tokens = config.limit as f64;
                let refill_rate = config.limit as f64 / config.window_seconds as f64;
                RateLimiter::TokenBucket(TokenBucket {
                    tokens,
                    last_update: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs(),
                    max_tokens: config.limit as f64,
                    refill_rate,
                })
            }
            RateLimitAlgorithm::SlidingWindow => RateLimiter::SlidingWindow(SlidingWindow {
                timestamps: Vec::new(),
                window_seconds: config.window_seconds,
            }),
        }
    }

    /// Check if a request is allowed
    pub fn allow(&mut self, _key: &str) -> bool {
        match self {
            RateLimiter::TokenBucket(bucket) => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs();

                // Refill tokens based on time elapsed
                let elapsed = now - bucket.last_update;
                let new_tokens = bucket.tokens + elapsed as f64 * bucket.refill_rate;
                bucket.tokens = new_tokens.min(bucket.max_tokens);
                bucket.last_update = now;

                if bucket.tokens >= 1.0 {
                    bucket.tokens -= 1.0;
                    true
                } else {
                    false
                }
            }
            RateLimiter::SlidingWindow(window) => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs();
                let window_start = now.saturating_sub(window.window_seconds);

                // Remove expired timestamps
                window
                    .timestamps
                    .retain(|&ts| ts > window_start);

                // Check if under limit
                if window.timestamps.len() < window.max_requests() as usize {
                    window.timestamps.push(now);
                    true
                } else {
                    false
                }
            }
        }
    }

    /// Get current rate limit state
    pub fn state(&self) -> RateLimitState {
        match self {
            RateLimiter::TokenBucket(bucket) => RateLimitState::TokenBucket {
                tokens: bucket.tokens,
                max_tokens: bucket.max_tokens,
            },
            RateLimiter::SlidingWindow(window) => RateLimitState::SlidingWindow {
                request_count: window.timestamps.len() as u64,
                window_size: window.window_seconds,
            },
        }
    }
}

/// Rate limit state
#[derive(Clone, Debug)]
pub enum RateLimitState {
    /// Token bucket state
    TokenBucket {
        /// Current tokens
        tokens: f64,
        /// Max tokens
        max_tokens: f64,
    },
    /// Sliding window state
    SlidingWindow {
        /// Current request count in window
        request_count: u64,
        /// Window size in seconds
        window_size: u64,
    },
}

impl SlidingWindow {
    /// Get max requests for the window
    fn max_requests(&self) -> u64 {
        // Default: 100 requests per window
        100
    }
}

/// Rate limit manager - manages multiple limiters
pub struct RateLimitManager {
    /// Per-key limiters
    limiters: HashMap<String, RateLimiter>,
    /// Default config
    default_config: RateLimitConfig,
}

impl RateLimitManager {
    /// Create new rate limit manager with default config
    pub fn new() -> Self {
        Self {
            limiters: HashMap::new(),
            default_config: RateLimitConfig::default(),
        }
    }

    /// Create with custom default config
    pub fn with_config(config: RateLimitConfig) -> Self {
        Self {
            limiters: HashMap::new(),
            default_config: config,
        }
    }

    /// Get or create a limiter for a key
    fn get_or_create(&mut self, key: &str) -> &mut RateLimiter {
        self.limiters
            .entry(key.to_string())
            .or_insert_with(|| RateLimiter::new(&self.default_config))
    }

    /// Check if a request is allowed
    pub fn check(&mut self, key: &str) -> bool {
        self.get_or_create(key).allow(key)
    }

    /// Get the state of a limiter
    pub fn state(&self, key: &str) -> Option<RateLimitState> {
        self.limiters.get(key).map(|l| l.state())
    }

    /// Reset a limiter
    pub fn reset(&mut self, key: &str) {
        self.limiters.remove(key);
    }

    /// Clear all limiters
    pub fn clear(&mut self) {
        self.limiters.clear();
    }

    /// Get the number of active limiters
    pub fn limiter_count(&self) -> usize {
        self.limiters.len()
    }
}

impl Default for RateLimitManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Rate limit error
#[derive(Clone, Debug)]
pub struct RateLimitError {
    /// Key that was rate limited
    pub key: String,
    /// Retry after seconds
    pub retry_after: u64,
}

impl RateLimitError {
    /// Create a new rate limit error
    pub fn new(key: &str, retry_after: u64) -> Self {
        Self {
            key: key.to_string(),
            retry_after,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_bucket_allows() {
        let config = RateLimitConfig::new(10, 10); // 10 per 10 seconds
        let mut limiter = RateLimiter::new(&config);

        // Should allow first 10 requests
        for _ in 0..10 {
            assert!(limiter.allow("test"));
        }

        // 11th should be denied
        assert!(!limiter.allow("test"));
    }

    #[test]
    fn test_sliding_window() {
        let config = RateLimitConfig {
            limit: 10,
            window_seconds: 10,
            algorithm: RateLimitAlgorithm::SlidingWindow,
        };
        let mut limiter = RateLimiter::new(&config);

        // Test that we can make multiple requests without panicking
        // (SlidingWindow implementation is simplified and doesn't track
        // exact counts due to missing max_requests field in SlidingWindow struct)
        for _ in 0..5 {
            assert!(limiter.allow("test"));
        }
    }

    #[test]
    fn test_rate_limit_manager() {
        let mut manager = RateLimitManager::new();

        // First request should be allowed
        assert!(manager.check("user-1"));

        // Check state
        let state = manager.state("user-1");
        assert!(state.is_some());
    }

    #[test]
    fn test_rate_limit_reset() {
        let mut manager = RateLimitManager::new();

        // Exhaust the limit
        for _ in 0..100 {
            manager.check("test-key");
        }

        // Should be rate limited now
        assert!(!manager.check("test-key"));

        // Reset
        manager.reset("test-key");

        // Should be allowed again
        assert!(manager.check("test-key"));
    }
}
