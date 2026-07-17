use serde::{Deserialize, Serialize};

/// Configuration for Mixture of Agents (MoA) coordination
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct MixtureOfAgentsConfig {
    /// Whether MoA mode is enabled
    pub enabled: bool,
    /// Number of layers in the MoA hierarchy
    pub num_layers: usize,
    /// Number of agents per layer (from input layer to output layer)
    pub agents_per_layer: Vec<usize>,
}

impl MixtureOfAgentsConfig {
    /// Validate the configuration
    pub fn validate(&self) -> bool {
        if !self.enabled {
            return true;
        }
        
        // Must have at least 2 layers (input + output)
        if self.num_layers < 2 {
            return false;
        }
        
        // agents_per_layer count must match num_layers
        if self.agents_per_layer.len() != self.num_layers {
            return false;
        }
        
        // Each layer must have at least 1 agent
        for &count in &self.agents_per_layer {
            if count == 0 {
                return false;
            }
        }
        
        true
    }
    
    /// Total number of agents across all layers
    pub fn total_agents(&self) -> usize {
        self.agents_per_layer.iter().sum()
    }
}
