// ── Memory Tool Wrapper ───────────────────────────────────────────────────────
// Adapts `MemoryManager::handle_tool_call()` into `Tool` trait objects
// so memory tools (`memory.add`, `memory.replace`, `memory.remove`)
// are discoverable and executable via `ToolRegistry`.

use std::sync::Arc;

use oben_models::ToolMeta;
use oben_sessions::memory_provider::{MemoryManager, ToolSchema};
use oben_tools::registry::{Tool, ToolCall};
use oben_models::ToolResult;

/// A single memory tool that delegates to a shared `MemoryManager`.
#[derive(Clone)]
pub struct MemoryToolWrapper {
    schema: ToolSchema,
    memory_manager: Arc<std::sync::Mutex<MemoryManager>>,
}

impl MemoryToolWrapper {
    fn new(schema: ToolSchema, memory_manager: Arc<std::sync::Mutex<MemoryManager>>) -> Self {
        Self {
            schema,
            memory_manager,
        }
    }
}

#[async_trait::async_trait]
impl Tool for MemoryToolWrapper {
    fn name(&self) -> &str {
        &self.schema.name
    }

    fn description(&self) -> &str {
        &self.schema.description
    }

    async fn execute(&self, call: &ToolCall) -> ToolResult {
        let call_id = call.call_id.clone();
        let name = self.schema.name.clone();
        let args = call.args.clone();
        let mgr = Arc::clone(&self.memory_manager);

        let result = match tokio::task::spawn_blocking(move || {
            let mut mgr = mgr.lock().unwrap();
            mgr.handle_tool_call(&name, &args)
        })
        .await
        {
            Ok(res) => res.to_string(),
            Err(e) => format!("Memory tool panicked: {e}"),
        };

        ToolResult {
            call_id,
            output: result,
            error: None,
        }
    }

    fn clone_tool(&self) -> Box<dyn Tool> {
        Box::new(Self {
            memory_manager: Arc::clone(&self.memory_manager),
            schema: self.schema.clone(),
        })
    }
}

/// Register all memory tool wrappers into the given registry.
pub fn register_memory_tools(
    registry: &mut oben_tools::ToolRegistry,
    memory_manager: Arc<std::sync::Mutex<MemoryManager>>,
) {
    let schemas = {
        let mgr = memory_manager.lock().unwrap();
        mgr.get_all_tool_schemas()
    };

    for schema in schemas {
        let memory_manager = Arc::clone(&memory_manager);
        let tool = Box::new(MemoryToolWrapper::new(schema.clone(), memory_manager));
        let def = ToolMeta {
            name: schema.name,
            description: schema.description,
            parameters: oben_models::ToolParameters::JsonSchema {
                schema: schema.parameters,
            },
        };
        registry.register_with_def(tool, def);
    }
}
