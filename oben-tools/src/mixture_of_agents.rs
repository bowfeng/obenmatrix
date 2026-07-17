//! Mixture of Agents (MoA) coordination tool
//!
//! Implements the MoA pattern:
//! - Input: user prompt + list of agent roles
//! - Each agent processes the prompt independently
//! - Middleware agent synthesizes responses
//! - Final agent produces the final answer
//!
//! Based on the paper: "Mixture of Agents: Enhancing Large Language Model Capabilities"
//! https://arxiv.org/abs/2309.11475

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tracing::{info, debug};

use oben_models::ToolResult;
use oben_config::MixtureOfAgentsConfig;
use crate::registry::{Tool, ToolCall, ToolRegistry};

/// MoA coordination result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoAResult {
    /// Layer-by-layer outputs
    pub layer_outputs: Vec<Vec<String>>,
    /// Final synthesized output
    pub final_output: String,
    /// Total time taken (ms)
    pub duration_ms: u64,
    /// Number of agents involved
    pub total_agents: usize,
    /// Configuration used
    pub config: MixtureOfAgentsConfig,
}

/// Mixture of Agents coordinated tool
pub struct MoATool {
    config: MixtureOfAgentsConfig,
}

impl MoATool {
    pub fn new(config: MixtureOfAgentsConfig) -> Self {
        Self { config }
    }
    
    /// Simulate running agents (without actual LLM calls for testing)
    /// In production, this would call the LLM transport directly
    pub async fn execute_moa(
        &self,
        prompt: &str,
        agents_per_layer: &[usize],
    ) -> Result<MoAResult> {
        let start_time = std::time::Instant::now();
        
        if agents_per_layer.is_empty() {
            return Err(anyhow::anyhow!("At least one layer is required"));
        }
        
        let total_layers = agents_per_layer.len();
        let mut layer_outputs: Vec<Vec<String>> = Vec::with_capacity(total_layers);
        let mut previous_layer_results: Vec<String> = Vec::new();
        
        // Process each layer
        for (layer_idx, &num_agents) in agents_per_layer.iter().enumerate() {
            debug!("Processing layer {layer_idx} with {num_agents} agents");
            
            let mut layer_results: Vec<String> = Vec::with_capacity(num_agents);
            
            // Simulate each agent's response
            for agent_idx in 0..num_agents {
                // Build the layer-specific prompt
                let layer_prompt = if layer_idx == 0 {
                    format!("Role: Layer {} Agent {}\n\nTask: {}\n\nGive your best response.",
                            layer_idx, agent_idx, prompt)
                } else {
                    let prev_str = previous_layer_results.join("\n---\n");
                    format!("Role: Layer {} Agent {}\n\nPrevious layer responses:\n{}\n\nTask: {}\n\nSynthesize and improve based on previous responses.",
                            layer_idx, agent_idx, prev_str, prompt)
                };
                
                // Simulate agent response (in production, this would call the LLM)
                let response = format!("Agent {}/{} response to: {}", layer_idx, agent_idx, layer_prompt);
                info!("Agent layer {}/{} completed", layer_idx, agent_idx);
                
                layer_results.push(response);
            }
            
            layer_outputs.push(layer_results.clone());
            previous_layer_results = layer_results;
        }
        
        // Final layer: synthesize the best response
        let final_output = if previous_layer_results.len() == 1 {
            previous_layer_results.pop().unwrap_or_default()
        } else {
            // Multiple results - use a synthesizer agent
            format!("Synthesized from {} responses:\n{}",
                    previous_layer_results.len(),
                    previous_layer_results.join("\n"))
        };
        
        let duration_ms = start_time.elapsed().as_millis() as u64;
        
        Ok(MoAResult {
            layer_outputs,
            final_output,
            duration_ms,
            total_agents: previous_layer_results.len(),
            config: self.config.clone(),
        })
    }
}

#[async_trait::async_trait]
impl Tool for MoATool {
    fn name(&self) -> &str {
        "mixture_of_agents"
    }
    
    fn description(&self) -> &str {
        "Execute Mixture of Agents coordination: spawn multiple agents across layers, have them process the prompt independently, then synthesize the final output"
    }
    
    async fn execute(&self, call: &ToolCall) -> ToolResult {
        let prompt = match call.required_str("prompt") {
            Ok(p) => p,
            Err(e) => {
                return ToolResult {
                    call_id: call.call_id.clone(),
                    output: String::new(),
                    error: Some(format!("Missing prompt: {}", e)),
                };
            }
        };
        
        let num_layers = call.required_str("num_layers")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(self.config.num_layers);
        
        let agents_per_layer = call.optional_array("agents_per_layer")
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_u64().map(|n| n as usize))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|| self.config.agents_per_layer.iter().map(|&n| n as usize).collect());
        
        // Trim or expand agents_per_layer to match num_layers
        let mut layer_config = agents_per_layer;
        if layer_config.len() > num_layers {
            layer_config.truncate(num_layers);
        } else if layer_config.len() < num_layers {
            // Pad with the last value or 1
            let last_val = *layer_config.last().unwrap_or(&1);
            layer_config.resize(num_layers, last_val);
        }
        
        match self.execute_moa(prompt, &layer_config).await {
            Ok(result) => {
                let output = format!(
                    "MoA completed in {}ms with {} agents across {} layers.\n\nFinal Output:\n{}",
                    result.duration_ms,
                    result.total_agents,
                    result.layer_outputs.len(),
                    result.final_output
                );
                ToolResult {
                    call_id: call.call_id.clone(),
                    output,
                    error: None,
                }
            }
            Err(e) => {
                ToolResult {
                    call_id: call.call_id.clone(),
                    output: String::new(),
                    error: Some(format!("MoA execution failed: {}", e)),
                }
            }
        }
    }
    
    fn clone_tool(&self) -> Box<dyn Tool> {
        Box::new(MoATool::new(self.config.clone()))
    }
}

/// Register the MoA tool
pub fn register(registry: &mut ToolRegistry) {
    let config = MixtureOfAgentsConfig::default();
    let tool = MoATool::new(config);
    registry.register(Box::new(tool));
}

/// Create a MoA tool from config
pub fn create_from_config(config: &MixtureOfAgentsConfig) -> MoATool {
    MoATool::new(config.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_moa_tool_creation() {
        let config = MixtureOfAgentsConfig {
            enabled: true,
            num_layers: 3,
            agents_per_layer: vec![2, 2, 1],
        };
        let tool = MoATool::new(config);
        
        assert_eq!(tool.name(), "mixture_of_agents");
    }

    #[tokio::test]
    async fn test_moa_coordination() -> Result<()> {
        let config = MixtureOfAgentsConfig {
            enabled: true,
            num_layers: 2,
            agents_per_layer: vec![2, 1],
        };
        let tool = MoATool::new(config);
        
        let result = tool.execute_moa("What is Rust?", &[2, 1]).await?;
        
        assert!(!result.final_output.is_empty());
        assert_eq!(result.layer_outputs.len(), 2);
        assert_eq!(result.layer_outputs[0].len(), 2);
        assert_eq!(result.layer_outputs[1].len(), 1);
        
        Ok(())
    }

    #[tokio::test]
    async fn test_moa_single_layer() -> Result<()> {
        let config = MixtureOfAgentsConfig {
            enabled: true,
            num_layers: 1,
            agents_per_layer: vec![3],
        };
        let tool = MoATool::new(config);
        
        let result = tool.execute_moa("Test prompt", &[3]).await?;
        
        assert!(!result.final_output.is_empty());
        assert_eq!(result.layer_outputs.len(), 1);
        assert_eq!(result.layer_outputs[0].len(), 3);
        
        Ok(())
    }

    #[tokio::test]
    async fn test_moa_tool_execution() {
        let mut registry = ToolRegistry::new();
        register(&mut registry);
        
        let result = registry.execute(
            "mixture_of_agents",
            &serde_json::json!({
                "prompt": "What is 2+2?",
                "num_layers": "2",
                "agents_per_layer": [2, 1]
            }),
        ).await;
        
        assert!(result.error.is_none());
        assert!(!result.output.is_empty());
        assert!(result.output.contains("MoA completed"));
    }

    #[tokio::test]
    async fn test_moa_validation() {
        let config = MixtureOfAgentsConfig {
            enabled: true,
            num_layers: 3,
            agents_per_layer: vec![2, 2, 1],
        };
        
        assert!(config.validate());
        assert_eq!(config.total_agents(), 5);
    }

    #[tokio::test]
    async fn test_moa_empty_agents() {
        let config = MixtureOfAgentsConfig {
            enabled: true,
            num_layers: 2,
            agents_per_layer: vec![],
        };
        
        assert!(!config.validate());
    }
}
