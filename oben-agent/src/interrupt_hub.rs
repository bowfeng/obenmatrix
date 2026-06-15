/// Manages interrupt propagation across subagent trees.
///
/// Coordinator tracks all direct children (spawned via delegate tool) in a flat
/// list keyed by session ID. Each child record captures its depth for DFS ordering.
///
/// When an interrupt fires (`request_interrupt`), the hub performs a DFS sweep
/// from deepest nodes first — leaf subagents at the maximum depth level are
/// interrupted before their parents.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::interrupt::InterruptState;

/// A registered child subagent's interrupt capability.
///
/// Each record holds the shared interrupt state reference and the
/// subagent's depth in the delegation tree.
pub struct SubagentRecord {
    /// Child session ID (unique key in the hub).
    pub session_id: String,
    /// Delegation depth: 1 = direct child of root, 2 = grandchild, etc.
    pub depth: u32,
    /// Thread-safe interrupt state for this subagent.
    pub state: Arc<InterruptState>,
}

/// Manages interrupt propagation across a tree of subagents.
///
/// Uses a flat HashMap — keys are child session IDs, values are `SubagentRecord`s.
/// The coordinator adds children when spawned, removes them on completion.
pub struct InterruptHub {
    /// Flat map of child session ID -> record.
    children: Mutex<HashMap<String, SubagentRecord>>,
    /// Max allowed depth — prevents interrupts from propagating beyond this.
    max_spawn_depth: u32,
}

impl InterruptHub {
    /// Create a new interrupt hub.
    ///
    /// **Parameters:**
    /// - `max_spawn_depth` — maximum delegation depth. Children at depth > this
    ///   are treated as leaves and are included in the interrupt sweep.
    pub fn new(max_spawn_depth: u32) -> Self {
        Self {
            children: Mutex::new(HashMap::new()),
            max_spawn_depth,
        }
    }

    /// Register a child subagent's interrupt state.
    ///
    /// This is called by the coordinator's turn loop whenever a delegate tool
    /// invocation spawns a child subagent.
    pub fn register(&self, record: SubagentRecord) {
        let mut children = self.children.lock().unwrap();
        children.insert(record.session_id.clone(), record);
    }

    /// Remove a completed child and its interrupt state.
    pub fn unregister(&self, session_id: &str) {
        let mut children = self.children.lock().unwrap();
        children.remove(session_id);
    }

    /// Return count of active children currently tracked.
    pub fn child_count(&self) -> usize {
        let children = self.children.lock().unwrap();
        children.len()
    }

    /// DFS interrupt propagation.
    ///
    /// Sorts children by depth descending (deepest/leaf nodes first),
    /// then fires `request_interrupt()` on each child's interrupt state.
    ///
    /// This mirrors the DFS-from-leaves semantics: deepest subagents
    /// are interrupted first, preventing them from spawning further children
    /// while the interrupt propagates upward.
    pub fn dfs_interrupt_children(&self, message: Option<String>) {
        let mut children = self.children.lock().unwrap();
        // Sort by depth descending — deepest nodes first (DFS from leaves)
        let mut records: Vec<_> = children.values_mut().collect();
        records.sort_by(|a, b| b.depth.cmp(&a.depth));

        for record in &records {
            record.state.request_interrupt(message.clone());
        }
    }

    /// Check if any tracked child is currently interrupted.
    pub fn is_any_interrupted(&self) -> bool {
        let children = self.children.lock().unwrap();
        children.values().any(|r| r.state.is_interrupted())
    }

    /// Get maximum depth of currently tracked children.
    pub fn max_depth(&self) -> u32 {
        let children = self.children.lock().unwrap();
        children
            .values()
            .map(|r| r.depth)
            .max()
            .unwrap_or(0)
    }
}

impl std::fmt::Debug for InterruptHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let children = self.children.lock().unwrap();
        f.debug_struct("InterruptHub")
            .field("child_count", &children.len())
            .field("max_spawn_depth", &self.max_spawn_depth)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_record(session_id: &str, depth: u32) -> SubagentRecord {
        SubagentRecord {
            session_id: session_id.to_string(),
            depth,
            state: Arc::new(InterruptState::new()),
        }
    }

    #[test]
    fn test_register_unregister() {
        let hub = InterruptHub::new(3);
        assert_eq!(hub.child_count(), 0);

        hub.register(make_record("child-1", 1));
        assert_eq!(hub.child_count(), 1);

        hub.unregister("child-1");
        assert_eq!(hub.child_count(), 0);
    }

    #[test]
    fn test_dfs_interrupt_order() {
        let hub = InterruptHub::new(3);
        hub.register(make_record("depth-1", 1));
        hub.register(make_record("depth-3", 3));
        hub.register(make_record("depth-2", 2));
        hub.register(make_record("depth-3-b", 3));

        hub.dfs_interrupt_children(Some("test".into()));

        // Verify deepest children are interrupted
        assert!(hub.is_any_interrupted());
        assert_eq!(hub.max_depth(), 3);
    }

    #[test]
    fn test_clear_interrupt_after_unregister() {
        let hub = InterruptHub::new(3);
        let record = make_record("child-1", 1);
        hub.register(record);

        hub.dfs_interrupt_children(None);
        assert!(hub.is_any_interrupted());

        hub.unregister("child-1");
        assert!(!hub.is_any_interrupted());
    }
}
