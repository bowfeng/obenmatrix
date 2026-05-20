//! ObenAgent — main entry point.

use tokio::runtime::Runtime;

fn main() {
    let rt = Runtime::new().expect("Failed to create Tokio runtime");
    if let Err(e) = rt.block_on(oben_cli::run_cli()) {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}
