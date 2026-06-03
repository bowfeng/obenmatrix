/// Error classification for API errors.
///
/// Categorizes LLM API errors into types that determine recovery strategy:
/// retry, fail-fast, or user message.
use std::fmt;

/// Category of an API error, determining recovery strategy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorKind {
    /// Rate limit — retry after backoff.
    RateLimit,
    /// Authentication/authorization failure — fail fast.
    Authentication,
    /// Model not found — fail fast (wrong model name).
    ModelNotFound,
    /// Bad request — input validation error.
    BadRequest,
    /// Network/timeout — retry with backoff.
    Network,
    /// Server error (5xx) — retry with backoff.
    ServerError,
    /// Response truncation — the model hit max_tokens/length.
    CompletionLength,
    /// Unknown/unclassified error.
    Other(String),
}

impl ErrorKind {
    /// Whether this error should trigger a retry.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            ErrorKind::RateLimit | ErrorKind::Network | ErrorKind::ServerError
        )
    }

    /// Whether this error should fail immediately (no retry).
    pub fn is_fatal(&self) -> bool {
        matches!(
            self,
            ErrorKind::Authentication
                | ErrorKind::ModelNotFound
                | ErrorKind::BadRequest
                | ErrorKind::CompletionLength
        )
    }

    /// Human-readable message for display to user.
    pub fn user_message(&self, detail: &str) -> String {
        match self {
            ErrorKind::RateLimit => {
                format!("Rate limit exceeded. {}", detail)
            }
            ErrorKind::Authentication => {
                format!("Authentication failed. {}", detail)
            }
            ErrorKind::ModelNotFound => {
                format!(
                    "Model not found: {}. Check the model name and provider.",
                    detail
                )
            }
            ErrorKind::BadRequest => {
                format!(
                    "Bad request: {}. Check your input or configuration.",
                    detail
                )
            }
            ErrorKind::Network => {
                format!("Network error: {}. Check your connection.", detail)
            }
            ErrorKind::ServerError => {
                format!(
                    "Server error: {}. The provider is experiencing issues.",
                    detail
                )
            }
            ErrorKind::CompletionLength => {
                format!("Response was truncated. {}", detail)
            }
            ErrorKind::Other(msg) => msg.clone(),
        }
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorKind::RateLimit => write!(f, "RateLimit"),
            ErrorKind::Authentication => write!(f, "Authentication"),
            ErrorKind::ModelNotFound => write!(f, "ModelNotFound"),
            ErrorKind::BadRequest => write!(f, "BadRequest"),
            ErrorKind::Network => write!(f, "Network"),
            ErrorKind::ServerError => write!(f, "ServerError"),
            ErrorKind::CompletionLength => write!(f, "CompletionLength"),
            ErrorKind::Other(msg) => write!(f, "{}", msg),
        }
    }
}

/// Classified API error with category and raw details.
#[derive(Debug, Clone)]
pub struct ClassifiedError {
    pub kind: ErrorKind,
    pub detail: String,
    /// HTTP status code, if available.
    pub http_code: Option<u16>,
    /// Original error message for chaining.
    pub source_msg: String,
}

impl ClassifiedError {
    pub fn new(
        kind: ErrorKind,
        detail: String,
        http_code: Option<u16>,
        source_msg: String,
    ) -> Self {
        Self {
            kind,
            detail,
            http_code,
            source_msg,
        }
    }

    pub fn from_anyhow(err: &anyhow::Error) -> Self {
        let source_msg = err.to_string();
        let http_code = extract_http_code(&source_msg);

        let kind = classify_error(&source_msg, http_code);

        // Extract a concise detail from the error message
        let detail = extract_error_detail(&source_msg, http_code);

        Self {
            kind,
            detail,
            http_code,
            source_msg,
        }
    }
}

impl std::error::Error for ClassifiedError {}

impl fmt::Display for ClassifiedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.kind.user_message(&self.detail))
    }
}

/// Classify an error message into an ErrorKind.
fn classify_error(msg: &str, http_code: Option<u16>) -> ErrorKind {
    let lower = msg.to_lowercase();

    // Check HTTP code first (most reliable)
    if let Some(code) = http_code {
        return match code {
            429 => ErrorKind::RateLimit,
            401 | 403 => ErrorKind::Authentication,
            404 => ErrorKind::ModelNotFound,
            400 => {
                // 400 can be many things — check message
                if lower.contains("completion length")
                    || lower.contains("max_tokens")
                    || lower.contains("max_completion_tokens")
                    || lower.contains("context length")
                {
                    ErrorKind::CompletionLength
                } else {
                    ErrorKind::BadRequest
                }
            }
            500 | 502 | 503 | 504 => ErrorKind::ServerError,
            _ => ErrorKind::Other(format!("HTTP {}", code)),
        };
    }

    // Fall back to message-based classification
    if lower.contains("rate limit")
        || lower.contains("rate_limited")
        || lower.contains("too many requests")
    {
        return ErrorKind::RateLimit;
    }
    if lower.contains("auth")
        || lower.contains("unauthorized")
        || lower.contains("forbidden")
        || lower.contains("invalid api key")
        || lower.contains("authentication")
    {
        return ErrorKind::Authentication;
    }
    if lower.contains("model not found")
        || lower.contains("model was deleted")
        || lower.contains("no endpoints ready")
    {
        return ErrorKind::ModelNotFound;
    }
    if lower.contains("completion length")
        || lower.contains("max_tokens")
        || lower.contains("max_completion_tokens")
        || lower.contains("context length")
    {
        return ErrorKind::CompletionLength;
    }
    if lower.contains("network")
        || lower.contains("timeout")
        || lower.contains("timed out")
        || lower.contains("connection refused")
        || lower.contains("connection reset")
        || lower.contains("connection aborted")
        || lower.contains("io error")
    {
        return ErrorKind::Network;
    }
    if lower.contains("server") || lower.contains("500") || lower.contains("internal") {
        return ErrorKind::ServerError;
    }

    ErrorKind::Other(msg.to_string())
}

/// Extract HTTP status code from an error message.
fn extract_http_code(msg: &str) -> Option<u16> {
    // Match patterns like "HTTP 429", "status code: 503", "HTTP_429", "429 "
    let patterns = [
        r"HTTP[_\s]?(\d{3})",
        r"status[_\s]?code[_\s:]?\s*(\d{3})",
        r"\b(\d{3})\b", // any 3-digit number (fallback)
    ];

    for pattern in &patterns {
        if let Ok(re) = regex::Regex::new(pattern) {
            if let Some(cap) = re.captures(msg) {
                if let Ok(code) = cap[1].parse::<u16>() {
                    if (100..=599).contains(&code) {
                        return Some(code);
                    }
                }
            }
        }
    }
    None
}

/// Extract a concise detail string from an error.
fn extract_error_detail(msg: &str, _http_code: Option<u16>) -> String {
    // Strip common prefixes
    let trimmed = msg.trim();

    // Try to get just the meaningful part
    for prefix in ["HTTP ", "status code: ", "Error: ", "error: "] {
        if trimmed.starts_with(prefix) {
            return trimmed[prefix.len()..].trim().to_string();
        }
    }

    // Truncate long messages
    if trimmed.len() > 200 {
        format!("{}...", &trimmed[..197])
    } else {
        trimmed.to_string()
    }
}

/// Check if an error is retryable based on classification.
pub fn is_retryable(err: &anyhow::Error) -> bool {
    let classified = ClassifiedError::from_anyhow(err);
    classified.kind.is_retryable()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_429_is_rate_limit() {
        let err = anyhow::anyhow!("HTTP 429: rate limited");
        let classified = ClassifiedError::from_anyhow(&err);
        assert_eq!(classified.kind, ErrorKind::RateLimit);
        assert!(classified.kind.is_retryable());
        assert!(!classified.kind.is_fatal());
    }

    #[test]
    fn test_401_is_auth() {
        let err = anyhow::anyhow!("HTTP 401: invalid API key");
        let classified = ClassifiedError::from_anyhow(&err);
        assert_eq!(classified.kind, ErrorKind::Authentication);
        assert!(!classified.kind.is_retryable());
        assert!(classified.kind.is_fatal());
    }

    #[test]
    fn test_404_is_model_not_found() {
        let err = anyhow::anyhow!("HTTP 404: model not found");
        let classified = ClassifiedError::from_anyhow(&err);
        assert_eq!(classified.kind, ErrorKind::ModelNotFound);
    }

    #[test]
    fn test_500_is_server_error() {
        let err = anyhow::anyhow!("HTTP 500: internal server error");
        let classified = ClassifiedError::from_anyhow(&err);
        assert_eq!(classified.kind, ErrorKind::ServerError);
        assert!(classified.kind.is_retryable());
    }

    #[test]
    fn test_completion_length() {
        let err = anyhow::anyhow!("maximum context length exceeded");
        let classified = ClassifiedError::from_anyhow(&err);
        assert_eq!(classified.kind, ErrorKind::CompletionLength);
        assert!(classified.kind.is_fatal());
    }

    #[test]
    fn test_network_error() {
        let err = anyhow::anyhow!("connection refused");
        let classified = ClassifiedError::from_anyhow(&err);
        assert_eq!(classified.kind, ErrorKind::Network);
        assert!(classified.kind.is_retryable());
    }

    #[test]
    fn test_timeout_error() {
        let err = anyhow::anyhow!("request timed out after 30s");
        let classified = ClassifiedError::from_anyhow(&err);
        // "timed out" matches "timeout" pattern
        assert_eq!(classified.kind, ErrorKind::Network);
    }

    #[test]
    fn test_user_message_formatting() {
        let err = ClassifiedError::new(
            ErrorKind::RateLimit,
            "429 Too Many Requests".to_string(),
            Some(429),
            "HTTP 429: rate limited".to_string(),
        );
        let msg = err.kind.user_message(&err.detail);
        assert!(msg.contains("Rate limit"));
    }

    #[test]
    fn test_unknown_error_classifies_as_other() {
        let err = anyhow::anyhow!("something weird happened");
        let classified = ClassifiedError::from_anyhow(&err);
        assert!(matches!(classified.kind, ErrorKind::Other(_)));
    }

    #[test]
    fn test_error_display() {
        let err = ClassifiedError::new(
            ErrorKind::RateLimit,
            "rate limited".to_string(),
            Some(429),
            "HTTP 429".to_string(),
        );
        let display = err.to_string();
        assert!(display.contains("Rate limit"));
    }

    #[tokio::test]
    async fn test_503_server_error_is_retryable() {
        let err = anyhow::anyhow!("HTTP 503: service unavailable");
        let classified = ClassifiedError::from_anyhow(&err);
        assert!(is_retryable(&err));
        assert!(classified.kind.is_retryable());
    }

    #[tokio::test]
    async fn test_400_bad_request_is_not_retryable() {
        let err = anyhow::anyhow!("HTTP 400: bad request: invalid parameter");
        let classified = ClassifiedError::from_anyhow(&err);
        assert!(!is_retryable(&err));
        assert!(classified.kind.is_fatal());
    }
}
