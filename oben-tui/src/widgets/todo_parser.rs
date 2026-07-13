//! TODO parsing — extracts and tracks TODO markers from assistant output.
//!
//! Parses `TODO:`, `DONE:`, and `CANCELLED:` markers from message content,
//! tracks their state (pending/in_progress/completed/cancelled), and provides
//! rendering for display in details panels.

use regex::Regex;

/// Represents a single TODO item with its state.
#[derive(Debug, Clone, PartialEq)]
pub struct TodoItem {
    /// Unique identifier for the task
    pub id: String,
    /// Task description
    pub content: String,
    /// Current status
    pub status: TodoStatus,
}

/// Status of a TODO item.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
    Cancelled,
}

impl Default for TodoItem {
    fn default() -> Self {
        Self {
            id: String::new(),
            content: String::new(),
            status: TodoStatus::Pending,
        }
    }
}

/// Parse TODO markers from assistant message output.
///
/// Supports patterns like:
/// - `TODO: [id] description` (pending)
/// - `TODO: [id] [in_progress] description` (in progress)
/// - `DONE: [id] description` (completed)
/// - `CANCELLED: [id] description` (cancelled)
pub fn parse_todos_from_text(text: &str) -> Vec<TodoItem> {
    let mut todos = Vec::new();
    
    let patterns = [
        (r"TODO:\s*\[([^\]]+)\]\s*\[in_progress\]\s*(.+)", TodoStatus::InProgress),
        (r"TODO:\s*\[([^\]]+)\]\s*(.+)", TodoStatus::Pending),
        (r"DONE:\s*\[([^\]]+)\]\s*(.+)", TodoStatus::Completed),
        (r"CANCELLED:\s*\[([^\]]+)\]\s*(.+)", TodoStatus::Cancelled),
    ];
    let patterns = [
        (r"TODO:\s*\[([^\]]+)\]\s*\[in_progress\]\s*(.+)", TodoStatus::InProgress),
        (r"TODO:\s*\[([^\]]+)\]\s*(.+)", TodoStatus::Pending),
        (r"DONE:\s*\[([^\]]+)\]\s*(.+)", TodoStatus::Completed),
        (r"CANCELLED:\s*\[([^\]]+)\]\s*(.+)", TodoStatus::Cancelled),
    ];
    
    for line in text.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("TODO:") && 
           !trimmed.starts_with("DONE:") && 
           !trimmed.starts_with("CANCELLED:") {
            continue;
        }
        
        for (pattern, status) in patterns.iter() {
            if let Some(caps) = Regex::new(pattern).ok().and_then(|re| re.captures(trimmed)) {
                let id = caps.get(1).map(|m| m.as_str().trim().to_string()).unwrap_or_default();
                let content = caps.get(2).map(|m| m.as_str().trim().to_string()).unwrap_or_default();
                
                todos.push(TodoItem {
                    id,
                    content,
                    status: *status,
                });
                break;
            }
        }
    }
    
    todos
}

/// Format a TODO item for display.
pub fn format_todo_item(todo: &TodoItem) -> String {
    let marker = match todo.status {
        TodoStatus::Pending => "[ ]",
        TodoStatus::InProgress => "[>]",
        TodoStatus::Completed => "[x]",
        TodoStatus::Cancelled => "[-]",
    };
    
    format!("{} {} - {}", marker, todo.id, todo.content)
}

/// Format a list of TODO items for display.
pub fn format_todos(todos: &[TodoItem]) -> String {
    if todos.is_empty() {
        return String::new();
    }
    
    let lines: Vec<String> = todos.iter().map(format_todo_item).collect();
    lines.join("\n")
}

/// Count todos by status.
pub fn count_by_status(todos: &[TodoItem]) -> (usize, usize, usize, usize) {
    let mut pending = 0;
    let mut in_progress = 0;
    let mut completed = 0;
    let mut cancelled = 0;
    
    for todo in todos {
        match todo.status {
            TodoStatus::Pending => pending += 1,
            TodoStatus::InProgress => in_progress += 1,
            TodoStatus::Completed => completed += 1,
            TodoStatus::Cancelled => cancelled += 1,
        }
    }
    
    (pending, in_progress, completed, cancelled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_todo() {
        let text = "TODO: [task1] Implement feature";
        let todos = parse_todos_from_text(text);
        
        assert_eq!(todos.len(), 1);
        assert_eq!(todos[0].id, "task1");
        assert_eq!(todos[0].content, "Implement feature");
        assert_eq!(todos[0].status, TodoStatus::Pending);
    }

    #[test]
    fn test_parse_todo_with_in_progress() {
        let text = "TODO: [task1] [in_progress] Working on it";
        let todos = parse_todos_from_text(text);
        
        assert_eq!(todos.len(), 1);
        assert_eq!(todos[0].status, TodoStatus::InProgress);
    }

    #[test]
    fn test_parse_done() {
        let text = "DONE: [task1] Completed task";
        let todos = parse_todos_from_text(text);
        
        assert_eq!(todos.len(), 1);
        assert_eq!(todos[0].status, TodoStatus::Completed);
    }

    #[test]
    fn test_parse_cancelled() {
        let text = "CANCELLED: [task1] Cancelled task";
        let todos = parse_todos_from_text(text);
        
        assert_eq!(todos.len(), 1);
        assert_eq!(todos[0].status, TodoStatus::Cancelled);
    }

    #[test]
    fn test_parse_mixed_todos() {
        let text = r#"TODO: [task1] First task
TODO: [task2] [in_progress] Second task
DONE: [task3] Third task"#;
        let todos = parse_todos_from_text(text);
        
        assert_eq!(todos.len(), 3);
        assert_eq!(todos[0].status, TodoStatus::Pending);
        assert_eq!(todos[1].status, TodoStatus::InProgress);
        assert_eq!(todos[2].status, TodoStatus::Completed);
    }

    #[test]
    fn test_format_todo_item() {
        let todo = TodoItem {
            id: "task1".to_string(),
            content: "Implement feature".to_string(),
            status: TodoStatus::Pending,
        };
        
        let formatted = format_todo_item(&todo);
        assert!(formatted.contains("[ ]"));
        assert!(formatted.contains("task1"));
        assert!(formatted.contains("Implement feature"));
    }

    #[test]
    fn test_count_by_status() {
        let todos = vec![
            TodoItem { id: "1".to_string(), content: "A".to_string(), status: TodoStatus::Pending },
            TodoItem { id: "2".to_string(), content: "B".to_string(), status: TodoStatus::InProgress },
            TodoItem { id: "3".to_string(), content: "C".to_string(), status: TodoStatus::Completed },
            TodoItem { id: "4".to_string(), content: "D".to_string(), status: TodoStatus::Cancelled },
        ];
        
        let (pending, in_progress, completed, cancelled) = count_by_status(&todos);
        
        assert_eq!(pending, 1);
        assert_eq!(in_progress, 1);
        assert_eq!(completed, 1);
        assert_eq!(cancelled, 1);
    }
}
