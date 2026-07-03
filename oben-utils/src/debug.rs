//! Debug helpers — system info dump, log tailing, and paste upload utilities.

use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;

use crate::file_safety::read_tail;
use crate::redact::redact_sensitive_text;

/// Default max bytes to read from a log file for debug reports.
pub const DEFAULT_MAX_LOG_BYTES: usize = 512_000;
/// Default number of tail lines to capture from log files.
pub const DEFAULT_TAIL_LINES: usize = 100;

/// Capture a system info string summarizing the environment.
pub fn dump_system_info() -> String {
    let hostname = hostname::get()
        .map(|h| h.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "unknown".to_string());

    let (os_name, os_version) = if cfg!(target_os = "macos") {
        ("macOS", platform_info())
    } else if cfg!(target_os = "linux") {
        ("Linux", platform_info())
    } else if cfg!(target_os = "windows") {
        ("Windows", platform_info())
    } else {
        ("unknown", "unknown".to_string())
    };

    let rust_version = env!("CARGO_PKG_VERSION");
    let date = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_else(|_| "unknown".to_string());

    format!(
        "\
  Hostname: {}
  OS: {} (version: {})
  Platform: {}
  Rust: {}
  Date: {}
",
        hostname,
        os_name,
        os_version,
        std::env::consts::OS,
        rust_version,
        date
    )
}

fn platform_info() -> String {
    #[cfg(target_os = "macos")]
    {
        match std::process::Command::new("sw_vers")
            .args(["-productVersion"])
            .output()
        {
            Ok(out) if out.status.success() => {
                String::from_utf8_lossy(&out.stdout).trim().to_string()
            }
            _ => "unknown".to_string(),
        }
    }

    #[cfg(target_os = "linux")]
    {
        if let Ok(content) = fs::read_to_string("/etc/os-release") {
            for line in content.lines() {
                if line.starts_with("PRETTY_NAME=") {
                    return line["PRETTY_NAME=".len()..].trim_matches('"').to_string();
                }
            }
        }
        "unknown".to_string()
    }

    #[cfg(target_os = "windows")]
    {
        "unknown".to_string()
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        "unknown".to_string()
    }
}

/// Read log file tail, applying redaction.
pub fn read_log_tail(path: &Path, max_bytes: usize, tail_lines: usize) -> String {
    if !path.exists() {
        return format!("[log file not found: {}]", path.display());
    }

    match read_tail(path, max_bytes, tail_lines) {
        Ok(text) if text.is_empty() => "[log file is empty]".to_string(),
        Ok(text) => redact_sensitive_text(&text, true),
        Err(e) => format!("[error reading log: {}]", e),
    }
}

/// Read multiple log files for a debug report.
pub fn read_multiple_log_tails(paths: &[&str], max_bytes: usize, tail_lines: usize) -> String {
    let mut output = String::new();
    for path_str in paths {
        let path = Path::new(path_str);
        output.push_str(&format!("\n--- {} ---\n", path_str));
        output.push_str(&read_log_tail(path, max_bytes, tail_lines));
    }

    if output.is_empty() {
        "[no logs captured]".to_string()
    } else {
        output
    }
}

/// Compute a unique ID for a paste submission.
pub fn generate_paste_id() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    format!("{}-{:08x}", timestamp, nanos)
}

/// Extract the paste ID from a URL like "https://paste.rs/<id>".
pub fn extract_paste_id(url: &str) -> Option<&str> {
    if let Some(stripped) = url.strip_prefix("https://paste.rs/") {
        Some(stripped.trim_end_matches('/'))
    } else if let Some(stripped) = url.strip_prefix("https://paste.rs") {
        stripped.trim_end_matches('/').split('/').last()
    } else {
        None
    }
}

/// Get the directory for storing debug paste tracking files.
pub fn debug_paste_dir() -> Result<std::path::PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("No home directory"))?;
    let paste_dir = home.join(".obenmatrix/pastes");
    if !paste_dir.exists() {
        fs::create_dir_all(&paste_dir)?;
    }
    Ok(paste_dir)
}

/// Upload log content to a paste service.
pub async fn upload_to_paste(content: &str, title: Option<&str>) -> Result<String> {
    let client = reqwest::Client::new();
    match upload_to_paste_rs(&client, content, title).await {
        Ok(url) => Ok(url),
        Err(e) => {
            tracing::warn!("paste.rs upload failed: {}, trying fallback", e);
            upload_to_dpaste(&client, content, title).await
        }
    }
}

async fn upload_to_paste_rs(
    client: &reqwest::Client,
    content: &str,
    _title: Option<&str>,
) -> Result<String> {
    let mut builder = client.post("https://paste.rs");
    builder = builder.header("Content-Type", "text/plain");
    if let Some(t) = _title {
        builder = builder.header("X-Paste-Name", t);
    }
    builder = builder.header("X-Paste-Expiry", "6h");
    let resp = builder.body(content.to_string()).send().await?;
    let url = resp.text().await?;
    Ok(url.trim().to_string())
}

async fn upload_to_dpaste(
    _client: &reqwest::Client,
    _content: &str,
    _title: Option<&str>,
) -> Result<String> {
    let title = _title.unwrap_or("oben-debug");
    let resp = _client
        .post("https://dpaste.com/api/v3/")
        .body(format!("site=dpaste.com&expiry=7d&title={}", title))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .send()
        .await?;

    let code = resp.status().as_u16();
    let json: serde_json::Value = resp.json().await?;
    if let Some(url_val) = json.get("url") {
        return Ok(url_val.as_str().unwrap_or("").to_string());
    }
    anyhow::bail!("dpaste upload failed: {}", code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_system_info_format() {
        let info = dump_system_info();
        assert!(info.contains("Hostname:"));
        assert!(info.contains("OS:"));
    }

    #[test]
    fn test_extract_paste_id() {
        let id = extract_paste_id("https://paste.rs/abc123");
        assert_eq!(id, Some("abc123"));
    }

    #[test]
    fn test_extract_paste_id_with_trailing_slash() {
        let id = extract_paste_id("https://paste.rs/def456/");
        assert_eq!(id, Some("def456"));
    }

    #[test]
    fn test_extract_paste_id_non_paste() {
        let id = extract_paste_id("https://example.com/some/path");
        assert_eq!(id, None);
    }

    #[test]
    fn test_extract_paste_id_invalid() {
        let id = extract_paste_id("not-a-url");
        assert_eq!(id, None);
    }

    #[test]
    fn test_generate_paste_id() {
        let id1 = generate_paste_id();
        let id2 = generate_paste_id();
        assert!(!id1.is_empty());
        assert!(!id2.is_empty());
    }

    #[test]
    fn test_read_log_tail_nonexistent() {
        let result = read_log_tail(Path::new("/tmp/nonexistent_log_12345.txt"), 1024, 50);
        assert!(result.contains("not found"));
    }

    #[test]
    fn test_read_log_tail_empty() {
        let tmp = std::env::temp_dir().join("oben_test_empty_log.txt");
        fs::write(&tmp, "").unwrap();
        let result = read_log_tail(&tmp, 1024, 50);
        assert!(result.contains("empty"));
        let _ = fs::remove_file(&tmp);
    }
}
