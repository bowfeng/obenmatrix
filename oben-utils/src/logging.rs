//! Initialize tracing subscriber — all logs go to file, none to stdout.

use std::io::Write;
use std::sync::Mutex;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

static LOG_PATH: Mutex<Option<String>> = Mutex::new(None);

const MAX_LOG_SIZE: u64 = 100 * 1024 * 1024;

fn rotate_logs(log_dir: &std::path::Path) {
    let mut log_files: Vec<_> = std::fs::read_dir(log_dir)
        .unwrap_or_else(|_| panic!("Failed to read log directory: {}", log_dir.display()))
        .filter_map(|e| e.ok())
        .filter(|entry| {
            entry.path().extension().is_none()
                && entry.file_name().to_string_lossy().starts_with("oa-")
        })
        .collect();

    log_files.sort_by(|a, b| {
        let modified_b = std::fs::metadata(b.path()).and_then(|m| m.modified());
        let modified_a = std::fs::metadata(a.path()).and_then(|m| m.modified());
        match (modified_b, modified_a) {
            (Ok(b_time), Ok(a_time)) => b_time.cmp(&a_time).reverse(),
            _ => std::cmp::Ordering::Equal,
        }
    });

    for old_file in &log_files[5..] {
        let _ = std::fs::remove_file(old_file.path());
    }
}

/// Initialize the tracing subscriber. Logs are written to `~/.obenmatrix/logs/oa-{datetime}.log`.
/// Returns the log path so callers can use it for other purposes (e.g. panic hooks).
pub fn init(level: tracing::Level) -> String {
    let log_dir = dirs::home_dir()
        .map(|d| d.join(".obenmatrix/logs"))
        .unwrap_or_else(|| std::path::PathBuf::from("./logs"));
    let _ = std::fs::create_dir_all(&log_dir);

    let current_log = log_dir.join("oa-current.log");
    if current_log.try_exists().unwrap_or(false)
        && std::fs::metadata(&current_log)
            .map(|m| m.len() > MAX_LOG_SIZE)
            .unwrap_or(false)
    {
        let datetime = chrono::Local::now().format("%Y%m%dT%H%M%S");
        let backup_path = log_dir.join(format!("oa-{}.log", datetime));
        let _ = std::fs::rename(&current_log, &backup_path);
    }

    rotate_logs(&log_dir);

    let datetime = chrono::Local::now().format("%Y%m%dT%H%M%S");
    let log_path = log_dir.join(format!("oa-{datetime}.log"));
    let log_path_str = log_path.to_str().unwrap().to_string();

    let mut guard = LOG_PATH.lock().unwrap();
    *guard = Some(log_path_str.clone());

    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .expect("Failed to open log file");

    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("oben={level}")));

    let file_layer = tracing_subscriber::fmt::layer()
        .with_target(false)
        .with_file(true)
        .with_line_number(true)
        .with_thread_ids(true)
        .with_ansi(false)
        .with_writer(log_file);

    tracing_subscriber::registry()
        .with(file_layer)
        .with(env_filter)
        .init();

    log_path_str
}

/// Install a panic hook that writes panic output to the log file.
///
/// Call this **after** `init()`.
pub fn init_panic_hook() {
    let _default = std::panic::take_hook();
    let log_path = {
        let guard = LOG_PATH.lock().unwrap();
        guard
            .clone()
            .unwrap_or_else(|| "/tmp/panic.log".to_string())
    };
    std::panic::set_hook(Box::new(move |info: &std::panic::PanicHookInfo<'_>| {
        let message = if let Some(s) = info.payload().downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "unknown panic".to_string()
        };

        let location = if let Some(loc) = info.location() {
            format!("{}:{}:{}", loc.file(), loc.line(), loc.column())
        } else {
            "unknown location".to_string()
        };

        let backtrace = std::backtrace::Backtrace::force_capture();

        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&log_path)
            .unwrap_or_else(|_| {
                std::fs::OpenOptions::new()
                    .append(true)
                    .create(true)
                    .open("/tmp/panic.log")
                    .unwrap()
            });

        let _ = writeln!(file, "=== PANIC at {} ===\n{}", location, message);
        let _ = file.write_all(format!("\nBacktrace:\n{}\n", backtrace).as_bytes());
    }));
}
