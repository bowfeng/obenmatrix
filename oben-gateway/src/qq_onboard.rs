//! QQ Bot QR code onboarding.
//!
//! Flow: generate AES bind key -> portal API create_bind_task -> poll until
//! user scans -> AES-256-GCM decrypt -> return (app_id, decrypted_secret).

use aes_gcm::KeyInit;
use anyhow::{anyhow, bail, Context as _, Result};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use qr2term;
use rand::RngCore;
use std::time::{Duration, Instant};
use tokio::time::sleep;
use tracing::warn;

use aes_gcm::aead::Aead;
use aes_gcm::Aes256Gcm;

/// Result of a successful QR-code onboarding flow.
#[derive(Debug, Clone)]
pub struct QQOnboardResult {
    /// QQ Bot App ID.
    pub app_id: String,
    /// Decrypted client secret.
    pub client_secret: String,
    /// OpenID of the user who scanned the QR code.
    pub user_openid: Option<String>,
}

type BindResult = (i32, Option<String>, Option<String>, Option<String>);

async fn create_bind_task(client: &reqwest::Client) -> Result<(String, String)> {
    let mut buf = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut buf);
    let bind_key = STANDARD.encode(&buf);

    let body = serde_json::json!({ "key": bind_key });

    let resp = client
        .post("https://q.qq.com/lite/create_bind_task")
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .context("Failed to call create_bind_task")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let truncated: String = body.chars().take(200).collect();
        bail!("create_bind_task failed {}: {}", status, truncated);
    }

    let text = resp.text().await.context("Failed to read create_bind_task body")?;
    let json: serde_json::Value =
        serde_json::from_str(&text).context("Failed to parse create_bind_task response")?;

    let retcode: i64 = json.get("retcode").and_then(|v| v.as_i64()).unwrap_or(-1);
    if retcode != 0 {
        let msg = json.get("msg").and_then(|v| v.as_str()).unwrap_or("unknown");
        bail!("create_bind_task retcode={retcode}: {msg}");
    }

    // task_id is nested under response.data (matching Hermes: data.get("data", {}).get("task_id"))
    let task_id = json
        .get("data")
        .and_then(|d| d.get("task_id"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("Missing task_id in response: {}", text))?
        .to_string();

    Ok((task_id, bind_key))
}

async fn poll_bind_result(client: &reqwest::Client, task_id: &str) -> Result<BindResult> {
    let body = serde_json::json!({ "task_id": task_id });

    let resp = client
        .post("https://q.qq.com/lite/poll_bind_result")
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .context("Failed to call poll_bind_result")?;

    let status = resp.status();
    let text = resp.text().await?;

    if !status.is_success() {
        warn!("poll http error: {}", &text[..text.len().min(100)]);
        return Err(anyhow!("HTTP error"));
    }

    let json: serde_json::Value = serde_json::from_str(&text)?;
    if json.get("retcode").and_then(|v| v.as_i64()).unwrap_or(-1) != 0 {
        let msg = json.get("msg").and_then(|v| v.as_str()).unwrap_or("unknown");
        warn!("poll retcode: {}", msg);
        return Err(anyhow!("retcode: {msg}"));
    }

    let data = json
        .get("data")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();

    let status: i32 = data.get("status").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let appid = data.get("bot_appid").and_then(|v| v.as_str()).map(|s| s.to_string());
    let enc = data.get("bot_encrypt_secret").and_then(|v| v.as_str()).map(|s| s.to_string());
    let uid = data.get("openid").and_then(|v| v.as_str()).map(|s| s.to_string());

    Ok((status, appid, enc, uid))
}

fn decrypt_secret(encrypted_b64: &str, key_b64: &str) -> Result<String> {
    let key = STANDARD.decode(key_b64)?;
    let raw = STANDARD.decode(encrypted_b64)?;

    if raw.len() < 29 {
        bail!("Encrypted secret too short: {} bytes", raw.len());
    }

    let iv: [u8; 12] = raw[..12].try_into().map_err(|_| anyhow!("Bad IV"))?;
    let ct_tag = &raw[12..];

    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|_| anyhow!("Bad AES key"))?;
    let pt = cipher
        .decrypt(&iv.into(), ct_tag)
        .map_err(|e| anyhow!("AES-GCM decrypt failed: {e}"))?;

    String::from_utf8(pt).map_err(|e| anyhow!("Not valid UTF-8: {e}"))
}

fn build_qr_url(task_id: &str) -> String {
    format!(
        "https://q.qq.com/qqbot/openclaw/connect.html?task_id={task_id}&_wv=2&source=obenmatrix"
    )
}

/// Run the QR scan-to-configure flow for QQ Bot.
///
/// Returns on success with app_id, decrypted client_secret, and user_openid.
/// Returns error on timeout (600s), cancellation, or max refreshes (3).
pub async fn onboard_qq_bot() -> Result<QQOnboardResult> {
    let client = reqwest::Client::new();
    let max_refreshes = 3usize;
    let deadline = Instant::now() + Duration::from_secs(600);

    for refresh in 0..=max_refreshes {
        if refresh > 0 {
            warn!("\nQR expired. Retry {refresh}/{max_refreshes}...");
            if refresh >= max_refreshes {
                bail!("Max refreshes ({max_refreshes}) reached");
            }
        }

        let (task_id, bind_key) = create_bind_task(&client).await?;
        let qr_url = build_qr_url(&task_id);

        println!("\n  Scan this QR code with your phone QQ app:");
        println!();
        let _ = qr2term::print_qr(&qr_url);
        println!("  Or open this URL directly on your phone:");
        println!("  {qr_url}");

        let remaining = deadline.saturating_duration_since(Instant::now()).as_secs();
        print!("\n  Waiting for scan ({remaining}s)... Ctrl-C to cancel.\n");

        let mut last_status: Option<i32> = None;

        while Instant::now() < deadline {
            let (status, app_id, enc_secret, user_openid) = match poll_bind_result(&client, &task_id).await {
                Ok(r) => r,
                Err(e) => {
                    warn!("Poll error: {e}");
                    sleep(Duration::from_secs(2)).await;
                    continue;
                }
            };

            if last_status != Some(status) {
                let msg: &str = match status {
                    0 => "Waiting for scan...",
                    1 => "QR scanned, confirm on phone",
                    2 => "Complete!",
                    3 => "Cancelled",
                    _ => "Unknown status",
                };
                println!("  [scan] {msg}");
                last_status = Some(status);
            }

            if status == 2 {
                let encrypted = enc_secret.ok_or_else(|| {
                    anyhow!("Missing encrypted_secret in poll response")
                })?;
                let client_secret = decrypt_secret(&encrypted, &bind_key)
                    .context("Failed to decrypt client_secret")?;

                println!("\n  QR scan complete!");
                println!(
                    "  App ID:     {}",
                    app_id.clone().unwrap_or_else(|| "<unknown>".into())
                );
                if let Some(ref uid) = user_openid {
                    println!("  OpenID:     {uid}");
                }

                return Ok(QQOnboardResult {
                    app_id: app_id.unwrap_or_default(),
                    client_secret,
                    user_openid,
                });
            }

            if status == 3 {
                println!("\n  QR code expired or cancelled.");
                break;
            }

            sleep(Duration::from_secs(2)).await;
        }
    }

    bail!("QR scan timed out after 600 seconds")
}
