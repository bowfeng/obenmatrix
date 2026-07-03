//! HTTP server for cron job submission.
//!
//! Exposes a single `POST /cron/submit` endpoint that accepts a
//! [`CronSubmitRequest`], validates the prompt, creates a new [`CronJob`],
//! and persists it via the [`CronStore`].

use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    response::Json,
    Router,
};
use tokio::net::TcpListener;
use tower_http::cors::{Any, CorsLayer};
use tracing::{info, warn};

use crate::http::{CronSubmitRequest, CronSubmitResponse};
use crate::jobs::{CronJob, CronStore, scan_cron_prompt};

/// Run the HTTP server with the given socket address and shared cron store.
pub async fn run_server(store: Arc<CronStore>, addr: std::net::SocketAddr) {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([axum::http::Method::POST, axum::http::Method::OPTIONS])
        .allow_headers([axum::http::header::CONTENT_TYPE]);

    let app = Router::new()
        .route("/cron/submit", axum::routing::post(submit_handler))
        .layer(cors)
        .with_state(store);

    let listener = TcpListener::bind(addr).await.expect("bind TCP listener");
    info!("HTTP server listening on {}", listener.local_addr().unwrap());

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("server error");
}

/// Graceful shutdown: wait for SIGINT (Ctrl-C).
async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to listen for ctrl-c");
    info!("Received shutdown signal");
}

/// Handle `POST /cron/submit`.
///
/// Validates the prompt against injection patterns, creates a new job,
/// and persists it to the store.
async fn submit_handler(
    State(store): State<Arc<CronStore>>,
    Json(req): Json<CronSubmitRequest>,
) -> Result<Json<CronSubmitResponse>, (StatusCode, String)> {
    info!("Received request on /cron/submit");

    // Validate prompt against known injection / exfiltration patterns.
    if let Err(e) = scan_cron_prompt(&req.prompt) {
        warn!(error = %e, "Cron job submission blocked by scanner");
        return Err((
            StatusCode::BAD_REQUEST,
            format!("Prompt validation failed: {}", e),
        ));
    }

    // Derive a job name from the prompt; use a cron schedule that fires every 30 minutes.
    let schedule_str = "every 30m".to_string();
    let name = req
        .prompt
        .chars()
        .take(64)
        .collect::<String>();

    let job = match CronJob::new(name, req.prompt.clone(), &schedule_str, None) {
        Ok(job) => job,
        Err(e) => {
            warn!(error = %e, "Failed to create cron job");
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to create job: {}", e),
            ));
        }
    };

    if let Err(e) = store.create(job.clone()) {
        warn!(error = %e, "Failed to persist cron job");
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to save job: {}", e),
        ));
    }

    info!(job_id = job.id, "Created cron job {}", job.id);

    Ok(Json(CronSubmitResponse {
        job_id: job.id,
        status: "scheduled".to_string(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Request body roundtrip: serialize → deserialize matches original values.
    #[test]
    fn test_roundtrip_request() {
        let original = CronSubmitRequest {
            prompt: "test prompt".into(),
            deliver_target: Some(crate::DeliverTarget::Origin),
            session_id: Some("sess-1".into()),
        };

        let json = serde_json::to_string(&original).expect("serialize");
        let decoded: CronSubmitRequest =
            serde_json::from_str(&json).expect("deserialize");

        assert_eq!(decoded.prompt, original.prompt);
        assert_eq!(decoded.deliver_target, original.deliver_target);
        assert_eq!(decoded.session_id, original.session_id);
    }
}
