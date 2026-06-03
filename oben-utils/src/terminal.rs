//! Terminal helpers — spinner, progress bar, colors.

use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// Internal shared state for the spinner.
struct SpinnerState {
    frames: Vec<&'static str>,
    index: usize,
    message: String,
    stopped: bool,
}

/// Simple spinner that prints to stderr.
pub struct Spinner {
    state: Arc<Mutex<SpinnerState>>,
    stop: Arc<std::sync::atomic::AtomicBool>,
}

impl Spinner {
    pub fn new(message: impl Into<String>) -> Self {
        let state = Arc::new(Mutex::new(SpinnerState {
            frames: vec!["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"],
            index: 0,
            message: message.into(),
            stopped: false,
        }));
        Self {
            state,
            stop: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    pub fn start(&self) {
        let stop = self.stop.clone();
        let state = self.state.clone();
        thread::spawn(move || {
            let stderr = io::stderr();
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                let mut s = state.lock().unwrap();
                if s.stopped {
                    break;
                }
                let frame = s.frames[s.index % s.frames.len()];
                s.index += 1;
                let _ = stderr.lock().flush();
                print!("\r{} {} ", frame, s.message);
                drop(s);
                thread::sleep(Duration::from_millis(80));
            }
            // Clear line
            let s = state.lock().unwrap();
            eprintln!("\r{}\x1B[2K", s.message);
        });
    }

    pub fn stop(&self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        let mut s = self.state.lock().unwrap();
        s.stopped = true;
    }
}

/// Animate a short-lived action with a spinner.
pub fn with_spinner<F, T>(message: impl Into<String>, f: F) -> T
where
    F: FnOnce() -> T,
{
    let spinner = Spinner::new(message);
    spinner.start();
    let result = f();
    spinner.stop();
    result
}

/// Print a simple table from rows of string slices.
pub fn print_table<W: Write>(headers: &[&str], rows: Vec<Vec<String>>, mut out: W) {
    if rows.is_empty() {
        return;
    }
    // Calculate column widths
    let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
    for row in &rows {
        for (i, cell) in row.iter().enumerate() {
            if i < widths.len() {
                widths[i] = widths[i].max(cell.len());
            }
        }
    }
    // Print header
    let header_line: Vec<String> = headers
        .iter()
        .enumerate()
        .map(|(i, h)| format!("{:<width$}", h, width = widths[i]))
        .collect();
    let _ = writeln!(out, "{}", header_line.join(" | "));
    // Print separator
    let sep: Vec<String> = headers.iter().map(|_| "-".repeat(widths[0])).collect();
    let _ = writeln!(out, "{}", sep.join("-"));
    // Print rows
    for row in rows {
        let line: Vec<String> = row
            .into_iter()
            .enumerate()
            .map(|(i, cell)| format!("{:<width$}", cell, width = widths[i]))
            .collect();
        let _ = writeln!(out, "{}", line.join(" | "));
    }
}

/// Convenience wrapper that prints to stderr.
pub fn print_table_stderr(headers: &[&str], rows: Vec<Vec<String>>) {
    print_table(headers, rows, io::stderr());
}
