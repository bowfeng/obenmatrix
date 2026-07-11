pub mod clarify;
pub mod code_execution;
pub mod computer_use;
pub mod delegate;
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
pub mod toolset_filter;
pub mod stt;
pub mod tts;
pub mod vision_analyze;
pub mod voice;
pub mod web;
pub mod web_extract;
pub mod image_gen;
pub mod video_gen;
pub mod mcp_client;
pub mod file_sync;
pub mod cron_delivery;
pub mod kanban;

pub use registry::*;
pub use terminal::*;

// ---------------------------------------------------------------------------
// Auto-registration
///
/// Each tool module provides a free `register(registry: &mut ToolRegistry)`
/// function.
///
/// To add a new tool:
///   1. Create `oben-tools/src/my_tool.rs` implementing `Tool`
///   2. Add a free `pub fn register(registry: &mut ToolRegistry)` in that file
///   3. Add `my_tool::register` to `ALL_TOOLS` below
///   4. Done — `discover_builtin_tools` picks it up automatically.
// ---------------------------------------------------------------------------

/// All builtin tool modules, in load order.
pub struct BuiltinTools;

impl BuiltinTools {
    /// Register all builtin tools into the given registry.
    pub fn register_all(registry: &mut ToolRegistry) {
        terminal::register(registry);
        read_write::register(registry);
        web::register(registry);
        search::register(registry);
        search_files::register(registry);
        patch::register(registry);
        web_extract::register(registry);
        vision_analyze::register(registry);
        clarify::register(registry);
        todo::register(registry);
        code_execution::register(registry);
        osv_check::register(registry);
        skill::register(registry);
        computer_use::register(registry);
        delegate::register(registry);
        tts::register(registry);
        stt::register(registry);
        image_gen::register(registry);
        video_gen::register(registry);
        mcp_client::register(registry);
        file_sync::register(registry);
        cron_delivery::register(registry);
        kanban::register(registry);
    }
}
