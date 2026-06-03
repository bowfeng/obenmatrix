/// Plan state — the collection of top-level plan nodes.
use crate::plan::{NodeStatus, PlanNode};
use std::path::Path;

/// The full plan state — goal description and list of nodes.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PlanState {
    /// The user's goal statement.
    pub goal: String,
    /// Top-level plan nodes.
    #[serde(default)]
    pub nodes: Vec<PlanNode>,
    /// When the plan was created.
    #[serde(default)]
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl PlanState {
    /// Create a new plan state with no nodes.
    pub fn new(goal: impl Into<String>) -> Self {
        Self {
            goal: goal.into(),
            nodes: Vec::new(),
            created_at: chrono::Utc::now(),
        }
    }

    /// Add a node to the plan.
    pub fn add_node(&mut self, node: PlanNode) {
        self.nodes.push(node);
    }

    /// Find a node by title (first match, shallow search).
    pub fn find_node(&self, title: &str) -> Option<&PlanNode> {
        self.nodes.iter().find(|n| n.title == title)
    }

    /// Find a mutable reference to a node by title.
    pub fn find_node_mut(&mut self, title: &str) -> Option<&mut PlanNode> {
        self.nodes.iter_mut().find(|n| n.title == title)
    }

    /// Get the next pending node to work on.
    pub fn next_pending_node(&self) -> Option<&PlanNode> {
        self.nodes.iter().find(|n| n.status == NodeStatus::Pending)
    }

    /// Count pending nodes.
    pub fn pending_count(&self) -> usize {
        self.nodes
            .iter()
            .filter(|n| n.status == NodeStatus::Pending)
            .count()
    }

    /// Count done nodes.
    pub fn done_count(&self) -> usize {
        self.nodes
            .iter()
            .filter(|n| n.status == NodeStatus::Done)
            .count()
    }

    /// Count total nodes.
    pub fn total_count(&self) -> usize {
        self.nodes.iter().map(|n| n.total_count()).sum()
    }

    /// Mark a node as in-progress.
    pub fn mark_in_progress(&mut self, title: &str) {
        if let Some(node) = self.find_node_mut(title) {
            node.mark_in_progress();
        }
    }

    /// Mark a node as done with a completion message.
    pub fn mark_done(&mut self, title: &str, message: impl Into<String>) {
        if let Some(node) = self.find_node_mut(title) {
            node.mark_done(message);
        }
    }

    /// Mark a node as failed with a reason.
    pub fn mark_failed(&mut self, title: &str, reason: impl Into<String>) {
        if let Some(node) = self.find_node_mut(title) {
            node.mark_failed(reason);
        }
    }

    /// Format the entire plan as a markdown checklist.
    pub fn to_markdown(&self) -> String {
        let mut lines = vec![format!("## Plan: **{}**", self.goal)];
        for node in &self.nodes {
            lines.push(node.to_markdown(0));
            lines.push(String::new());
        }
        lines.join("\n")
    }

    /// Save the plan state to a JSON file.
    pub fn save_to_file(&self, path: impl AsRef<Path>) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)
    }

    /// Load the plan state from a JSON file.
    pub fn load_from_file(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let json = std::fs::read_to_string(path)?;
        let state: Self = serde_json::from_str(&json)?;
        Ok(state)
    }
}
