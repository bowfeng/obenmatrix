/// Plan node — a single unit of work in the agent's plan.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Status of a plan node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeStatus {
    Pending,
    InProgress,
    Done,
    Failed,
}

impl Default for NodeStatus {
    fn default() -> Self {
        NodeStatus::Pending
    }
}

/// A single unit of work in the agent's plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanNode {
    /// Human-readable label for this node.
    pub title: String,
    /// Longer instruction for the LLM.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Current status of this node.
    #[serde(default)]
    pub status: NodeStatus,
    /// Sub-nodes (nested plan items).
    #[serde(default)]
    pub sub_nodes: Vec<PlanNode>,
    /// What was accomplished (set when status is Done).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_message: Option<String>,
    /// Why it failed (set when status is Failed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
    /// Files/artifacts produced by this node.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<String>,
    /// When the node was created.
    #[serde(default)]
    pub created_at: DateTime<Utc>,
    /// When the node was completed or failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
}

impl PlanNode {
    /// Create a new plan node with Pending status.
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            description: None,
            status: NodeStatus::Pending,
            sub_nodes: Vec::new(),
            completion_message: None,
            failure_reason: None,
            artifacts: Vec::new(),
            created_at: Utc::now(),
            completed_at: None,
        }
    }

    /// Set a longer description for this node.
    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Set the status directly.
    pub fn status(mut self, status: NodeStatus) -> Self {
        self.status = status;
        self
    }

    /// Add a sub-node.
    pub fn add_sub_node(mut self, child: PlanNode) -> Self {
        self.sub_nodes.push(child);
        self
    }

    /// Add a sub-node (fluent variant).
    pub fn push_sub_node(&mut self, child: PlanNode) {
        self.sub_nodes.push(child);
    }

    /// Mark this node as done with a completion message.
    pub fn done(mut self, message: impl Into<String>) -> Self {
        self.status = NodeStatus::Done;
        self.completion_message = Some(message.into());
        self.completed_at = Some(Utc::now());
        self
    }

    /// Mark this node as failed with a reason.
    pub fn fail(mut self, reason: impl Into<String>) -> Self {
        self.status = NodeStatus::Failed;
        self.failure_reason = Some(reason.into());
        self.completed_at = Some(Utc::now());
        self
    }

    /// Add an artifact (file produced by this node).
    pub fn add_artifact(mut self, artifact: impl Into<String>) -> Self {
        self.artifacts.push(artifact.into());
        self
    }

    /// Add an artifact (fluent variant).
    pub fn push_artifact(&mut self, artifact: impl Into<String>) {
        self.artifacts.push(artifact.into());
    }

    /// Mark this node as done (fluent variant).
    pub fn mark_done(&mut self, message: impl Into<String>) {
        self.status = NodeStatus::Done;
        self.completion_message = Some(message.into());
        self.completed_at = Some(Utc::now());
    }

    /// Mark this node as failed (fluent variant).
    pub fn mark_failed(&mut self, reason: impl Into<String>) {
        self.status = NodeStatus::Failed;
        self.failure_reason = Some(reason.into());
        self.completed_at = Some(Utc::now());
    }

    /// Set in-progress status.
    pub fn mark_in_progress(&mut self) {
        self.status = NodeStatus::InProgress;
    }

    /// Check if this node is a leaf (no sub-nodes).
    pub fn is_leaf(&self) -> bool {
        self.sub_nodes.is_empty()
    }

    /// Count total nodes including all descendants.
    pub fn total_count(&self) -> usize {
        1 + self
            .sub_nodes
            .iter()
            .map(|n| n.total_count())
            .sum::<usize>()
    }

    /// Count nodes at this level (not including descendants).
    pub fn child_count(&self) -> usize {
        self.sub_nodes.len()
    }

    /// Format this node and its children as a markdown checklist.
    pub fn to_markdown(&self, indent: usize) -> String {
        let prefix = "  ".repeat(indent);
        let checkbox = match self.status {
            NodeStatus::Pending => "- [ ]",
            NodeStatus::InProgress => "- [~]",
            NodeStatus::Done => "- [x]",
            NodeStatus::Failed => "- [!]",
        };
        let mut lines = vec![format!("{}{} **{}**", prefix, checkbox, self.title)];
        if let Some(ref desc) = self.description {
            lines.push(format!("{}  _{}_{}", prefix, desc, ""));
        }
        for sub in &self.sub_nodes {
            lines.push(sub.to_markdown(indent + 1));
        }
        lines.join("\n")
    }
}
