/// Todo tool — TODO list management with JSON persistence.
///
/// Supports add, complete, remove, list, and priority levels.

use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::sync::LazyLock;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use chrono::Utc;

use oben_models::{Tool, ToolParameter, ToolParameters, ToolResult};

use super::registry::{ToolHandler, SelfRegisteringTool};

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItem {
    pub id: u64,
    pub title: String,
    pub completed: bool,
    pub priority: String,  // low, medium, high, critical
    pub created_at: String,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoStore {
    pub items: Vec<TodoItem>,
    pub next_id: u64,
}

impl TodoStore {
    fn new() -> Self {
        Self { items: Vec::new(), next_id: 1 }
    }

    fn load() -> Self {
        let path = Self::get_path();
        if path.exists() {
            match fs::read_to_string(&path) {
                Ok(json) => {
                    match serde_json::from_str(&json) {
                        Ok(store) => store,
                        Err(_) => Self::new(),
                    }
                }
                Err(_) => Self::new(),
            }
        } else {
            Self::new()
        }
    }

    fn save(&self) {
        let path = Self::get_path();
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let json = serde_json::to_string_pretty(self).unwrap_or_default();
        let tmp_path = path.with_extension("tmp");
        if let Err(e) = fs::write(&tmp_path, &json) {
            tracing::warn!("Failed to save todo: {}", e);
        } else {
            let _ = fs::rename(&tmp_path, &path);
        }
    }

    fn get_path() -> std::path::PathBuf {
        if let Ok(home) = std::env::var("HOME") {
            Path::new(&home).join(".config/obenalien/todo.json")
        } else {
            Path::new(".").join("todo.json")
        }
    }

    fn add(&mut self, title: &str, priority: &str) -> &TodoItem {
        let now = Utc::now().to_rfc3339();
        let item = TodoItem {
            id: self.next_id,
            title: title.to_string(),
            completed: false,
            priority: priority.to_string(),
            created_at: now,
            completed_at: None,
        };
        self.next_id += 1;
        self.items.push(item);
        self.items.last().unwrap()
    }

    fn complete(&mut self, id: u64) -> bool {
        if let Some(item) = self.items.iter_mut().find(|i| i.id == id && !i.completed) {
            item.completed = true;
            item.completed_at = Some(Utc::now().to_rfc3339());
            self.save();
            true
        } else {
            false
        }
    }

    fn remove(&mut self, id: u64) -> bool {
        let len_before = self.items.len();
        self.items.retain(|i| i.id != id);
        let removed = self.items.len() < len_before;
        if removed {
            self.save();
        }
        removed
    }
}

// ---------------------------------------------------------------------------
// Tool definitions
// ---------------------------------------------------------------------------

fn make_todo_tool() -> Tool {
    let params = vec![
        ToolParameter {
            name: "action".into(),
            description: "Action: add, complete, remove, list.".into(),
            parameter_type: "string".into(),
            required: true,
        },
        ToolParameter {
            name: "title".into(),
            description: "Task title (required for 'add').".into(),
            parameter_type: "string".into(),
            required: false,
        },
        ToolParameter {
            name: "id".into(),
            description: "Task ID (required for 'complete' and 'remove').".into(),
            parameter_type: "string".into(),
            required: false,
        },
        ToolParameter {
            name: "priority".into(),
            description: "Priority: low, medium, high, critical (default: medium).".into(),
            parameter_type: "string".into(),
            required: false,
        },
    ];
    Tool {
        name: "todo".into(),
        description: "Manage TODO tasks. Actions: add (new task), complete (mark done), remove (delete), list (show all).".into(),
        parameters: ToolParameters::Flat(params),
    }
}

fn make_todo_handler() -> ToolHandler {
    Arc::new(|args: Value| {
        Box::pin(async move {
            let call_id = args.get("call_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let action = args.get("action")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'action' argument"))?;

            let store = {
                let lock = STORE.lock().unwrap();
                lock.clone()
            };

            let result = match action {
                "add" => {
                    let title = args.get("title")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| anyhow::anyhow!("'title' is required for 'add'"))?;
                    let priority = args.get("priority")
                        .and_then(|v| v.as_str())
                        .unwrap_or("medium");
                    if !["low", "medium", "high", "critical"].contains(&priority) {
                        return Ok(ToolResult {
                            call_id,
                            output: String::new(),
                            error: Some(format!("Invalid priority '{}'. Use: low, medium, high, critical.", priority)),
                        });
                    }
                    add_task(&store, call_id, title, priority)
                }
                "complete" => {
                    let id_str = args.get("id")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| anyhow::anyhow!("'id' is required for 'complete'"))?;
                    let id: u64 = id_str.parse().map_err(|_| anyhow::anyhow!("Invalid task ID '{}'.", id_str))?;
                    complete_task(&store, call_id, id)
                }
                "remove" => {
                    let id_str = args.get("id")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| anyhow::anyhow!("'id' is required for 'remove'"))?;
                    let id: u64 = id_str.parse().map_err(|_| anyhow::anyhow!("Invalid task ID '{}'.", id_str))?;
                    remove_task(&store, call_id, id)
                }
                "list" => list_tasks(&store, call_id),
                _ => {
                    return Ok(ToolResult {
                        call_id,
                        output: String::new(),
                        error: Some(format!("Unknown action '{}'. Use: add, complete, remove, list.", action)),
                    });
                }
            };

            Ok(result)
        })
    })
}

fn add_task(store: &TodoStore, call_id: String, title: &str, priority: &str) -> ToolResult {
    let mut store = store.clone();
    let item = store.add(title, priority);
    let item_id = item.id;
    let item_priority = item.priority.clone();
    let item_title = item.title.clone();
    store.save();
    ToolResult {
        call_id,
        output: format!("Added: #{} [{}] {}", item_id, item_priority, item_title),
        error: None,
    }
}

fn complete_task(store: &TodoStore, call_id: String, id: u64) -> ToolResult {
    let mut store = store.clone();
    if store.complete(id) {
        ToolResult {
            call_id,
            output: format!("Completed: #{}", id),
            error: None,
        }
    } else {
        ToolResult {
            call_id,
            output: String::new(),
            error: Some(format!("Task #{} not found or already completed.", id)),
        }
    }
}

fn remove_task(store: &TodoStore, call_id: String, id: u64) -> ToolResult {
    let mut store = store.clone();
    if store.remove(id) {
        ToolResult {
            call_id,
            output: format!("Removed: #{}", id),
            error: None,
        }
    } else {
        ToolResult {
            call_id,
            output: String::new(),
            error: Some(format!("Task #{} not found.", id)),
        }
    }
}

fn list_tasks(store: &TodoStore, call_id: String) -> ToolResult {
    let mut output = String::from("📋 TODO List\n");
    output.push_str("=============\n\n");

    if store.items.is_empty() {
        output.push_str("No tasks.\n");
        return ToolResult {
            call_id,
            output,
            error: None,
        };
    }

    for item in &store.items {
        let status = if item.completed { "✅" } else { "⬜" };
        let priority_marker = match item.priority.as_str() {
            "critical" => "🔴",
            "high" => "🟠",
            "medium" => "🟡",
            _ => "⚪",
        };
        output.push_str(&format!(
            "{} {} {} #{} - {}\n",
            status, priority_marker, item.priority, item.id, item.title
        ));
    }

    let pending = store.items.iter().filter(|i| !i.completed).count();
    let completed = store.items.iter().filter(|i| i.completed).count();
    output.push_str(&format!("\n{} pending, {} completed", pending, completed));

    ToolResult {
        call_id,
        output,
        error: None,
    }
}

// ---------------------------------------------------------------------------
// Self-registration
// ---------------------------------------------------------------------------

static STORE: LazyLock<Mutex<TodoStore>> = LazyLock::new(|| Mutex::new(TodoStore::new()));

pub struct TodoTool;

impl SelfRegisteringTool for TodoTool {
    fn tool() -> Tool {
        make_todo_tool()
    }

    fn handler() -> ToolHandler {
        make_todo_handler()
    }
}

/// Register this module into the given registry.
pub fn register(registry: &mut super::registry::ToolRegistry) {
    TodoTool::register_self(registry);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::env;

    fn make_registry() -> super::super::registry::ToolRegistry {
        let mut registry = super::super::registry::ToolRegistry::new();
        TodoTool::register_self(&mut registry);
        registry
    }

    fn reset_store() {
        let path = TodoStore::get_path();
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("tmp"));
        let mut store = STORE.lock().unwrap();
        *store = TodoStore::new();
        drop(store);
    }

    #[tokio::test]
    async fn rejects_invalid_action() {
        let registry = make_registry();
        let result = registry.execute("todo", &json!({
            "action": "invalid",
            "call_id": "test-5",
        })).await;

        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("Unknown action"));
    }

    #[tokio::test]
    async fn rejects_missing_action() {
        let registry = make_registry();
        let result = registry.execute("todo", &json!({
            "title": "test",
            "call_id": "test-6",
        })).await;

        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("Missing 'action'"));
    }

    #[tokio::test]
    async fn rejects_invalid_priority() {
        let registry = make_registry();
        let result = registry.execute("todo", &json!({
            "action": "add",
            "title": "test",
            "priority": "ultra",
            "call_id": "test-7",
        })).await;

        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("Invalid priority"));
    }

    #[tokio::test]
    async fn empty_list() {
        reset_store();
        let registry = make_registry();
        let result = registry.execute("todo", &json!({
            "action": "list",
            "call_id": "test-8",
        })).await;

        assert!(result.error.is_none());
        assert!(result.output.contains("No tasks"));
    }
}
