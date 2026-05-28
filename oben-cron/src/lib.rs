//! Cron scheduler for scheduled agent task delivery.
//!
//! Supports 4 schedule types: once, interval, cron expression, ISO timestamp.
//! Jobs are persisted to JSON with automatic next-run computation and file locking.
//!
//! Example: `oben cron list` to see all jobs, `oben cron create --help` for creating.

pub mod schedule;
pub mod jobs;

/// Resolve the `obenalien` binary for cron execution.
///
/// Priority: `OBEN_BIN` env → local cargo artifacts → `which` → literal fallback.
pub fn cron_exec_binary() -> String {
    if let Ok(val) = std::env::var("OBEN_BIN") {
        return val;
    }
    // Check common cargo debug/release paths
    let pwd = std::env::current_dir().ok();
    if let Some(ref p) = pwd {
        for c in &[
            "target/debug/obenalien",
            "target/release/obenalien",
            "./target/debug/obenalien",
            "./target/release/obenalien",
        ] {
            let full_path = p.join(c);
            if full_path.exists() {
                return full_path.to_string_lossy().to_string();
            }
        }
    }
    // Fall back to PATH lookup
    if let Ok(out) = std::process::Command::new("which")
        .arg("obenalien")
        .output()
    {
        if out.status.success() {
            return String::from_utf8_lossy(&out.stdout).trim().to_string();
        }
    }
    "obenalien".to_string()
}

pub use schedule::*;
pub use jobs::*;
