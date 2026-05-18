//! Initialize tracing subscriber.

use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

/// Initialize the tracing subscriber with the given log level.
/// The `RUST_LOG` environment variable overrides the level argument.
pub fn init(level: tracing::Level) {
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_target(false)
        .with_ansi(true);

    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("oben={level}")));

    tracing_subscriber::registry()
        .with(fmt_layer)
        .with(env_filter)
        .init();
}

pub fn init_test() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .try_init();
}
