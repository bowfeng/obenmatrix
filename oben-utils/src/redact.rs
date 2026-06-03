//! Secret and PII redaction.

use once_cell::sync::Lazy;
use regex::Regex;
use std::sync::atomic::{AtomicBool, Ordering};

static REDACT_ENABLED: AtomicBool = AtomicBool::new(true);

fn mask_secret(value: &str, head: usize, tail: usize, floor: usize, placeholder: &str) -> String {
    if value.is_empty() {
        return String::new();
    }
    if value.len() < floor {
        return placeholder.to_string();
    }
    let tail_start = value.len().saturating_sub(tail);
    let head_part = &value[..head.min(value.len())];
    if tail_start > head {
        format!("{}...{}", head_part, &value[tail_start..])
    } else {
        placeholder.to_string()
    }
}

static REDACT_PATTERNS: Lazy<Vec<(&'static str, Regex)>> = Lazy::new(|| {
    vec![
        ("api_key", Regex::new(r"(?:sk[-_][a-zA-Z0-9_-]{20,}|ghp_[a-zA-Z0-9]{30,}|gho_[a-zA-Z0-9]{30,}|ghu_[a-zA-Z0-9]{30,}|AKIA[A-Z0-9]{16})").unwrap()),
        ("env_key", Regex::new(r"(API_KEY|AUTH_TOKEN|ACCESS_KEY|SECRET_KEY|PRIVATE_KEY|CLIENT_SECRET)=([\S]+)").unwrap()),
        ("json_api_key", Regex::new(r#""api_key"\s*:\s*"([^"]{8,})""#).unwrap()),
        ("auth_header", Regex::new(r"Authorization:\s*Bearer\s+[^\s]+").unwrap()),
        ("private_key_start", Regex::new(r"(-----BEGIN[A-Z ]*PRIVATE KEY-----)[\s\S]*?(-----END[A-Z ]*PRIVATE KEY-----)").unwrap()),
        ("db_conn", Regex::new(r"((?:postgres(?:ql)?|mysql|mongodb(?:\+srv)?|redis|amqp|amqps?)://[^:/@]+:)([^@/]+)(@)").unwrap()),
        ("jwt", Regex::new(r"eyJ[A-Za-z0-9_-]{10,}\.eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}").unwrap()),
        ("url_creds", Regex::new(r"(https?://)[^@]+:([^@/]+)@").unwrap()),
        ("params_token", Regex::new(r"((?:access_token|refresh_token|api_key|client_secret|private_key|password|secret|auth)=)[^\s&]+").unwrap()),
        ("discord", Regex::new(r"<@!?\d{17,20}>").unwrap()),
        ("phone", Regex::new(r"\+(?:1|7)?[\s-]?\(?\d{3}\)?[\s-]?\d{3}[\s-]?\d{4}").unwrap()),
    ]
});

pub fn redact_sensitive_text(text: &str, force: bool) -> String {
    let enabled = REDACT_ENABLED.load(Ordering::Relaxed);
    if !enabled && !force {
        return text.to_string();
    }

    let mut result = text.to_string();

    for (name, pattern) in REDACT_PATTERNS.iter() {
        if !pattern.is_match(&result) {
            continue;
        }
        match *name {
            "api_key" => {
                let new = pattern
                    .replace_all(&result, |caps: &regex::Captures| {
                        mask_secret(&caps[0], 4, 4, 12, "[REDACTED]")
                    })
                    .into_owned();
                if new != result {
                    result = new;
                    continue;
                }
            }
            "env_key" => {
                let new = pattern.replace_all(&result, "$1=[REDACTED]").into_owned();
                if new != result {
                    result = new;
                    continue;
                }
            }
            "json_api_key" => {
                let new = pattern
                    .replace_all(&result, "\"api_key\": \"[REDACTED]\"")
                    .into_owned();
                if new != result {
                    result = new;
                    continue;
                }
            }
            "auth_header" => {
                let new = pattern
                    .replace_all(&result, "Authorization: Bearer [REDACTED]")
                    .into_owned();
                if new != result {
                    result = new;
                    continue;
                }
            }
            "private_key_start" => {
                if pattern.is_match(&result) {
                    result = "[REDACTED PRIVATE KEY]".to_string();
                }
            }
            "db_conn" => {
                let new = pattern
                    .replace_all(&result, |caps: &regex::Captures| {
                        format!("{}***{}", &caps[1], &caps[3])
                    })
                    .into_owned();
                if new != result {
                    result = new;
                    continue;
                }
            }
            "jwt" => {
                let new = pattern.replace_all(&result, "[JWT_TOKEN]").into_owned();
                if new != result {
                    result = new;
                    continue;
                }
            }
            "url_creds" => {
                let new = pattern
                    .replace_all(&result, |caps: &regex::Captures| {
                        format!("{}***@", &caps[1])
                    })
                    .into_owned();
                if new != result {
                    result = new;
                    continue;
                }
            }
            "params_token" => {
                let new = pattern
                    .replace_all(&result, |caps: &regex::Captures| format!("{}***", &caps[1]))
                    .into_owned();
                if new != result {
                    result = new;
                    continue;
                }
            }
            "discord" => {
                let new = pattern.replace_all(&result, "<***>").into_owned();
                if new != result {
                    result = new;
                    continue;
                }
            }
            "phone" => {
                let new = pattern.replace_all(&result, "[PHONE]").into_owned();
                if new != result {
                    result = new;
                    continue;
                }
            }
            _ => {}
        }
    }
    result
}

pub fn is_redaction_enabled() -> bool {
    REDACT_ENABLED.load(Ordering::Relaxed)
}

pub fn set_redaction_enabled(enabled: bool) {
    REDACT_ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn redaction_banner() -> String {
    "[oben debug: log content redacted at upload time]".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sk_prefix_key() {
        let result = redact_sensitive_text("sk-abc123def456ghi789jkl012mno345pqr", false);
        assert!(!result.contains("sk-abc123def"));
    }

    #[test]
    fn test_ghp_token() {
        let result = redact_sensitive_text("ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghij", false);
        assert!(!result.contains("ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ"));
    }

    #[test]
    fn test_env_assignment() {
        let result = redact_sensitive_text("API_KEY=my_super_secret_key_12345 TOKEN=val", false);
        assert!(result.contains("API_KEY=[REDACTED]"));
    }

    #[test]
    fn test_db_connection_string() {
        let result =
            redact_sensitive_text("postgresql://admin:supersecret@localhost:5432/mydb", false);
        assert!(!result.contains("supersecret"));
    }

    #[test]
    fn test_jwt_redaction() {
        let result = redact_sensitive_text("eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8I", false);
        assert!(!result.contains("eyJhbGciOiJIU"));
    }

    #[test]
    fn test_auth_header() {
        let result = redact_sensitive_text("Authorization: Bearer supersecrettoken", false);
        assert!(result.contains("Authorization: Bearer [REDACTED]"));
    }

    #[test]
    fn test_private_key_block() {
        let result = redact_sensitive_text(
            "-----BEGIN RSA PRIVATE KEY-----\ndata\n-----END RSA PRIVATE KEY-----",
            false,
        );
        assert!(result.contains("[REDACTED PRIVATE KEY]"));
    }

    #[test]
    fn test_sensitive_url_params() {
        let result = redact_sensitive_text(
            "https://api.example.com?access_token=secret123&api_key=key456",
            false,
        );
        assert!(result.contains("access_token=***"));
        assert!(result.contains("api_key=***"));
    }

    #[test]
    fn test_redaction_can_be_disabled() {
        set_redaction_enabled(false);
        let result = redact_sensitive_text("API_KEY=my_secret", false);
        assert_eq!(result, "API_KEY=my_secret");
        set_redaction_enabled(true);
    }

    #[test]
    fn test_discord_snowflake() {
        let result = redact_sensitive_text("User: <@!123456789012345678>", false);
        assert!(!result.contains("123456789012345678"));
    }

    #[test]
    fn test_clean_text_unchanged() {
        let result = redact_sensitive_text("This is a normal message.", false);
        assert_eq!(result, "This is a normal message.");
    }

    #[test]
    fn test_empty_text() {
        let result = redact_sensitive_text("", false);
        assert_eq!(result, "");
    }

    #[test]
    fn test_url_userinfo() {
        let result = redact_sensitive_text("https://user:pass123@api.example.com", false);
        assert!(!result.contains("pass123"));
    }

    #[test]
    fn test_akia_prefix() {
        let result = redact_sensitive_text("AKIAIOSFODNN7EXAMPLE", false);
        assert!(
            result.is_empty() || result.contains("[REDACTED]") || !result.contains("AKIAIOSFODNN")
        );
    }

    #[test]
    fn test_json_api_key() {
        let result = redact_sensitive_text(r#"{"api_key": "my_super_secret_key"}"#, false);
        assert!(result.contains("[REDACTED]"));
        assert!(!result.contains("my_super_secret_key"));
    }
}
