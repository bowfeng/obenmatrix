use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tracing::{info, warn};

/// Standalone cron daemon — runs in a 60s tick loop, handling due jobs and
/// receiving signals, with PID file management.
#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::fmt().init();

    let base_dir = oben_cron::jobs::CronStore::default_path();
    info!("Cron daemon base dir: {:?}", base_dir);

    let store = oben_cron::jobs::CronStore::new(base_dir.clone())?;

    // Write PID file so supervisors can track the process
    let pid_path = base_dir.join("cron.pid");
    let pid = std::process::id();
    fs::write(&pid_path, pid.to_string()).with_context(|| "Write PID file")?;
    info!("PID file written: {} (pid: {})", pid_path.display(), pid);

    let running = Arc::new(AtomicBool::new(true));

    // Set up signal handlers for graceful shutdown
    let running_for_signal = Arc::clone(&running);
    tokio::spawn(async move {
        match tokio::signal::ctrl_c().await {
            Ok(()) => info!("Received SIGINT (ctrl-c), shutting down..."),
            Err(e) => warn!("Failed to register SIGINT handler: {}", e),
        }
        running_for_signal.store(false, Ordering::SeqCst);
    });

    let running_for_signal = Arc::clone(&running);
    #[cfg(unix)]
    tokio::spawn(async move {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                if signal.recv().await.is_some() {
                    info!("Received SIGTERM, shutting down...");
                    running_for_signal.store(false, Ordering::SeqCst);
                }
            }
            Err(e) => warn!("Failed to register SIGTERM handler: {}", e),
        }
    });

    // Main tick loop
    let tick_interval = Duration::from_secs(60);
    while running.load(Ordering::SeqCst) {
        tokio::time::sleep(tick_interval).await;

        if !running.load(Ordering::SeqCst) {
            break;
        }

        let due = store.get_due_jobs();
        if !due.is_empty() {
            info!("cron tick: {} job(s) due", due.len());
            for job in due {
                let id = job.id.clone();
                let name = job.name.clone();
                let ober_exec = oben_cron::cron_exec_binary();
                if let Err(e) = store.advance_job(&id, &ober_exec) {
                    warn!("Failed to process {}: {}", id, e);
                }
                info!("tick: completed cron job '{}' ({})", name, id);
            }
        }
    }

    info!("Cron daemon stopped");
    Ok(())
}
