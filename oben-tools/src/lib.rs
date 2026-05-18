/// Tool registry and tool execution.
///
/// Maps to `tools/registry.py`, `tools/process_registry.py`, plus individual tool modules.

pub mod registry;
pub mod shell;
pub mod read_write;
pub mod web;
pub mod search;

pub use registry::*;
