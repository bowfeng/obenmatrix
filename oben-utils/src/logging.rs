//! Initialize tracing subscriber — all logs go to file, none to stdout.

use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

/// Initialize the tracing subscriber. Logs are written to `~/.obenalien/logs/oa-{datetime}.log`.
pub fn init(level: tracing::Level) {
    let log_dir = dirs::home_dir()
        .map(|d| d.join(".obenalien/logs"))
        .unwrap_or_else(|| std::path::PathBuf::from("./logs"));
    let _ = std::fs::create_dir_all(&log_dir);
    let datetime = chrono::Local::now().format("%Y%m%dT%H%M%S");
    let log_path = log_dir.join(format!("oa-{datetime}.log"));

    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .expect("Failed to open log file");

    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("oben={level}")));

    let file_layer = tracing_subscriber::fmt::layer()
        .with_target(false)
        .with_ansi(false)
        .with_writer(log_file);

    tracing_subscriber::registry()
        .with(file_layer)
        .with(env_filter)
        .init();
}
