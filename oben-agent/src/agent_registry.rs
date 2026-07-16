use dashmap::DashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::agent_builder::AgentBuilder;
use crate::agent::Agent;
use oben_config::AppConfig;
use oben_tools::ToolRegistry;

pub struct AgentRegistry {
    agents: DashMap<String, Arc<Mutex<Agent>>>,
}

impl AgentRegistry {
    pub fn new() -> Self {
        AgentRegistry {
            agents: DashMap::new(),
        }
    }

    pub fn insert(&self, name: String, agent: Arc<Mutex<Agent>>) {
        self.agents.insert(name, agent);
    }

    pub fn get(&self, name: &str) -> Option<Arc<Mutex<Agent>>> {
        self.agents.get(name).map(|entry| Arc::clone(entry.value()))
    }

    pub fn lookup_by_role(&self, _role: &str) -> Vec<Arc<Mutex<Agent>>> {
        self.agents
            .iter()
            .map(|entry| Arc::clone(entry.value()))
            .collect()
    }

    pub async fn from_config(&self, config: &AppConfig) -> anyhow::Result<()> {
        for named_config in &config.agents {
            let agent = AgentBuilder::new()
                .with_config(config.clone())
                .with_system_prompt(named_config.role.clone())
                .with_tools(Arc::new(ToolRegistry::new()))
                .with_agent_name(Some(named_config.name.clone()))
                .build()
                .await?;

            let agent_arc = Arc::new(Mutex::new(agent));
            self.insert(named_config.name.clone(), agent_arc);
        }

        if config.agent.identity.is_some() || config.agent.execution_discipline.is_some() {
            let agent = AgentBuilder::new()
                .with_config(config.clone())
                .with_system_prompt(config.agent.identity.clone().unwrap_or_default())
                .with_tools(Arc::new(ToolRegistry::new()))
                .with_agent_name(Some("default".to_string()))
                .build()
                .await?;

            let agent_arc = Arc::new(Mutex::new(agent));
            self.insert("default".to_string(), agent_arc);
        }

        Ok(())
    }

    pub fn len(&self) -> usize {
        self.agents.len()
    }

    pub fn is_empty(&self) -> bool {
        self.agents.is_empty()
    }

    pub fn remove(&self, name: &str) -> Option<Arc<Mutex<Agent>>> {
        self.agents.remove(name).map(|(_, agent)| agent)
    }

    pub fn list_agents(&self) -> Vec<String> {
        self.agents.iter().map(|entry| entry.key().clone()).collect()
    }
}

impl Default for AgentRegistry {
    fn default() -> Self {
        Self::new()
    }
}
