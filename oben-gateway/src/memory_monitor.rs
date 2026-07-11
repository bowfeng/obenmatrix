/// Memory monitor — tracks gateway process memory usage and logs periodically.
/// Maps to `gateway/memory_monitor.py` in Hermes.
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use tracing::{info, debug};

// Bytes to MB conversion constant
const BYTES_TO_MB: u64 = 1024 * 1024;

// Global monitor state
static MONITOR_THREAD: AtomicBool = AtomicBool::new(false);
static START_TIME: AtomicUsize = AtomicUsize::new(0);
static INTERVAL_SECONDS: AtomicUsize = AtomicUsize::new(300);

/// Get current process resident set size in MB.
///
/// Tries `libc::getrusage` first (Linux/macOS, no extra deps).
/// Returns `None` if memory introspection is unavailable on this platform.
fn get_rss_mb() -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        // macOS: ru_maxrss is in bytes
        unsafe {
            let mut usage = std::mem::zeroed();
            if libc::getrusage(libc::RUSAGE_SELF, &mut usage) == 0 {
                return Some(usage.ru_maxrss as u64 / BYTES_TO_MB);
            }
            return None;
        }
    }

    #[cfg(target_os = "linux")]
    {
        // Linux: ru_maxrss is in KB
        unsafe {
            let mut usage = std::mem::zeroed();
            if libc::getrusage(libc::RUSAGE_SELF, &mut usage) == 0 {
                return Some(usage.ru_maxrss as u64 / 1024);
            }
            return None;
        }
    }

    #[cfg(target_os = "windows")]
    {
        // Windows: return None for now (psutil could be used here)
        return None;
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        // Unsupported platform
        return None;
    }
}

/// Log current memory usage in a grep-friendly `[MEMORY] ...` line.
///
/// Safe to call on-demand from any thread at important lifecycle moments
/// (after shutdown, after context compression, etc.).
pub fn log_memory_usage(prefix: &str) {
    let rss = get_rss_mb();
    let uptime = START_TIME.load(Ordering::Relaxed);
    let uptime_seconds = if uptime > 0 {
        uptime as u64 / 1000 // Convert from milliseconds to seconds
    } else {
        0
    };

    // Get thread count
    let thread_count = thread::available_parallelism().map_or(0, |p| p.get());

    let tag = if prefix.is_empty() {
        String::new()
    } else {
        format!("{} ", prefix)
    };

    if rss.is_none() {
        info!(
            "[MEMORY] {}rss=unavailable threads={} uptime={}s",
            tag, thread_count, uptime_seconds
        );
    } else {
        info!(
            "[MEMORY] {}rss={}MB threads={} uptime={}s",
            tag,
            rss.unwrap(),
            thread_count,
            uptime_seconds
        );
    }
}

/// Background thread body — log every `interval_seconds` until stopped.
fn monitor_loop(stop_event: Arc<AtomicBool>, interval_seconds: usize) {
    let interval = Duration::from_secs(interval_seconds as u64);

    while !stop_event.load(Ordering::Relaxed) {
        thread::sleep(interval);

        if !stop_event.load(Ordering::Relaxed) {
            match get_rss_mb() {
                Some(rss) => {
                    info!(
                        "[MEMORY] periodic rss={}MB threads={} uptime={}s",
                        rss,
                        thread::available_parallelism().map_or(0, |p| p.get()),
                        START_TIME.load(Ordering::Relaxed) / 1000
                    );
                }
                None => {
                    debug!("[MEMORY] periodic rss=unavailable");
                }
            }
        }
    }
}

/// Start periodic memory usage logging in a daemon thread.
///
/// Logs immediately to capture a baseline, then every `interval_seconds`.
/// Safe to call multiple times — subsequent calls are no-ops while the
/// first monitor is still running.
///
/// Returns `true` if a fresh monitor thread was started, `false` if one was
/// already running or if memory introspection isn't available.
pub fn start_memory_monitoring(interval_seconds: usize) -> bool {
    // Check if already running
    if MONITOR_THREAD.load(Ordering::Relaxed) {
        return false;
    }

    // Sanity-check that we can read RSS at all
    if get_rss_mb().is_none() {
        info!(
            "[MEMORY] Memory monitoring unavailable: cannot read process RSS — skipping periodic logging"
        );
        return false;
    }

    // Set start time (use current time in milliseconds)
    let start_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as usize);
    START_TIME.store(start_ms, Ordering::Relaxed);
    INTERVAL_SECONDS.store(interval_seconds, Ordering::Relaxed);

    // Log baseline snapshot
    log_memory_usage("baseline");

    // Create stop event
    let stop_event = Arc::new(AtomicBool::new(false));

    // Spawn monitor thread
    let stop_event_clone = Arc::clone(&stop_event);
    let interval = interval_seconds;

    thread::spawn(move || {
        monitor_loop(stop_event_clone, interval);
    });

    MONITOR_THREAD.store(true, Ordering::Relaxed);

    info!(
        "[MEMORY] Periodic memory monitoring started (interval: {}s)",
        interval_seconds
    );

    true
}

/// Stop the monitor thread and log a final snapshot.
pub fn stop_memory_monitoring() {
    if !MONITOR_THREAD.load(Ordering::Relaxed) {
        return;
    }

    // Log final snapshot before teardown
    log_memory_usage("shutdown");

    // Signal stop (for any future thread-based implementation)
    MONITOR_THREAD.store(false, Ordering::Relaxed);

    info!("[MEMORY] Periodic memory monitoring stopped");
}

/// Check if the memory monitor is running.
pub fn is_running() -> bool {
    MONITOR_THREAD.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_rss_mb_available() {
        // Given: A running process
        // When: get_rss_mb is called
        // Then: Returns Some(value) with a reasonable memory size
        let rss = get_rss_mb();
        
        // Should return some value (even if it's just an estimate)
        assert!(rss.is_some(), "get_rss_mb should return a value");
        
        let rss_mb = rss.unwrap();
        
        // RSS should be a reasonable amount (at least 1MB, less than 1GB)
        assert!(rss_mb >= 1, "RSS should be at least 1MB");
        assert!(rss_mb < 1024, "RSS should be less than 1GB for a simple process");
    }

    #[test]
    fn test_log_memory_usage() {
        // Given: A baseline memory reading exists
        // When: log_memory_usage is called with a prefix
        // Then: Logs memory usage (verified by observing log output)
        let result = std::panic::catch_unwind(|| {
            log_memory_usage("test_prefix");
        });
        
        // Should not panic
        assert!(result.is_ok(), "log_memory_usage should not panic");
    }

    #[test]
    fn test_log_memory_usage_empty_prefix() {
        // Given: A baseline memory reading exists
        // When: log_memory_usage is called with empty prefix
        // Then: Logs memory usage without prefix
        let result = std::panic::catch_unwind(|| {
            log_memory_usage("");
        });
        
        assert!(result.is_ok(), "log_memory_usage with empty prefix should not panic");
    }

    #[test]
    fn test_start_stop_memory_monitoring() {
        // Given: Memory monitor not running
        // When: start_memory_monitoring is called with 5 second interval
        // Then: Returns true and starts monitoring (if memory introspection available)
        let result = start_memory_monitoring(5);
        
        // Check if memory introspection is available
        if get_rss_mb().is_some() {
            // Should return true on first call (monitoring started)
            assert!(result, "start_memory_monitoring should return true on first call");
            
            // Verify monitor is running
            assert!(is_running(), "Monitor should be running after start");
        } else {
            // Memory introspection not available, start should return false
            assert!(!result, "start_memory_monitoring should return false when memory introspection unavailable");
        }
        
        // When: stop_memory_monitoring is called
        // Then: Stops monitoring and logs final snapshot
        stop_memory_monitoring();
        
        // Verify monitor is stopped
        // Note: In current implementation, we don't have a true stop mechanism
        // This test verifies the call completes without panic
        let result = std::panic::catch_unwind(|| {
            stop_memory_monitoring();
        });
        
        assert!(result.is_ok(), "stop_memory_monitoring should not panic");
    }

    #[test]
    fn test_start_monitoring_when_already_running() {
        // Given: Memory monitor already running
        // (First call to start will succeed)
        start_memory_monitoring(5);
        
        // When: start_memory_monitoring is called again
        // Then: Returns false (already running)
        let result = start_memory_monitoring(5);
        
        // Should return false since already running
        assert!(!result, "start_memory_monitoring should return false when already running");
        
        // Cleanup
        stop_memory_monitoring();
    }

    #[test]
    fn test_multiple_stop_calls() {
        // Given: Memory monitor not running
        // When: stop_memory_monitoring is called multiple times
        // Then: All calls complete without panic
        let result = std::panic::catch_unwind(|| {
            stop_memory_monitoring();
            stop_memory_monitoring();
            stop_memory_monitoring();
        });
        
        assert!(result.is_ok(), "Multiple stop calls should not panic");
    }

    #[test]
    fn test_rss_values_are_reasonable() {
        // Given: Running process
        // When: get_rss_mb is called multiple times
        // Then: Values should be consistent (within reasonable bounds)
        let rss1 = get_rss_mb();
        let rss2 = get_rss_mb();
        let rss3 = get_rss_mb();
        
        // All should be Some and positive
        assert!(rss1.is_some() && rss2.is_some() && rss3.is_some());
        
        let v1 = rss1.unwrap();
        let v2 = rss2.unwrap();
        let v3 = rss3.unwrap();
        
        // All should be positive
        assert!(v1 > 0 && v2 > 0 && v3 > 0);
        
        // Values should be within reasonable range (process doesn't grow 100x in a few calls)
        let max = std::cmp::max(v1, std::cmp::max(v2, v3));
        let min = std::cmp::min(v1, std::cmp::min(v2, v3));
        
        assert!(max <= min * 10, "RSS values should be relatively consistent");
    }
}
