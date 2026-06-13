use serde::{Deserialize, Serialize};

/// A tool that the agent can call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolMeta {
    pub name: String,
    pub description: String,
    pub parameters: ToolParameters,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToolParameters {
    /// OpenAI-style JSON Schema.
    JsonSchema { schema: serde_json::Value },
    /// Simple flat schema.
    Flat(Vec<ToolParameter>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolParameter {
    pub name: String,
    pub description: String,
    pub parameter_type: String,
    pub required: bool,
}

/// An invocation of a tool by the agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub tool_name: String,
    pub arguments: serde_json::Value,
}

impl ToolCall {
    /// Create from a transport tool call (avoids cloning the arguments Value).
    pub fn from_transport(tc: &crate::TransportToolCall) -> Self {
        Self {
            id: tc.id.clone(),
            tool_name: tc.tool_name.clone(),
            arguments: tc.arguments.clone(),
        }
    }
}

/// Result of executing a tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub call_id: String,
    pub output: String,
    pub error: Option<String>,
}

impl ToolMeta {
    pub fn builder(name: impl Into<String>, description: impl Into<String>) -> ToolBuilder {
        ToolBuilder {
            name: name.into(),
            description: description.into(),
            schema: None,
            flat_params: Vec::new(),
        }
    }
}

impl ToolParameter {
    pub fn required(name: impl Into<String>, desc: impl Into<String>, ptype: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: desc.into(),
            parameter_type: ptype.into(),
            required: true,
        }
    }

    pub fn optional(name: impl Into<String>, desc: impl Into<String>, ptype: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: desc.into(),
            parameter_type: ptype.into(),
            required: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ToolBuilder {
    name: String,
    description: String,
    schema: Option<serde_json::Value>,
    flat_params: Vec<ToolParameter>,
}

impl ToolBuilder {
    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.description = desc.into();
        self
    }

    pub fn json_schema(mut self, schema: serde_json::Value) -> Self {
        self.schema = Some(schema);
        self
    }

    pub fn param(
        mut self,
        name: impl Into<String>,
        desc: impl Into<String>,
        ptype: impl Into<String>,
        required: bool,
    ) -> Self {
        self.flat_params.push(ToolParameter {
            name: name.into(),
            description: desc.into(),
            parameter_type: ptype.into(),
            required,
        });
        self
    }

    pub fn build(self) -> ToolMeta {
        let parameters = if let Some(schema) = self.schema {
            ToolParameters::JsonSchema { schema }
        } else {
            ToolParameters::Flat(self.flat_params)
        };
        ToolMeta {
            name: self.name,
            description: self.description,
            parameters,
        }
    }
}
