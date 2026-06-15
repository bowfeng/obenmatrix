/// Subagent tree — interrupt hub and child tracking.
///
/// Re-exports the `InterruptHub` and `SubagentRecord` types from the
/// interrupt_hub module, providing a coordinator-facing API for managing
/// subagent interrupt propagation.
pub use crate::interrupt_hub::{InterruptHub, SubagentRecord};
