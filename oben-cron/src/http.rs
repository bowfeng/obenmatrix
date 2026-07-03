//! HTTP request/response data types and client for the cron scheduler.
//!
//! This module provides both the data types for cron submission and a lightweight
//! HTTP client (`CronClient`) for daemons or other services to submit cron jobs
//! to theoben agent endpoint. No HTTP server endpoints are defined here.

use serde::{Deserialize, Serialize};

/// Lightweight HTTP client for submitting cron jobs to the agent daemon.
///
/// # Example
/// ```ignore
/// let client = CronClient::new(None);
/// let resp = client.submit(&request).await?;
/// ```
#[derive(Clone)]
pub struct CronClient {
    base_url: String,
    client: reqwest::Client,
}

impl CronClient {
    /// Create a new `CronClient` with an optional base URL.
    ///
    /// When `base_url` is `None`, defaults to `http://localhost:8790`.
    pub fn new(base_url: Option<String>) -> Self {
        let base_url = base_url.unwrap_or_else(|| "http://localhost:8790".to_string());
        Self {
            base_url,
            client: reqwest::Client::new(),
        }
    }

    /// Submit a cron job to the daemon endpoint.
    pub async fn submit(
        &self,
        request: &CronSubmitRequest,
    ) -> Result<CronSubmitResponse, reqwest::Error> {
        self.client
            .post(format!("{}/cron/submit", self.base_url))
            .json(request)
            .send()
            .await?
            .json::<CronSubmitResponse>()
            .await
    }
}

/// Request body for submitting a cron job via HTTP.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CronSubmitRequest {
    /// The prompt to execute.
    pub prompt: String,
    /// Where to deliver the result.
    #[serde(default)]
    pub deliver_target: Option<crate::DeliverTarget>,
    /// Optional session ID to associate with the job.
    #[serde(default)]
    pub session_id: Option<String>,
}

/// Response returned after a cron job is submitted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronSubmitResponse {
    /// Unique job identifier.
    pub job_id: String,
    /// Current job status.
    pub status: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serde roundtrip: serialize then deserialize — should produce the same value.
    #[test]
    fn test_submit_request_roundtrip() {
        let original = CronSubmitRequest {
            prompt: "run analysis".into(),
            deliver_target: Some(crate::DeliverTarget::Origin),
            session_id: Some("sess-abc".into()),
        };

        let json = serde_json::to_string(&original).expect("serialize");
        let roundtrip: CronSubmitRequest =
            serde_json::from_str(&json).expect("deserialize");

        assert_eq!(roundtrip.prompt, original.prompt);
        assert_eq!(
            roundtrip.deliver_target,
            original.deliver_target,
            "deliver_target should survive roundtrip"
        );
        assert_eq!(
            roundtrip.session_id, original.session_id,
            "session_id should survive roundtrip"
        );
    }

    /// Missing `prompt` must fail deserialization.
    #[test]
    fn test_missing_prompt_fails() {
        let json = r#"{"deliver_target":null}"#;
        let result: Result<CronSubmitRequest, _> = serde_json::from_str(json);
        assert!(
            result.is_err(),
            "deserialization must fail when required `prompt` field is missing"
        );
    }
}
