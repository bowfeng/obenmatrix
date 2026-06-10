pub mod clarify;
pub mod code_execution;
pub mod computer_use;
pub mod memory;
pub mod osv_check;
pub mod patch;
pub mod read_write;
/// Tool registry and tool execution.
///
/// Maps to `tools/registry.py`, `tools/process_registry.py`, plus individual tool modules.
pub mod registry;
pub mod search;
pub mod search_files;
pub mod skill;
pub mod terminal;
pub mod todo;
pub mod vision_analyze;
pub mod web;
pub mod web_extract;
pub mod toolset_filter;
pub mod delegate;

pub use registry::*;
pub use terminal::*;

// ---------------------------------------------------------------------------
// Auto-registration
///
/// Each tool module provides a free `register(registry: &mut ToolRegistry)`
/// function. Add the function name here to auto-discover.
///
/// To add a new tool:
///   1. Create `oben-tools/src/my_tool.rs` implementing `SelfRegisteringTool`
///   2. Add a free `pub fn register(registry: &mut ToolRegistry)` in that file
///   3. Add `my_tool::register` to `ALL_TOOLS` below
///   4. Done — `discover_builtin_tools` picks it up automatically.
// ---------------------------------------------------------------------------

/// All module registration functions, in load order.
pub static ALL_TOOLS: &[fn(&mut ToolRegistry)] = &[
    terminal::register,
    read_write::register_file_tools,
    web::register,
    search::register,
    search_files::register,
    patch::register,
    web_extract::register,
    vision_analyze::register,
    memory::register,
    clarify::register,
    todo::register,
    code_execution::register,
    osv_check::register,
    skill::register,
    computer_use::ComputerUseTool::register_self,
    delegate::register,
];
