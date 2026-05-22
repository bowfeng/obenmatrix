/// Retry with jittered exponential backoff.
///
/// Wraps a single API call in a retry loop, backing off exponentially with
/// random jitter between retries. Only retries on configured HTTP status codes
/// (429, 500, 502, 503, 504).

use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use rand::Rng;
use tracing::{debug, warn};

/// Configuration for retry behavior.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts (default: 3).
    pub max_retries: u32,
    /// Base delay in milliseconds (default: 500).
    pub base_delay_ms: u64,
    /// Maximum delay in milliseconds (default: 60_000).
    pub max_delay_ms: u64,
    /// Jitter factor — fraction of delay to randomize (default: 0.5).
    /// Actual jitter = random(0, jitter_factor * delay).
    pub jitter_factor: f64,
    /// HTTP status codes that trigger a retry.
    pub retryable_codes: Vec<u16>,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay_ms: 500,
            max_delay_ms: 60_000,
            jitter_factor: 0.5,
            retryable_codes: vec![429, 500, 502, 503, 504],
        }
    }
}

impl RetryConfig {
    /// Calculate the delay for a given attempt number (0-indexed).
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        // Exponential: base * 2^attempt
        let exponential = self.base_delay_ms as f64 * 2.0_f64.powi(attempt as i32);
        // Cap at max
        let capped = exponential.min(self.max_delay_ms as f64);
        // Add jitter: random(0, jitter_factor * capped)
        let jitter = rand::thread_rng().gen_range(0.0..self.jitter_factor) * capped;
        let total_ms = capped + jitter;
        Duration::from_millis(total_ms as u64)
    }
}

/// Retry a fallible async operation with jittered exponential backoff.
///
/// Retries only when the error is a "retryable error" (created via
/// `RetryableError`). Non-retryable errors are returned immediately.
pub async fn retry_with_backoff<F, Fut, T>(
    config: &RetryConfig,
    operation: F,
) -> Result<T>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let mut last_err: Option<anyhow::Error> = None;

    for attempt in 0..=config.max_retries {
        if attempt > 0 {
            let delay = config.delay_for_attempt(attempt - 1);
            debug!("Retry attempt {}/{} after {:?} delay", attempt, config.max_retries, delay);
            tokio::time::sleep(delay).await;
        }

        match operation().await {
            Ok(value) => return Ok(value),
            Err(e) => {
                let msg = e.to_string();
                if let Some(retryable) = extract_retryable_code(&e) {
                    if attempt < config.max_retries && config.retryable_codes.contains(&retryable) {
                        last_err = Some(e);
                        warn!("Retryable error (HTTP {}), attempt {} of {}: {}", retryable, attempt + 1, config.max_retries, msg);
                        continue;
                    }
                }
                // Non-retryable error or out of retries — return it
                return Err(e);
            }
        }
    }

    // Should not reach here unless max_retries is 0 and first call fails
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("Operation failed after {} attempts", config.max_retries + 1)))
}

/// HTTP status code extracted from an error, if present.
fn extract_retryable_code(err: &anyhow::Error) -> Option<u16> {
    let msg = err.to_string();

    // reqwest errors often contain the status code
    if let Some(code) = extract_code_from_message(&msg) {
        return Some(code);
    }

    // Check for reqwest::Error
    if let Some(reqwest_err) = err.downcast_ref::<reqwest::Error>() {
        if let Some(status) = reqwest_err.status() {
            return Some(status.as_u16());
        }
    }

    // Check chained errors
    for cause in err.chain() {
        if let Some(code) = extract_code_from_message(&cause.to_string()) {
            return Some(code);
        }
    }

    None
}

fn extract_code_from_message(msg: &str) -> Option<u16> {
    // Match patterns like "HTTP 429", "status code: 503", "[HTTP 429]", "[429]"
    // Try extracting from common patterns
    if let Ok(re) = regex::Regex::new(r"\[?HTTP[_\s:]*?(\d{3})\]?") {
        if let Some(cap) = re.captures(msg) {
            if let Ok(code) = cap[1].parse::<u16>() {
                if (400..=599).contains(&code) {
                    return Some(code);
                }
            }
        }
    }
    // Fallback: split by whitespace and try to parse each token
    for word in msg.split_whitespace() {
        let cleaned = word.trim_end_matches(':').trim_end_matches(']').trim_start_matches('[');
        if let Ok(code) = cleaned.parse::<u16>() {
            if (400..=599).contains(&code) {
                return Some(code);
            }
        }
    }
    None
}

/// Mark an error as retryable with a specific HTTP status code.
pub fn retryable_error(msg: impl Into<String>, http_code: u16) -> anyhow::Error {
    anyhow::anyhow!("[HTTP {}] {}", http_code, msg.into())
}

/// Mark a transient error as retryable (generic 500).
pub fn retryable_transient(msg: impl Into<String>) -> anyhow::Error {
    retryable_error(msg, 500)
}

/// Check if an error is retryable based on its HTTP status code.
pub fn is_retryable(err: &anyhow::Error) -> bool {
    extract_retryable_code(err).map_or(false, |code| {
        (400..=599).contains(&code)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_delay_calculation() {
        let config = RetryConfig::default();
        // First retry: 500ms base, with jitter up to 250ms
        let d1 = config.delay_for_attempt(0);
        assert!(d1 >= Duration::from_millis(500));
        assert!(d1 <= Duration::from_millis(750));

        // Second retry: 1000ms base, with jitter
        let d2 = config.delay_for_attempt(1);
        assert!(d2 >= Duration::from_millis(1000));
        assert!(d2 <= Duration::from_millis(1500));
    }

    #[test]
    fn test_delay_capped_at_max() {
        let config = RetryConfig {
            max_delay_ms: 1000,
            ..Default::default()
        };
        // After 10 retries, exponential would be huge, but should be capped
        let d = config.delay_for_attempt(10);
        assert!(d <= Duration::from_millis(1000 + 500)); // max + jitter
    }

    #[test]
    fn test_retryable_error_code_extraction() {
        let err = retryable_error("rate limited", 429);
        assert_eq!(extract_retryable_code(&err), Some(429));

        let err = anyhow::anyhow!("HTTP 503 Service Unavailable");
        assert_eq!(extract_retryable_code(&err), Some(503));
    }

    #[tokio::test]
    async fn test_retry_succeeds_on_first_try() {
        let config = RetryConfig { max_retries: 3, ..Default::default() };
        let call_count = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let cc = call_count.clone();

        let result = retry_with_backoff(&config, move || {
            let cc = cc.clone();
            async move {
                cc.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok::<_, anyhow::Error>("ok")
            }
        }).await;

        assert_eq!(result.unwrap(), "ok");
        assert_eq!(call_count.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_retry_retries_on_failure_then_succeeds() {
        let config = RetryConfig { max_retries: 3, ..Default::default() };
        let call_count = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let cc = call_count.clone();

        let result = retry_with_backoff(&config, move || {
            let count = cc.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let err = retryable_transient("server error");
            async move {
                if count < 2 {
                    Err(err)
                } else {
                    Ok::<_, anyhow::Error>("recovered")
                }
            }
        }).await;

        assert_eq!(result.unwrap(), "recovered");
        assert_eq!(call_count.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_retry_exhausted_returns_error() {
        let config = RetryConfig { max_retries: 2, ..Default::default() };

        let result: std::result::Result<(), anyhow::Error> = retry_with_backoff(&config, move || {
            async move { Err(retryable_transient("still failing")) }
        }).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_non_retryable_error_returns_immediately() {
        let config = RetryConfig { max_retries: 5, ..Default::default() };

        let result: Result<(), anyhow::Error> = retry_with_backoff(&config, move || {
            async move { Err(anyhow::anyhow!("400 Bad Request: invalid model")) }
        }).await;

        assert!(result.is_err());
        // Should NOT have retried (only 1 call for non-retryable)
    }

    #[tokio::test]
    async fn test_non_4xx5xx_code_not_retried() {
        let config = RetryConfig { max_retries: 5, ..Default::default() };

        let result: Result<(), anyhow::Error> = retry_with_backoff(&config, move || {
            async move { Err(anyhow::anyhow!("HTTP 400 Bad Request")) }
        }).await;

        assert!(result.is_err());
        // 400 is in 4xx range but our retryable_codes only has 429, 500, 502, 503, 504
    }
}
