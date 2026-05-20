//! ObenAgent CLI — clap parsing + all command implementations.
//!
//! Every CLI command handler lives here. Domain crates provide types and
//! business logic only; this crate handles parsing, wiring, and output.

pub mod cli;
pub mod dispatch;
pub use dispatch::run_cli;
