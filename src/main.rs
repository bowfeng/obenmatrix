//! ObenAgent — main entry point.
//!
//! Uses a multi-threaded tokio runtime so multiple Agent instances
//! can run concurrently without blocking each other.



#[tokio::main(flavor = "multi_thread")]
async fn main() {
    if let Err(e) = oben_cli::run_cli().await {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}
