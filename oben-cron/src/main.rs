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
    let store: Arc<oben_cron::jobs::CronStore> = Arc::new(store);

    // Write PID file so supervisors can track the process
    let pid_path = base_dir.join("cron.pid");
    let pid = std::process::id();
    fs::write(&pid_path, pid.to_string()).with_context(|| "Write PID file")?;
    info!("PID file written: {} (pid: {})", pid_path.display(), pid);

    // Start HTTP server for job submission
    let port: u16 = std::env::var("OBEN_CRON_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8790);
    let addr = format!("127.0.0.1:{}", port);
    let server_store = Arc::clone(&store);
    info!("HTTP server starting on {}", addr);
    tokio::spawn(async move {
        let parse_addr: std::net::SocketAddr = addr.parse().unwrap();
        oben_cron::server::run_server(server_store, parse_addr).await;
    });

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

    // Spawn cron tick loop as a background task
    let daemon_store = Arc::clone(&store);
    let daemon_handle = oben_cron::jobs::Daemon::spawn(
        daemon_store,
        Duration::from_secs(60),
    );

    // Wait for signal to stop
    while running.load(Ordering::SeqCst) {
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    // Stop daemon
    daemon_handle.0.stop();

    info!("Cron daemon stopped");
    Ok(())
}
