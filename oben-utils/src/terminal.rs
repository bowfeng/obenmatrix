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
                if s.stopped { break; }
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
