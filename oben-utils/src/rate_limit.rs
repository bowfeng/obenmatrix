//! Rate limit tracker for inference API responses.
//!
//! Parses `x-ratelimit-*` headers from provider responses (Nous Portal,
//! OpenRouter, OpenAI-compatible APIs) into typed state structs with
//! formatting for display.
//!
//! Supported headers — 12 fields total:
//! ```text
//! x-ratelimit-limit-requests          RPM cap
//! x-ratelimit-limit-requests-1h       RPH cap
//! x-ratelimit-limit-tokens            TPM cap
//! x-ratelimit-limit-tokens-1h         TPH cap
//! x-ratelimit-remaining-requests      requests left in minute window
//! x-ratelimit-remaining-requests-1h   requests left in hour window
//! x-ratelimit-remaining-tokens        tokens left in minute window
//! x-ratelimit-remaining-tokens-1h     tokens left in hour window
//! x-ratelimit-reset-requests          seconds until minute request window resets
//! x-ratelimit-reset-requests-1h       seconds until hour request window resets
//! x-ratelimit-reset-tokens            seconds until token minute window resets
//! x-ratelimit-reset-tokens-1h         seconds until token hour window resets
//! ```

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::SystemTime;

use once_cell::sync::Lazy;

/// One rate-limit window (e.g. requests per minute).
#[derive(Debug, Clone, PartialEq)]
pub struct RateLimitBucket {
    /// Maximum allowed requests/tokens in this window.
    pub limit: usize,
    /// Remaining requests/tokens.
    pub remaining: usize,
    /// Seconds until this window resets.
    pub reset_seconds: f64,
    /// `SystemTime` when this bucket was captured.
    captured_at: SystemTime,
}

impl RateLimitBucket {
    pub fn new(limit: usize, remaining: usize, reset_seconds: f64) -> Self {
        Self {
            limit,
            remaining,
            reset_seconds,
            captured_at: SystemTime::now(),
        }
    }

    /// Number of requests/tokens already used.
    pub fn used(&self) -> usize {
        if self.limit >= self.remaining {
            self.limit - self.remaining
        } else {
            0
        }
    }

    /// Usage percentage [0.0, 100.0].
    pub fn usage_pct(&self) -> f64 {
        if self.limit == 0 {
            0.0
        } else {
            (self.used() as f64 / self.limit as f64) * 100.0
        }
    }

    /// Estimated seconds remaining until reset, adjusted for elapsed time.
    pub fn remaining_seconds_now(&self) -> f64 {
        let elapsed = self
            .captured_at
            .elapsed()
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        (self.reset_seconds - elapsed).max(0.0)
    }
}

/// Full rate-limit state parsed from response headers.
#[derive(Debug, Clone, PartialEq)]
pub struct RateLimitState {
    /// Requests per minute window.
    pub requests_min: RateLimitBucket,
    /// Requests per hour window.
    pub requests_hour: RateLimitBucket,
    /// Tokens per minute window.
    pub tokens_min: RateLimitBucket,
    /// Tokens per hour window.
    pub tokens_hour: RateLimitBucket,
    /// Provider name (stored for display).
    pub provider: String,
    /// When state was captured.
    pub captured_at: SystemTime,
}

impl RateLimitState {
    pub fn has_data(&self) -> bool {
        self.captured_at != SystemTime::UNIX_EPOCH
    }

    /// Seconds since data was captured.
    pub fn age_seconds(&self) -> f64 {
        if !self.has_data() {
            return f64::INFINITY;
        }
        self.captured_at
            .elapsed()
            .map(|d| d.as_secs_f64())
            .unwrap_or(f64::INFINITY)
    }

    /// Check if any bucket usage is >= threshold (for warnings).
    pub fn usage_warnings(&self, threshold: f64) -> Vec<String> {
        let buckets = [
            ("requests/min", &self.requests_min),
            ("requests/hr", &self.requests_hour),
            ("tokens/min", &self.tokens_min),
            ("tokens/hr", &self.tokens_hour),
        ];
        let mut warnings = Vec::new();
        for (label, bucket) in &buckets {
            if bucket.limit > 0 && bucket.usage_pct() >= threshold {
                let reset = format_seconds(bucket.remaining_seconds_now());
                warnings.push(format!(
                    "  ⚠ {} at {:.0}% — resets in {}",
                    label,
                    bucket.usage_pct(),
                    reset
                ));
            }
        }
        warnings
    }
}

/// Parse x-ratelimit-* headers into a RateLimitState.
///
/// Returns None if no rate limit headers are present.
/// Case-insensitive header name matching.
pub fn parse_rate_limit_headers(
    headers: &HashMap<String, String>,
    provider: &str,
) -> Option<RateLimitState> {
    let lowered: HashMap<String, String> = headers
        .iter()
        .map(|(k, v)| (k.to_lowercase(), v.clone()))
        .collect();

    if !lowered.keys().any(|k| k.starts_with("x-ratelimit-")) {
        return None;
    }

    let now = SystemTime::now();

    macro_rules! bucket {
        ($resource:tt, $suffix:tt) => {{
            let tag = format!("{}{}", $resource, $suffix);
            let limit = safe_int(lowered.get(&format!("x-ratelimit-limit-{}", tag)));
            let remaining = safe_int(lowered.get(&format!("x-ratelimit-remaining-{}", tag)));
            let reset = safe_float(lowered.get(&format!("x-ratelimit-reset-{}", tag)));
            RateLimitBucket::new(limit, remaining.min(limit), reset)
        }};
    }

    Some(RateLimitState {
        requests_min: bucket!("requests", ""),
        requests_hour: bucket!("requests", "-1h"),
        tokens_min: bucket!("tokens", ""),
        tokens_hour: bucket!("tokens", "-1h"),
        provider: provider.to_string(),
        captured_at: now,
    })
}

/// Format rate limit state for terminal/chat display.
pub fn format_rate_limit_display(state: &RateLimitState) -> String {
    if !state.has_data() {
        return "No rate limit data yet — make an API request first.".to_string();
    }

    let provider = state.provider.to_uppercase();
    let freshness = state.freshness_label();

    let lines: Vec<String> = [
        format!("{} Rate Limits (captured {}):", provider, freshness),
        String::new(),
        bucket_line("Requests/min", &state.requests_min, 14),
        bucket_line("Requests/hr", &state.requests_hour, 14),
        String::new(),
        bucket_line("Tokens/min", &state.tokens_min, 14),
        bucket_line("Tokens/hr", &state.tokens_hour, 14),
    ]
    .into_iter()
    .filter(|_l| true)
    .collect();

    let mut output = lines.join("\n");

    let warnings = state.usage_warnings(80.0);
    if !warnings.is_empty() {
        output.push_str("\n\n");
        output.push_str(&warnings.join("\n"));
    }

    output
}

/// One-line compact summary for status bars / gateway messages.
pub fn format_rate_limit_compact(state: &RateLimitState) -> String {
    if !state.has_data() {
        return "No rate limit data.".to_string();
    }

    let mut parts = Vec::new();

    if state.requests_min.limit > 0 {
        parts.push(format!(
            "RPM: {}/{}",
            state.requests_min.remaining, state.requests_min.limit
        ));
    }
    if state.requests_hour.limit > 0 {
        parts.push(format!(
            "RPH: {} (resets {})",
            fmt_count(state.requests_hour.remaining),
            format_seconds(state.requests_hour.remaining_seconds_now())
        ));
    }
    if state.tokens_min.limit > 0 {
        parts.push(format!(
            "TPM: {} ({})",
            fmt_count(state.tokens_min.remaining),
            fmt_count(state.tokens_min.limit)
        ));
    }
    if state.tokens_hour.limit > 0 {
        parts.push(format!(
            "TPH: {} (resets {})",
            fmt_count(state.tokens_hour.remaining),
            format_seconds(state.tokens_hour.remaining_seconds_now())
        ));
    }

    parts.join(" | ")
}

/// Thread-safe global store for last-seen rate limit state per provider.
pub static RATE_LIMIT_STORE: Lazy<RateLimitStore> = Lazy::new(|| RateLimitStore {
    map: Mutex::new(HashMap::new()),
});

/// A store that maps provider names to their latest rate limit state.
#[derive(Debug)]
pub struct RateLimitStore {
    map: Mutex<HashMap<String, RateLimitState>>,
}

impl RateLimitStore {
    /// Save rate limit state for a provider.
    pub fn save(&self, provider: &str, state: RateLimitState) {
        let mut map = self.map.lock().unwrap();
        map.insert(provider.to_string(), state);
    }

    /// Get the latest rate limit state for a provider.
    pub fn get(&self, provider: &str) -> Option<RateLimitState> {
        let map = self.map.lock().unwrap();
        map.get(provider).cloned()
    }

    /// Remove all state.
    pub fn clear(&self) {
        let mut map = self.map.lock().unwrap();
        map.clear();
    }
}

// ── Formatting helpers ──────────────────────────────────────────────────

fn fmt_count(n: usize) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

fn format_seconds(seconds: f64) -> String {
    let s = seconds.max(0.0) as u64;
    if s < 60 {
        return format!("{}s", s);
    }
    let m = s / 60;
    let sec = s % 60;
    if m < 60 {
        return if sec > 0 {
            format!("{}m {}s", m, sec)
        } else {
            format!("{}m", m)
        };
    }
    let h = m / 60;
    let m = m % 60;
    if m > 0 {
        format!("{}h {}m", h, m)
    } else {
        format!("{}h", h)
    }
}

fn bar(pct: f64, width: usize) -> String {
    let filled = (pct / 100.0 * width as f64).floor() as usize;
    let filled = filled.min(width);
    format!(
        "[{}] {:.1}%",
        "█".repeat(filled) + &"░".repeat(width - filled),
        pct
    )
}

fn bucket_line(label: &str, bucket: &RateLimitBucket, label_width: usize) -> String {
    if bucket.limit == 0 {
        return format!("  {:<width$}  (no data)", label, width = label_width);
    }

    let pct = bucket.usage_pct();
    let used = fmt_count(bucket.used());
    let limit = fmt_count(bucket.limit);
    let remaining = fmt_count(bucket.remaining);
    let reset = format_seconds(bucket.remaining_seconds_now());
    let bar_str = bar(pct, 14);

    format!(
        "  {:<width$} {} {}% {}/{} ({}, resets in {})",
        label,
        bar_str,
        pct,
        used,
        limit,
        remaining,
        reset,
        width = label_width,
    )
}

impl RateLimitState {
    fn freshness_label(&self) -> String {
        let age = self.age_seconds();
        if age < 5.0 {
            "just now".to_string()
        } else if age < 60.0 {
            format!("{}s ago", age as u64)
        } else {
            format!("{} ago", format_seconds(age))
        }
    }
}

fn safe_int(value: Option<&String>) -> usize {
    value
        .map(|v| v.parse::<i64>().unwrap_or(0).max(0) as usize)
        .unwrap_or(0)
}

fn safe_float(value: Option<&String>) -> f64 {
    value.and_then(|v| v.parse().ok()).unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_headers() -> HashMap<String, String> {
        [
            ("x-ratelimit-limit-requests".into(), "800".into()),
            ("x-ratelimit-limit-requests-1h".into(), "33600".into()),
            ("x-ratelimit-limit-tokens".into(), "8000000".into()),
            ("x-ratelimit-limit-tokens-1h".into(), "336000000".into()),
            ("x-ratelimit-remaining-requests".into(), "795".into()),
            ("x-ratelimit-remaining-requests-1h".into(), "33590".into()),
            ("x-ratelimit-remaining-tokens".into(), "7999500".into()),
            ("x-ratelimit-remaining-tokens-1h".into(), "335999000".into()),
            ("x-ratelimit-reset-requests".into(), "45.5".into()),
            ("x-ratelimit-reset-requests-1h".into(), "3500.0".into()),
            ("x-ratelimit-reset-tokens".into(), "42.3".into()),
            ("x-ratelimit-reset-tokens-1h".into(), "3490.0".into()),
        ]
        .into_iter()
        .collect()
    }

    #[test]
    fn test_parse_nous_headers() {
        let headers = sample_headers();
        // add header key casing
        let state = parse_rate_limit_headers(&headers, "nous");
        assert!(state.is_some());
        let state = state.unwrap();
        assert_eq!(state.provider, "nous");
        assert!(state.has_data());
        assert_eq!(state.requests_min.limit, 800);
        assert_eq!(state.requests_min.remaining, 795);
        assert_eq!(state.requests_hour.limit, 33600);
        assert_eq!(state.requests_hour.remaining, 33590);
        assert_eq!(state.tokens_min.limit, 8000000);
        assert!(state.tokens_hour.limit > 0);
    }

    #[test]
    fn test_parse_no_headers() {
        let result = parse_rate_limit_headers(&HashMap::new(), "test");
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_partial_headers() {
        let mut headers = HashMap::new();
        headers.insert("x-ratelimit-limit-requests".into(), "100".into());
        headers.insert("x-ratelimit-remaining-requests".into(), "50".into());
        let state = parse_rate_limit_headers(&headers, "test");
        assert!(state.is_some());
        let state = state.unwrap();
        assert_eq!(state.requests_min.limit, 100);
        assert_eq!(state.requests_min.remaining, 50);
        assert_eq!(state.tokens_min.limit, 0);
    }

    #[test]
    fn test_bucket_used() {
        let b = RateLimitBucket::new(800, 795, 45.0);
        assert_eq!(b.used(), 5);
    }

    #[test]
    fn test_bucket_usage_pct() {
        let b = RateLimitBucket::new(100, 20, 30.0);
        assert!((b.usage_pct() - 80.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_bucket_usage_pct_zero_limit() {
        let b = RateLimitBucket::new(0, 0, 0.0);
        assert_eq!(b.usage_pct(), 0.0);
    }

    #[test]
    fn test_remaining_seconds_now() {
        let now = SystemTime::now();
        let mut b = RateLimitBucket::new(800, 795, 60.0);
        b.captured_at = now - std::time::Duration::from_secs(10);
        let elapsed = b.captured_at.elapsed().unwrap();
        let remaining = b.remaining_seconds_now();
        assert!(remaining >= 49.0 && remaining <= 51.0 + elapsed.as_secs_f64());
    }

    #[test]
    fn test_remaining_seconds_expired() {
        let mut b = RateLimitBucket::new(800, 795, 30.0);
        b.captured_at = SystemTime::now() - std::time::Duration::from_secs(60);
        assert_eq!(b.remaining_seconds_now(), 0.0);
    }

    #[test]
    fn test_format_count_millions() {
        assert_eq!(fmt_count(8000000), "8.0M");
        assert_eq!(fmt_count(336000000), "336.0M");
    }

    #[test]
    fn test_format_count_thousands() {
        assert_eq!(fmt_count(33600), "33.6K");
        assert_eq!(fmt_count(1500), "1.5K");
    }

    #[test]
    fn test_format_count_small() {
        assert_eq!(fmt_count(800), "800");
        assert_eq!(fmt_count(0), "0");
    }

    #[test]
    fn test_format_seconds_seconds() {
        assert_eq!(format_seconds(45.0), "45s");
        assert_eq!(format_seconds(0.0), "0s");
    }

    #[test]
    fn test_format_seconds_minutes() {
        assert_eq!(format_seconds(125.0), "2m 5s");
        assert_eq!(format_seconds(120.0), "2m");
    }

    #[test]
    fn test_format_seconds_hours() {
        assert_eq!(format_seconds(3660.0), "1h 1m");
        assert_eq!(format_seconds(3600.0), "1h");
    }

    #[test]
    fn test_bar() {
        assert_eq!(bar(50.0, 10), "[█████░░░░░] 50.0%");
        assert!(bar(0.0, 10).contains("░"));
        assert!(bar(100.0, 10).contains("█"));
    }

    #[test]
    fn test_format_display_no_data() {
        let state = RateLimitState {
            requests_min: RateLimitBucket::new(0, 0, 0.0),
            requests_hour: RateLimitBucket::new(0, 0, 0.0),
            tokens_min: RateLimitBucket::new(0, 0, 0.0),
            tokens_hour: RateLimitBucket::new(0, 0, 0.0),
            provider: "test".into(),
            captured_at: SystemTime::UNIX_EPOCH,
        };
        let result = format_rate_limit_display(&state);
        assert!(result.contains("No rate limit data"));
    }

    #[test]
    fn test_format_display_with_data() {
        let headers = sample_headers();
        let state = parse_rate_limit_headers(&headers, "nous").unwrap();
        let result = format_rate_limit_display(&state);
        assert!(result.contains("NOUS"));
        assert!(result.contains("Requests/min"));
        assert!(result.contains("Tokens/hr"));
        assert!(result.contains("resets in"));
    }

    #[test]
    fn test_format_display_warning_on_high_usage() {
        let mut headers = sample_headers();
        headers.insert("x-ratelimit-remaining-requests".into(), "50".into());
        let state = parse_rate_limit_headers(&headers, "nous").unwrap();
        let result = format_rate_limit_display(&state);
        assert!(result.contains("⚠"));
    }

    #[test]
    fn test_format_compact() {
        let headers = sample_headers();
        let state = parse_rate_limit_headers(&headers, "nous").unwrap();
        let result = format_rate_limit_compact(&state);
        assert!(result.contains("RPM:"));
        assert!(result.contains("RPH:"));
        assert!(result.contains("TPM:"));
        assert!(result.contains("TPH:"));
    }

    #[test]
    fn test_format_compact_no_data() {
        let state = RateLimitState {
            requests_min: RateLimitBucket::new(0, 0, 0.0),
            requests_hour: RateLimitBucket::new(0, 0, 0.0),
            tokens_min: RateLimitBucket::new(0, 0, 0.0),
            tokens_hour: RateLimitBucket::new(0, 0, 0.0),
            provider: "test".into(),
            captured_at: SystemTime::UNIX_EPOCH,
        };
        let result = format_rate_limit_compact(&state);
        assert_eq!(result, "No rate limit data.");
    }

    #[test]
    fn test_usage_warnings() {
        let mut headers = sample_headers();
        headers.insert("x-ratelimit-remaining-requests".into(), "50".into());
        let state = parse_rate_limit_headers(&headers, "nous").unwrap();
        let warnings = state.usage_warnings(80.0);
        assert!(!warnings.is_empty());
        assert!(warnings[0].contains("requests/min"));
    }

    #[test]
    fn test_usage_warnings_no_threshold_reached() {
        let headers = sample_headers();
        let state = parse_rate_limit_headers(&headers, "nous").unwrap();
        let warnings = state.usage_warnings(99.0);
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_rate_limit_store() {
        RATE_LIMIT_STORE.clear();
        let headers = sample_headers();
        let state = parse_rate_limit_headers(&headers, "nous").unwrap();
        RATE_LIMIT_STORE.save("nous", state.clone());

        let got = RATE_LIMIT_STORE.get("nous");
        assert!(got.is_some());
        assert_eq!(got.unwrap().provider, "nous");

        let missing = RATE_LIMIT_STORE.get("openai");
        assert!(missing.is_none());

        RATE_LIMIT_STORE.clear();
        assert!(RATE_LIMIT_STORE.get("nous").is_none());
    }

    #[test]
    fn test_case_insensitive_headers() {
        let mut headers = HashMap::new();
        headers.insert("X-RateLimit-Limit-Requests".into(), "100".into());
        headers.insert("X-Ratelimit-Remaining-Requests".into(), "90".into());
        let state = parse_rate_limit_headers(&headers, "test");
        assert!(state.is_some());
        assert_eq!(state.unwrap().requests_min.limit, 100);
    }
}
