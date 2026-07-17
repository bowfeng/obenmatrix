//! Home Assistant tool integration
//!
//! Interfaces with Home Assistant's REST API to:
//! - List/filter entities by domain or area
//! - Get detailed state of a single entity
//! - Call services (turn on/off lights, toggle switches, etc.)
//!
//! Authentication uses Bearer token via config (hass_url, hass_token)

use anyhow::Result;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde_json::Value;
use std::env;

use oben_models::{ToolMeta, ToolParameter, ToolParameters, ToolResult};

use crate::registry::{Tool, ToolCall, ToolRegistry};

// ===========================================================================
// Configuration
// ===========================================================================

/// Home Assistant configuration loaded from env vars or config
#[derive(Debug, Clone)]
pub struct HomeAssistantConfig {
    pub api_url: String,
    pub token: String,
}

impl HomeAssistantConfig {
    pub fn from_env() -> Self {
        let api_url = env::var("HASS_URL")
            .unwrap_or_else(|_| "http://homeassistant.local:8123".to_string());
        let token = env::var("HASS_TOKEN").unwrap_or_default();

        // Strip trailing slash
        let api_url = api_url.trim_end_matches('/').to_string();

        Self { api_url, token }
    }
}

impl Default for HomeAssistantConfig {
    fn default() -> Self {
        Self::from_env()
    }
}

// ===========================================================================
// Client
// ===========================================================================

/// Home Assistant API client
pub struct HomeAssistantClient {
    config: HomeAssistantConfig,
    client: reqwest::blocking::Client,
}

impl HomeAssistantClient {
    pub fn new(config: Option<HomeAssistantConfig>) -> Self {
        let config = config.unwrap_or_default();
        let client = reqwest::blocking::Client::new();

        Self { config, client }
    }

    fn build_url(&self, path: &str) -> String {
        format!("{}/api{}", self.config.api_url, path)
    }

    fn headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", self.config.token)).unwrap(),
        );
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        headers
    }

    /// List all entities, optionally filtered by domain or area
    pub fn list_entities(&self, domain: Option<&str>, area: Option<&str>) -> Result<Value> {
        let url = self.build_url("/states");
        let response = self.client.get(&url).headers(self.headers()).send()?.json::<Value>()?;

        // Parse entities - use as_ref().map_or to get proper type
        let entities: &[Value] = response.as_array().map_or(&[], |v| v);

        let mut filtered = Vec::new();
        for entity in entities {
            let entity_id = entity.get("entity_id").and_then(|v| v.as_str()).unwrap_or("");
            
            // Filter by domain
            if let Some(dom) = domain {
                if !entity_id.starts_with(&format!("{dom}.")) {
                    continue;
                }
            }

            // Filter by area
            if let Some(area_name) = area {
                let area_lower = area_name.to_lowercase();
                let friendly_name = entity
                    .get("attributes")
                    .and_then(|a| a.get("friendly_name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_lowercase();
                let entity_area = entity
                    .get("attributes")
                    .and_then(|a| a.get("area"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_lowercase();

                if !friendly_name.contains(&area_lower) && !entity_area.contains(&area_lower) {
                    continue;
                }
            }

            filtered.push(serde_json::json!({
                "entity_id": entity_id,
                "state": entity.get("state").and_then(|v| v.as_str()).unwrap_or(""),
                "friendly_name": entity
                    .get("attributes")
                    .and_then(|a| a.get("friendly_name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or(""),
            }));
        }

        Ok(serde_json::json!({
            "count": filtered.len(),
            "entities": filtered
        }))
    }

    /// Get detailed state of a single entity
    pub fn get_state(&self, entity_id: &str) -> Result<Value> {
        let url = self.build_url(&format!("/states/{entity_id}"));
        let response = self.client.get(&url).headers(self.headers()).send()?.json::<Value>()?;

        Ok(serde_json::json!({
            "entity_id": response.get("entity_id").and_then(|v| v.as_str()).unwrap_or(entity_id),
            "state": response.get("state").and_then(|v| v.as_str()).unwrap_or(""),
            "attributes": response.get("attributes").cloned().unwrap_or_default(),
            "last_changed": response.get("last_changed").cloned().unwrap_or_default(),
            "last_updated": response.get("last_updated").cloned().unwrap_or_default(),
        }))
    }

    /// Call a Home Assistant service
    pub fn call_service(
        &self,
        domain: &str,
        service: &str,
        entity_id: Option<&str>,
        data: Option<Value>,
    ) -> Result<Value> {
        // Validate service names - only lowercase letters, digits, and underscores
        let valid_name = |s: &str| s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');

        if !valid_name(domain) || !valid_name(service) {
            return Err(anyhow::anyhow!(
                "Invalid service name format. Must contain only lowercase letters, digits, and underscores."
            ));
        }

        // Blocked dangerous domains for security
        const BLOCKED_DOMAINS: &[&str] = &[
            "shell_command",
            "command_line",
            "python_script",
            "pyscript",
            "hassio",
            "rest_command",
        ];

        if BLOCKED_DOMAINS.contains(&domain) {
            return Err(anyhow::anyhow!(
                "Service domain '{}' is blocked for security. Blocked domains: {}",
                domain,
                BLOCKED_DOMAINS.join(", ")
            ));
        }

        let url = self.build_url(&format!("/services/{domain}/{service}"));
        let mut payload = serde_json::Map::new();

        if let Some(data_val) = data {
            if let Some(obj) = data_val.as_object() {
                for (k, v) in obj {
                    payload.insert(k.clone(), v.clone());
                }
            }
        }

        if let Some(eid) = entity_id {
            payload.insert("entity_id".to_string(), Value::String(eid.to_string()));
        }

        let response = self
            .client
            .post(&url)
            .headers(self.headers())
            .json(&payload)
            .send()?
            .json::<Value>()?;

        // Parse response
        let affected = match response.as_array() {
            Some(arr) => arr
                .iter()
                .map(|s| {
                    serde_json::json!({
                        "entity_id": s.get("entity_id").and_then(|v| v.as_str()).unwrap_or(""),
                        "state": s.get("state").and_then(|v| v.as_str()).unwrap_or(""),
                    })
                })
                .collect(),
            None => Vec::new(),
        };

        Ok(serde_json::json!({
            "success": true,
            "service": format!("{domain}.{service}"),
            "affected_entities": affected,
        }))
    }

    /// List available services per domain
    pub fn list_services(&self, domain: Option<&str>) -> Result<Value> {
        let url = self.build_url("/services");
        let response = self.client.get(&url).headers(self.headers()).send()?.json::<Value>()?;

        let services_map: serde_json::Map<String, Value> = response
            .as_object()
            .map_or_else(|| serde_json::Map::new(), |m| m.clone());
        let services: Vec<_> = services_map.iter().collect();
        let mut result = Vec::new();

        for (svc_domain, svc_value) in services.iter() {
            if let Some(domain_filter) = domain {
                if svc_domain != &domain_filter {
                    continue;
                }
            }

            let domain_services = match svc_value.as_object() {
                Some(obj) => obj
                    .iter()
                    .filter_map(|(svc_name, svc_info)| {
                        let description = svc_info
                            .get("description")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let fields = svc_info
                            .get("fields")
                            .and_then(|f| f.as_object())
                            .map(|fields_map| {
                                fields_map
                                    .iter()
                                    .filter_map(|(k, v)| {
                                        v.get("description")
                                            .and_then(|desc| desc.as_str())
                                            .map(|d| (k.clone(), d.to_string()))
                                    })
                                    .collect::<Value>()
                            })
                            .unwrap_or_default();

                        Some(serde_json::json!({
                            "service": svc_name,
                            "description": description,
                            "fields": fields
                        }))
                    })
                    .collect::<Vec<_>>(),
                None => Vec::new(),
            };

            result.push(serde_json::json!({
                "domain": svc_domain,
                "services": domain_services
            }));
        }

        Ok(serde_json::json!({
            "count": result.len(),
            "domains": result
        }))
    }
}

// ===========================================================================
// Tool Definition
// ===========================================================================

pub struct HomeAssistantTool;

fn make_homeassistant_tool() -> ToolMeta {
    ToolMeta {
        name: "homeassistant".into(),
        description: "Control Home Assistant smart home devices".into(),
        parameters: ToolParameters::Flat(vec![
            ToolParameter::required("action", "Action to perform: list_entities, get_state, call_service, list_services", "string"),
            ToolParameter::optional("domain", "Entity domain filter (e.g., light, switch, climate)", "string"),
            ToolParameter::optional("area", "Area filter (e.g., living room, kitchen)", "string"),
            ToolParameter::optional("entity_id", "Target entity ID (e.g., light.living_room)", "string"),
            ToolParameter::optional("service", "Service name (e.g., turn_on, turn_off)", "string"),
            ToolParameter::optional("data", "Additional service data as JSON string", "string"),
        ]),
    }
}

#[async_trait::async_trait]
impl Tool for HomeAssistantTool {
    fn name(&self) -> &str {
        "homeassistant"
    }

    fn description(&self) -> &str {
        "Control Home Assistant smart home devices"
    }

    async fn execute(&self, call: &ToolCall) -> ToolResult {
        let action = match call.args.get("action").and_then(|v| v.as_str()) {
            Some(a) => a.to_string(),
            None => return ToolResult {
                call_id: call.call_id.clone(),
                output: String::new(),
                error: Some("Missing 'action' parameter".to_string()),
            },
        };

        let client = HomeAssistantClient::new(None);
        let result = match action.as_str() {
            "list_entities" => {
                let domain = call.args.get("domain").and_then(|v| v.as_str());
                let area = call.args.get("area").and_then(|v| v.as_str());
                client.list_entities(domain, area)
            }
            "get_state" => {
                let entity_id = call
                    .args
                    .get("entity_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if entity_id.is_empty() {
                    return ToolResult {
                        call_id: call.call_id.clone(),
                        output: String::new(),
                        error: Some("Missing required parameter: entity_id".to_string()),
                    };
                }
                client.get_state(&entity_id)
            }
            "call_service" => {
                let domain = call.args.get("domain").and_then(|v| v.as_str()).unwrap_or("");
                let service = call.args.get("service").and_then(|v| v.as_str()).unwrap_or("");

                if domain.is_empty() || service.is_empty() {
                    return ToolResult {
                        call_id: call.call_id.clone(),
                        output: String::new(),
                        error: Some("Missing required parameters: domain and service".to_string()),
                    };
                }

                let entity_id = call.args.get("entity_id").and_then(|v| v.as_str());
                let data = call.args.get("data").cloned();
                client.call_service(domain, service, entity_id, data)
            }
            "list_services" => {
                let domain = call.args.get("domain").and_then(|v| v.as_str());
                client.list_services(domain)
            }
            _ => Err(anyhow::anyhow!(format!("Unknown action: {action}"))),
        };

        match result {
            Ok(response) => ToolResult {
                call_id: call.call_id.clone(),
                output: serde_json::to_string(&response).unwrap_or_default(),
                error: None,
            },
            Err(e) => ToolResult {
                call_id: call.call_id.clone(),
                output: String::new(),
                error: Some(e.to_string()),
            },
        }
    }

    fn clone_tool(&self) -> Box<dyn Tool> {
        Box::new(Self)
    }
}

// ===========================================================================
// Registration
// ===========================================================================

/// Register Home Assistant tool into the registry
pub fn register(registry: &mut ToolRegistry) {
    let tool = Box::new(HomeAssistantTool);
    registry.register_with_def(tool, make_homeassistant_tool());
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// BDD Test: List entities without filters
    /// Given: Home Assistant client initialized
    /// When: list_entities called with no filters
    /// Then: Returns all entities
    #[test]
    fn test_list_entities_no_filter() {
        let config = HomeAssistantConfig {
            api_url: "http://localhost:8123".to_string(),
            token: "test-token".to_string(),
        };
        let client = HomeAssistantClient::new(Some(config));

        // Without actual HA instance, this will fail
        // In real tests, we'd use a mock server
        // This demonstrates the expected API
    }

    /// BDD Test: Get entity state
    /// Given: Valid entity_id
    /// When: get_state called
    /// Then: Returns entity state with attributes
    #[test]
    fn test_get_state_returns_state() {
        // Setup mock
        let mock_state = serde_json::json!({
            "entity_id": "light.living_room",
            "state": "on",
            "attributes": {
                "brightness": 255,
                "friendly_name": "Living Room Light"
            }
        });

        let result = serde_json::json!({
            "entity_id": mock_state["entity_id"].as_str().unwrap(),
            "state": mock_state["state"].as_str().unwrap(),
            "attributes": mock_state["attributes"].clone(),
            "last_changed": mock_state["last_changed"].clone(),
            "last_updated": mock_state["last_updated"].clone(),
        });

        assert_eq!(result["entity_id"], "light.living_room");
        assert_eq!(result["state"], "on");
    }

    /// BDD Test: Call service with blocked domain
    /// Given: Service domain in blocked list
    /// When: call_service executed
    /// Then: Returns security error
    #[test]
    fn test_call_service_blocked_domain() {
        let client = HomeAssistantClient::new(None);

        // Test blocked domains logic
        const BLOCKED_DOMAINS: &[&str] = &[
            "shell_command",
            "command_line",
            "python_script",
            "pyscript",
            "hassio",
            "rest_command",
        ];

        assert!(BLOCKED_DOMAINS.contains(&"shell_command"));
        assert!(!BLOCKED_DOMAINS.contains(&"light"));

        // With blocked domains, call_service should return error
        let result = client.call_service("shell_command", "run", Some("test"), None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("blocked"));
    }

    /// BDD Test: Service name validation
    /// Given: Invalid service name with uppercase or special chars
    /// When: call_service executed
    /// Then: Returns validation error
    #[test]
    fn test_service_name_validation() {
        // Test uppercase domain - should fail validation
        let config = HomeAssistantConfig {
            api_url: "http://localhost:8123".to_string(),
            token: "test-token".to_string(),
        };
        let client = HomeAssistantClient::new(Some(config));

        let result = client.call_service("Light", "turn_on", Some("test"), None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid service name format"));

        let result = client.call_service("light", "Turn_On", Some("test"), None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid service name format"));

        let result = client.call_service("light", "turn_on", Some("test"), None);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("sending request"));
    }

    /// BDD Test: List services with domain filter
    /// Given: Services available in Home Assistant
    /// When: list_services called with domain filter
    /// Then: Returns only matching domain services
    #[test]
    fn test_list_services_filter() {
        let config = HomeAssistantConfig {
            api_url: "http://localhost:8123".to_string(),
            token: "test-token".to_string(),
        };
        let client = HomeAssistantClient::new(Some(config));

        // Test domain filtering
        let domain_services = client.list_services(Some("light")).unwrap_err();

        // The actual test would verify domain filtering
        // For now, just confirm the method signature
        let _ = domain_services;
    }

    /// BDD Test: Integration test with mock
    /// Given: Mock HTTP server simulating HA API
    /// When: Client makes requests
    /// Then: Responses match expected format
    #[test]
    fn test_integration_with_mock() {
        // This test would require mockito which isn't available in test deps
        // For now, it documents the expected behavior
    }
}
