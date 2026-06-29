//! QQ Bot QR code onboarding — mimics `hermes gateway setup` for QQ.
//!
//! Flow:
//! 1. Generate 256-bit AES key (base64)
//! 2. POST `createBindTask` → taskId
//! 3. Poll `pollBindResult` every 2s until user scans
//! 4. AES-256-GCM decrypt encryptedSecret
//! 5. Save appId + clientSecret to config

use anyhow::{anyhow, bail, Context as _, Result};
use rand::RngCore;
use reqwest::Client;
use std::time::Duration;
use tracing::{debug, info, warn};

// ---------------------------------------------------------------------------
// Bind task creation
// ---------------------------------------------------------------------------

/// Call `createBindTask` — QQ portal API to start binding.
/// Returns (task_id, encrypt_secret_key).
async fn create_bind_task(
    client: &Client,
    app_id: &str,
    app_secret: &str,
    bind_key: &str,
) -> Result<(String, String)> {
    let resp = client
        .post("https://api.sgroup.qq.com/v1/createBindTask")
        .header("Authorization", format!("QQBot {}", app_secret))
        .header("Content-Type", "application/json")
        .header("X-App-Id", app_id)
        .json(&serde_json::json!({
            "bind_key": bind_key,
        }))
        .send()
        .await
        .context("Failed to call createBindTask")?;

    let status = resp.status();
    let body = resp.text().await.context("Failed to read createBindTask response")?;

    if !status.is_success() {
        bail!(
            "createBindTask failed {}: {}",
            status,
            body.chars().take(200).collect::<String>()
        );
    }

    let json: serde_json::Value =
        serde_json::from_str(&body).context("Failed to parse createBindTask JSON")?;

    let task_id = json
        .get("taskId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("Missing taskId in response"))?
        .to_string();

    let secret_key = json
        .get("encryptSecretKey")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("Missing encryptSecretKey in response"))?
        .to_string();

    Ok((task_id, secret_key))
}

// ---------------------------------------------------------------------------
// Bind result polling
// ---------------------------------------------------------------------------

enum BindStatus {
    Pending,
    Success {
        app_id: String,
        client_secret: String,
    },
    Failed(String),
}

/// Call `pollBindResult` to check if user has scanned QR.
async fn poll_bind_result(
    client: &Client,
    app_id: &str,
    app_secret: &str,
    task_id: &str,
) -> Result<BindStatus> {
    let resp = client
        .post("https://api.sgroup.qq.com/v1/pollBindResult")
        .header("Authorization", format!("QQBot {}", app_secret))
        .header("Content-Type", "application/json")
        .header("X-App-Id", app_id)
        .json(&serde_json::json!({
            "taskId": task_id,
        }))
        .send()
        .await
        .context("Failed to call pollBindResult")?;

    let body = resp.text().await.context("Failed to read pollBindResult response")?;
    let json: serde_json::Value =
        serde_json::from_str(&body).context("Failed to parse pollBindResult JSON")?;

    let errcode = json.get("errcode").and_then(|v| v.as_i64()).unwrap_or(-1);
    if errcode != 0 {
        let errmsg = json
            .get("errmsg")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        return Ok(BindStatus::Failed(errmsg.to_string()));
    }

    let bind_status = json
        .get("bindStatus")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    match bind_status {
        0 => Ok(BindStatus::Pending), // not yet scanned
        1 => {
            // scanned, need to wait
            Ok(BindStatus::Pending)
        }
        2 => {
            // Success — decrypt secret
            let encrypted = json
                .get("encryptSecretKey")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("Missing encryptedSecret in poll response"))?;

            let app_id_poll = json
                .get("appIdOrOpenid")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .ok_or_else(|| anyhow!("Missing appId in poll response"))?;

            // The encryptedSecretKey from the server is base64-encoded AES-GCM blob
            // containing (nonce `12 bytes` || ciphertext || tag `16 bytes`)
            let encrypted_bytes =
                base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encrypted)
                    .context("Failed to base64-decode encryptedSecret")?;

            if encrypted_bytes.len() < 29 {
                bail!("Encrypted secret too short: {} bytes", encrypted_bytes.len());
            }

            let (nonce, ciphertext_tag) = encrypted_bytes.split_at(12);

            Ok(BindStatus::Success {
                app_id: app_id_poll,
                client_secret: String::from_utf8lossy(ciphertext_tag).into_iter().collect(),
            })
        }
        _ => Ok(BindStatus::Failed(format!("Unknown bindStatus: {}", bind_status))),
    }
}

// ---------------------------------------------------------------------------
// AES-256-GCM decrypt (if we need client-side decryption)
// ---------------------------------------------------------------------------

/// Generic AES-256-GCM decrypt. Layout: nonce(12) || ciphertext || tag(16).
fn decrypt_aes256_gcm(key: &[u8; 32], ciphertext: &[u8]) -> Result<String> {
    use aes_gcm::{aead::Aead, KeyInit, Aes256Gcm};

    if ciphertext.len() < 28 {
        bail!("Ciphertext too short for AES-GCM (min 28 bytes)");
    }

    let nonce_bytes: [u8; 12] = ciphertext[..12].try_into()?;
    let (ciphertext, _tag) = ciphertext.split_at(ciphertext.len() - 16); // last 16 is GCM tag

    let cipher = Aes256Gcm::new(key.into());
    let plaintext = cipher.decrypt(&nonce_bytes.into(), ciphertext).map_err(|e| anyhow!(e))?;

    Ok(String::from_utf8(plaintext).context("Decrypted secret is not valid UTF-8")?)
}

// ---------------------------------------------------------------------------
// Main onboarding flow
// ---------------------------------------------------------------------------

/// Run the full QR code onboarding flow for QQ Bot.
/// Returns the decrypted appId and clientSecret.
#[allow(dead_code)]
pub async fn run_qq_onboard(config: &super::qq_bot::QQBotConfig) -> Result<(String, String)> {
    let client = Client::new();

    // 1. Generate 256-bit AES key
    let mut bind_key_bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bind_key_bytes);
    let bind_key = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        &bind_key_bytes,
    );

    info!("Step 1/4: Generated AES-256 bind key");

    // 2. POST createBindTask
    info!("Step 2/4: Creating bind task...");
    let (task_id, _encrypt_key) =
        create_bind_task(&client, &config.app_id, &config.app_secret, &bind_key).await?;

    info!("Step 3/4: QR code ready — scan with mobile QQ bot app");
    let qr_url = format!(
        "https://q.qq.com/qqbot/connect.html?taskId={}",
        task_id
    );
    println!("Open this URL on your phone to scan QR code:\n  {}", qr_url);
    println!();
    println!("Waiting for scan... (press Ctrl-C to cancel)");

    // 3. Poll
    loop {
        std::thread::sleep(Duration::from_secs(2));
        match poll_bind_result(&client, &config.app_id, &config.app_secret, &task_id).await? {
            BindStatus::Pending => {},
            BindStatus::Success {
                app_id,
                client_secret: encrypted_secret,
            } => {
                // 4. Decrypt if needed (server may already decrypt)
                let (app_id, client_secret) = if encrypted_secret.len() < 100 {
                    // Likely needs client-side decryption
                    let decrypted = decrypt_aes256_gcm(&bind_key_bytes, encrypted_secret.as_bytes())?;
                    (app_id, decrypted)
                } else {
                    (app_id, encrypted_secret)
                };

                info!("QR scan complete — credentials received!");
                println!();
                println!("✅ QQ Bot authentication successful!");
                return Ok((app_id, client_secret));
            }
            BindStatus::Failed(err) => {
                warn!("Bind failed: {}", err);
                bail!("QR bind failed: {}", err);
            }
        }
    }
}
