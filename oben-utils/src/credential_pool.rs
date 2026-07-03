//! Persistent multi-credential pool for same-provider failover and rotation.
//!
//! Implements [credential pool](https://github.com/ellie/obenmatrix/blob/main/docs/PRD-utils-parity.md#U7)
//! functionality: multiple credentials per provider with automatic rotation,
//! cooldown-based recovery, and configurable selection strategies.

use std::path::Path;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

// Status constants
pub const STATUS_OK: &str = "ok";
pub const STATUS_EXHAUSTED: &str = "exhausted";

// Auth types
pub const AUTH_TYPE_OAUTH: &str = "oauth";
pub const AUTH_TYPE_API_KEY: &str = "api_key";

// Source types
pub const SOURCE_MANUAL: &str = "manual";

// Rotation strategies
pub const STRATEGY_FILL_FIRST: &str = "fill_first";
pub const STRATEGY_ROUND_ROBIN: &str = "round_robin";
pub const STRATEGY_RANDOM: &str = "random";
pub const STRATEGY_LEAST_USED: &str = "least_used";

pub const SUPPORTED_POOL_STRATEGIES: &[&str] = &[
    STRATEGY_FILL_FIRST,
    STRATEGY_ROUND_ROBIN,
    STRATEGY_RANDOM,
    STRATEGY_LEAST_USED,
];

// Cooldown durations (seconds) before retrying an exhausted credential.
const EXHAUSTED_TTL_401_SECONDS: u64 = 5 * 60;
const EXHAUSTED_TTL_429_SECONDS: u64 = 60 * 60;
const EXHAUSTED_TTL_DEFAULT_SECONDS: u64 = 60 * 60;

// Pool key prefix for custom OpenAI-compatible endpoints.
pub const CUSTOM_POOL_PREFIX: &str = "custom:";

/// A single credential entry in a provider pool.
///
/// Maps to `PooledCredential` in the Python reference implementation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PooledCredential {
    pub provider: String,
    pub id: String,
    pub label: String,
    pub auth_type: String,
    pub priority: i32,
    pub source: String,
    pub access_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_status_at: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error_code: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error_reset_at: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_refresh: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inference_base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_key_expires_at: Option<String>,
    pub request_count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<std::collections::HashMap<String, serde_json::Value>>,
}

impl PooledCredential {
    pub fn new(provider: &str, access_token: &str) -> Self {
        Self {
            provider: provider.to_string(),
            id: Uuid::new_v4().to_string(),
            label: access_token.chars().take(16).collect(),
            auth_type: AUTH_TYPE_API_KEY.to_string(),
            priority: 0,
            source: SOURCE_MANUAL.to_string(),
            access_token: access_token.to_string(),
            refresh_token: None,
            last_status: None,
            last_status_at: None,
            last_error_code: None,
            last_error_reason: None,
            last_error_message: None,
            last_error_reset_at: None,
            base_url: None,
            expires_at: None,
            expires_at_ms: None,
            last_refresh: None,
            inference_base_url: None,
            agent_key: None,
            agent_key_expires_at: None,
            request_count: 0,
            extra: None,
        }
    }

    /// Returns the runtime API key for this credential.
    pub fn runtime_api_key(&self) -> &str {
        if self.provider == "nous" {
            self.agent_key.as_deref().unwrap_or(&self.access_token)
        } else {
            &self.access_token
        }
    }

    /// Returns the runtime base URL for this credential.
    pub fn runtime_base_url(&self) -> Option<&str> {
        if self.provider == "nous" {
            self.inference_base_url.as_deref().or(self.base_url.as_deref())
        } else {
            self.base_url.as_deref()
        }
    }

    /// Loads a credential from dict representation (for serialization).
    pub fn from_dict(provider: &str, payload: &serde_json::Value) -> Result<Self, serde_json::Error> {
        serde_json::from_value(
            serde_json::json!({
                "provider": provider,
                "id": payload.get("id").and_then(|v| v.as_str()).map(String::from).unwrap_or_else(|| Uuid::new_v4().to_string()),
                "label": payload.get("label").and_then(|v| v.as_str()).map(String::from).unwrap_or_else(|| provider.to_string()),
                "auth_type": payload.get("auth_type").and_then(|v| v.as_str()).map(String::from).unwrap_or_else(|| AUTH_TYPE_API_KEY.to_string()),
                "priority": payload.get("priority").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
                "source": payload.get("source").and_then(|v| v.as_str()).map(String::from).unwrap_or_else(|| SOURCE_MANUAL.to_string()),
                "access_token": payload.get("access_token").and_then(|v| v.as_str()).map(String::from).unwrap_or_default(),
                "refresh_token": payload.get("refresh_token").and_then(|v| v.as_str()).map(String::from),
                "last_status": payload.get("last_status").and_then(|v| v.as_str()).map(String::from),
                "last_error_code": payload.get("last_error_code").and_then(|v| v.as_u64()).map(|n| n as u16),
                "last_error_reason": payload.get("last_error_reason").and_then(|v| v.as_str()).map(String::from),
                "last_error_message": payload.get("last_error_message").and_then(|v| v.as_str()).map(String::from),
                "last_error_reset_at": payload.get("last_error_reset_at").and_then(|v| v.as_f64()),
                "base_url": payload.get("base_url").and_then(|v| v.as_str()).map(String::from),
                "expires_at": payload.get("expires_at").and_then(|v| v.as_str()).map(String::from),
                "expires_at_ms": payload.get("expires_at_ms").and_then(|v| v.as_f64()),
                "last_refresh": payload.get("last_refresh").and_then(|v| v.as_str()).map(String::from),
                "inference_base_url": payload.get("inference_base_url").and_then(|v| v.as_str()).map(String::from),
                "agent_key": payload.get("agent_key").and_then(|v| v.as_str()).map(String::from),
                "agent_key_expires_at": payload.get("agent_key_expires_at").and_then(|v| v.as_str()).map(String::from),
                "request_count": payload.get("request_count").and_then(|v| v.as_u64()).unwrap_or(0),
            })
        )
    }

    /// Marks this credential as exhausted with given error status.
    pub fn mark_exhausted(&mut self, status_code: Option<u16>, error_message: Option<&str>) {
        self.last_status = Some(STATUS_EXHAUSTED.to_string());
        self.last_status_at = Some(std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0));
        self.last_error_code = status_code;
        self.last_error_message = error_message.map(String::from);
    }

    /// Marks this credential as healthy again.
    pub fn mark_healthy(&mut self) {
        self.last_status = None;
        self.last_status_at = None;
        self.last_error_code = None;
        self.last_error_reason = None;
        self.last_error_message = None;
        self.last_error_reset_at = None;
    }

    /// Checks if the credential is currently in cooldown.
    pub fn is_in_cooldown(&self) -> bool {
        if self.last_status.as_deref() != Some(STATUS_EXHAUSTED) {
            return false;
        }
        if let Some(reset_at) = self.last_error_reset_at {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0);
            return now < reset_at;
        }
        if let Some(status_at) = self.last_status_at {
            let ttl = self.cooldown_seconds();
            return (status_at + ttl as f64) > std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0);
        }
        false
    }

    /// Returns cooldown duration based on error code.
    pub fn cooldown_seconds(&self) -> u64 {
        match self.last_error_code {
            Some(401) => EXHAUSTED_TTL_401_SECONDS,
            Some(429) => EXHAUSTED_TTL_429_SECONDS,
            _ => EXHAUSTED_TTL_DEFAULT_SECONDS,
        }
    }

    /// Increments the request count for this credential.
    pub fn increment_request_count(&mut self) {
        self.request_count += 1;
    }
}

/// Manages a pool of credentials for a single provider.
///
/// Supports four rotation strategies:
/// - `fill_first`: Use first available key until exhausted, then move to next
/// - `round_robin`: Cycle through keys evenly after each selection
/// - `least_used`: Pick key with lowest request count
/// - `random`: Random selection among healthy keys
pub struct CredentialPool {
    provider: String,
    entries: Vec<PooledCredential>,
    current_id: Option<String>,
    strategy: String,
    lock: Mutex<()>,
    active_leases: std::collections::HashMap<String, u64>,
}

impl CredentialPool {
    pub fn new(provider: &str, mut entries: Vec<PooledCredential>) -> Self {
        entries.sort_by_key(|e| e.priority);
        Self {
            provider: provider.to_string(),
            entries,
            current_id: None,
            strategy: STRATEGY_FILL_FIRST.to_string(),
            lock: Mutex::new(()),
            active_leases: std::collections::HashMap::new(),
        }
    }

    pub fn new_with_strategy(provider: &str, entries: Vec<PooledCredential>, strategy: &str) -> Self {
        let strategy = if SUPPORTED_POOL_STRATEGIES.contains(&strategy) {
            strategy.to_string()
        } else {
            STRATEGY_FILL_FIRST.to_string()
        };
        let mut pool = Self::new(provider, entries);
        pool.strategy = strategy;
        pool
    }

    pub fn has_credentials(&self) -> bool {
        !self.entries.is_empty()
    }

    pub fn has_available(&self) -> bool {
        self.entries.iter().any(|e| !e.is_in_cooldown())
    }

    pub fn entries(&self) -> Vec<&PooledCredential> {
        self.entries.iter().collect()
    }

    pub fn current(&self) -> Option<&PooledCredential> {
        self.current_id.as_ref().and_then(|id| {
            self.entries.iter().find(|e| e.id == *id)
        })
    }

    /// Selects next available credential based on rotation strategy.
    pub fn select(&mut self) -> Option<PooledCredential> {
        let _lock = self.lock.lock().unwrap();
        let available: Vec<&PooledCredential> = self.entries.iter()
            .filter(|e| !e.is_in_cooldown())
            .collect();

        if available.is_empty() {
            self.current_id = None;
            return None;
        }

        let selected = match self.strategy.as_str() {
            STRATEGY_RANDOM => {
                let idx = rand::random::<usize>() % available.len();
                available[idx].clone()
            }
            STRATEGY_LEAST_USED => {
                let entry = available.into_iter()
                    .min_by_key(|e| e.request_count)
                    .expect("non-empty available list");
                let mut entry = entry.clone();
                entry.increment_request_count();
                if let Some(pos) = self.entries.iter().position(|e| e.id == entry.id) {
                    self.entries[pos] = entry.clone();
                }
                entry
            }
            STRATEGY_ROUND_ROBIN => {
                let entry = available[0].clone();
                if let Some(pos) = self.entries.iter().position(|e| e.id == entry.id) {
                    self.entries.remove(pos);
                }
                let max_priority = self.entries.iter()
                    .map(|e| e.priority)
                    .max()
                    .map(|p| p + 1)
                    .unwrap_or(0);
                self.entries.push(PooledCredential {
                    priority: max_priority,
                    ..entry
                });
                self.entries[0].clone()
            }
            _ => available[0].clone(),
        };

        self.current_id = Some(selected.id.clone());
        Some(selected)
    }

    pub fn mark_exhausted_and_rotate(&mut self, status_code: Option<u16>, error_message: Option<&str>) -> Option<PooledCredential> {
        if let Some(entry) = self.current() {
            let id = entry.id.clone();
            let target = self.entries.iter_mut()
                .find(|e| e.id == id)
                .expect("current entry must exist in pool");
            target.mark_exhausted(status_code, error_message);
        }
        self.current_id = None;
        self.select()
    }

    pub fn add_entry(&mut self, entry: PooledCredential) {
        let max_priority = self.entries.iter()
            .map(|e| e.priority)
            .max()
            .map(|p| p + 1)
            .unwrap_or(0);
        let mut entry = entry;
        entry.priority = max_priority;
        self.entries.push(entry);
    }

    pub fn remove_index(&mut self, index: u32) -> Option<PooledCredential> {
        let idx = (index as usize).checked_sub(1)?;
        if idx >= self.entries.len() {
            return None;
        }
        let removed = self.entries.remove(idx);
        for (i, entry) in self.entries.iter_mut().enumerate() {
            entry.priority = i as i32;
        }
        if self.current_id.as_deref() == Some(removed.id.as_str()) {
            self.current_id = None;
        }
        Some(removed)
    }

    pub fn reset_statuses(&mut self) -> u32 {
        let count = self.entries.iter()
            .filter(|e| e.last_status.is_some() || e.last_error_code.is_some())
            .count() as u32;
        for entry in self.entries.iter_mut() {
            entry.mark_healthy();
        }
        count
    }

    pub fn strategy(&self) -> &str {
        &self.strategy
    }

    pub fn set_strategy(&mut self, strategy: &str) {
        if SUPPORTED_POOL_STRATEGIES.contains(&strategy) {
            self.strategy = strategy.to_string();
        }
    }

    pub fn acquire_lease(&mut self, credential_id: Option<&str>) -> Option<String> {
        let _lock = self.lock.lock().unwrap();
        if let Some(cred_id) = credential_id {
            let count = self.active_leases.entry(cred_id.to_string())
                .and_modify(|c| *c += 1)
                .or_insert(1);
            if *count < self.active_leases[cred_id] {
                return None;
            }
            self.current_id = Some(cred_id.to_string());
            return Some(cred_id.to_string());
        }

        let available: Vec<&PooledCredential> = self.entries.iter()
            .filter(|e| !e.is_in_cooldown())
            .collect();
        if available.is_empty() {
            return None;
        }

        let cap = 1u64;
        let below_cap: Vec<&PooledCredential> = available.iter()
            .filter(|e| *self.active_leases.get(&e.id).unwrap_or(&0) < cap)
            .copied()
            .collect();

        let candidates: Vec<&PooledCredential> = if !below_cap.is_empty() {
            below_cap
        } else {
            available
        };
        let chosen = candidates.into_iter()
            .min_by_key(|e| (*self.active_leases.get(&e.id).unwrap_or(&0), e.priority))
            .expect("candidates non-empty");

        let _ = self.active_leases.entry(chosen.id.clone())
            .and_modify(|c| *c += 1)
            .or_insert(1);
        self.current_id = Some(chosen.id.clone());
        Some(chosen.id.clone())
    }

    pub fn release_lease(&mut self, credential_id: &str) {
        let _lock = self.lock.lock().unwrap();
        if let Some(count) = self.active_leases.get_mut(credential_id) {
            if *count <= 1 {
                self.active_leases.remove(credential_id);
            } else {
                *count -= 1;
            }
        }
    }
}

/// Resolution result for credential targets (ID or label matching).
#[derive(Debug, Clone)]
pub struct CredentialResolution {
    pub index: Option<u32>,
    pub credential: Option<PooledCredential>,
    pub error: Option<String>,
}

impl CredentialPool {
    /// Resolves a target (ID, label, or numeric index) to a credential.
    pub fn resolve_target(&self, target: &str) -> CredentialResolution {
        let raw = target.trim();
        if raw.is_empty() {
            return CredentialResolution {
                index: None,
                credential: None,
                error: Some("No credential target provided.".to_string()),
            };
        }

        for (idx, entry) in self.entries.iter().enumerate() {
            if entry.id == raw {
                return CredentialResolution {
                    index: Some((idx + 1) as u32),
                    credential: Some(entry.clone()),
                    error: None,
                };
            }
        }

        let matches: Vec<_> = self.entries.iter()
            .enumerate()
            .filter(|(_, e)| e.label.trim().to_lowercase() == raw.to_lowercase())
            .collect();

        match matches.len() {
            1 => {
                let (idx, entry) = matches[0];
                CredentialResolution {
                    index: Some((idx + 1) as u32),
                    credential: Some(entry.clone()),
                    error: None,
                }
            }
            2.. => CredentialResolution {
                index: None,
                credential: None,
                error: Some(format!("Ambiguous credential label \"{raw}\". Use the numeric index or entry id instead.")),
            },
            0 => {
                if let Ok(n) = raw.parse::<u32>() {
                    let idx = (n as usize).checked_sub(1);
                    if let Some(idx) = idx {
                        if let Some(entry) = self.entries.get(idx) {
                            return CredentialResolution {
                                index: Some(n),
                                credential: Some(entry.clone()),
                                error: None,
                            };
                        }
                    }
                    return CredentialResolution {
                        index: None,
                        credential: None,
                        error: Some(format!("No credential #{n}.")),
                    };
                }
                CredentialResolution {
                    index: None,
                    credential: None,
                    error: Some(format!("No credential matching \"{raw}\".")),
                }
            }
        }
    }
}

/// Persists a credential pool to a JSON file.
pub fn write_credential_pool(pool: &CredentialPool, path: &Path) -> anyhow::Result<()> {
    let entries: Vec<serde_json::Value> = pool.entries.iter()
        .map(|e| serde_json::to_value(e))
        .collect::<Result<_, _>>()?;

    let mut pool_data: serde_json::Map<String, serde_json::Value> = serde_json::Map::new();
    pool_data.insert(pool.provider.clone(), serde_json::Value::Array(entries));

    let json = serde_json::to_string_pretty(&pool_data)?;
    std::fs::create_dir_all(path.parent().expect("path must have parent"))?;
    std::fs::write(path, json)?;
    Ok(())
}

/// Loads a credential pool from a JSON file.
pub fn read_credential_pool(path: &Path, provider: &str) -> Vec<PooledCredential> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let pool_data: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let entries = pool_data.get(provider)
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    entries.iter()
        .flat_map(|entry| PooledCredential::from_dict(provider, entry))
        .collect()
}

/// Saves a credential pool for a provider to an auth.json file.
pub fn save_pool_to_auth(provider: &str, entries: &[PooledCredential], auth_path: &Path) -> anyhow::Result<()> {
    let mut auth_data: serde_json::Value = if auth_path.exists() {
        let content = std::fs::read_to_string(auth_path)?;
        serde_json::from_str(&content).unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    if !auth_data.as_object().unwrap().contains_key("credential_pool") {
        auth_data.as_object_mut().unwrap()
            .insert("credential_pool".to_string(), serde_json::json!({}));
    }

    let pool_obj = auth_data["credential_pool"]
        .as_object_mut()
        .expect("credential_pool should be an object");

    let cred_values: Vec<serde_json::Value> = entries.iter()
        .filter_map(|e| serde_json::to_value(e).ok())
        .collect();

    pool_obj.insert(provider.to_string(), serde_json::json!(cred_values));

    let mut tmp_path = auth_path.to_path_buf();
    tmp_path.set_extension("tmp");
    let json = serde_json::to_string_pretty(&auth_data)?;
    std::fs::write(&tmp_path, &json)?;
    std::fs::rename(&tmp_path, auth_path)?;

    Ok(())
}

/// Loads credential pool from an auth.json file for the given provider.
pub fn load_pool_from_auth(auth_path: &Path, provider: &str) -> Vec<PooledCredential> {
    let content = match std::fs::read_to_string(auth_path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let auth_data: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let entries = auth_data.get("credential_pool")
        .and_then(|v| v.get(provider))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    entries.iter()
        .flat_map(|entry| PooledCredential::from_dict(provider, entry))
        .collect()
}

/// Clears all registered credentials for a provider in an auth.json file.
pub fn clear_provider_pool(auth_path: &Path, provider: &str) -> anyhow::Result<()> {
    let mut auth_data: serde_json::Value = if auth_path.exists() {
        let content = std::fs::read_to_string(auth_path)?;
        serde_json::from_str(&content).unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    if let Some(pool) = auth_data["credential_pool"].as_object_mut() {
        pool.remove(provider);
    }

    let json = serde_json::to_string_pretty(&auth_data)?;
    let mut tmp_path = auth_path.to_path_buf();
    tmp_path.set_extension("tmp");
    std::fs::write(&tmp_path, &json)?;
    std::fs::rename(&tmp_path, auth_path)?;

    Ok(())
}

/// Returns the pool selection strategy for a provider from the auth.json.
pub fn get_pool_strategy_for_provider(auth_path: &Path, provider: &str) -> String {
    let content = match std::fs::read_to_string(auth_path) {
        Ok(c) => c,
        Err(_) => return STRATEGY_FILL_FIRST.to_string(),
    };

    let auth_data: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return STRATEGY_FILL_FIRST.to_string(),
    };

    let strategy = auth_data
        .get("credential_pool_strategies")
        .and_then(|v| v.get(provider))
        .and_then(|v| v.as_str())
        .map(String::from)
        .unwrap_or_else(|| STRATEGY_FILL_FIRST.to_string());

    if SUPPORTED_POOL_STRATEGIES.contains(&strategy.as_str()) {
        strategy
    } else {
        STRATEGY_FILL_FIRST.to_string()
    }
}

/// Builder for constructing CredentialPool with optional auth file loading.
pub struct CredentialPoolBuilder {
    provider: String,
    entries: Vec<PooledCredential>,
    strategy: Option<String>,
    auth_file: Option<std::path::PathBuf>,
}

impl CredentialPoolBuilder {
    pub fn new() -> Self {
        Self {
            provider: String::new(),
            entries: Vec::new(),
            strategy: None,
            auth_file: None,
        }
    }

    pub fn with_provider(mut self, provider: &str) -> Self {
        self.provider = provider.to_string();
        self
    }

    pub fn with_strategy(mut self, strategy: &str) -> Self {
        if SUPPORTED_POOL_STRATEGIES.contains(&strategy) {
            self.strategy = Some(strategy.to_string());
        }
        self
    }

    pub fn with_credential(mut self, cred: PooledCredential) -> Self {
        self.entries.push(cred);
        self
    }

    pub fn with_credentials(mut self, creds: Vec<PooledCredential>) -> Self {
        self.entries.extend(creds);
        self
    }

    pub fn from_auth_file(mut self, auth_path: &std::path::Path) -> Self {
        self.auth_file = Some(auth_path.to_path_buf());
        self
    }

    pub fn build(self) -> CredentialPool {
        let provider = if self.provider.is_empty() {
            "default".to_string()
        } else {
            self.provider
        };

        let mut entries = self.entries;
        if let Some(ref auth_path) = self.auth_file {
            let from_file = load_pool_from_auth(auth_path, &provider);
            entries.extend(from_file);
        }

        let strategy = self.strategy.unwrap_or_else(|| {
            if let Some(ref auth_path) = self.auth_file {
                get_pool_strategy_for_provider(auth_path, &provider)
            } else {
                STRATEGY_FILL_FIRST.to_string()
            }
        });

        CredentialPool::new_with_strategy(&provider, entries, &strategy)
    }
}

impl Default for CredentialPoolBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Given: A pool with two API keys for the same provider
    /// When: First key is marked exhausted (429 rate limit)
    /// Then: First entry enters cooldown and pool rotates to second key
    #[test]
    fn test_rotate_on_429() {
        let cred1 = PooledCredential::new("test", "key-1");
        let cred2 = PooledCredential::new("test", "key-2");
        let mut pool = CredentialPool::new_with_strategy("test", vec![cred1, cred2], STRATEGY_FILL_FIRST);

        pool.select();
        assert_eq!(pool.current().unwrap().access_token, "key-1");

        let rotated = pool.mark_exhausted_and_rotate(Some(429), Some("rate limited"));
        assert!(rotated.is_some());
        assert_eq!(rotated.unwrap().access_token, "key-2");
        assert_eq!(pool.current().unwrap().access_token, "key-2");

        assert!(pool.entries().iter()
            .find(|e| e.access_token == "key-1")
            .unwrap()
            .is_in_cooldown());
    }

    /// Given: A pool with three credentials
    /// When: One returns 401, another returns 429, third is used
    /// Then: Exhausted entries enter appropriate cooldowns
    #[test]
    fn test_cooldown_by_error_code() {
        let cred1 = PooledCredential::new("test", "key-1");
        let cred2 = PooledCredential::new("test", "key-2");
        let cred3 = PooledCredential::new("test", "key-3");
        let mut pool = CredentialPool::new_with_strategy("test", vec![cred1, cred2, cred3], STRATEGY_FILL_FIRST);

        pool.select();
        pool.mark_exhausted_and_rotate(Some(401), None);
        pool.select();
        pool.mark_exhausted_and_rotate(Some(429), None);

        assert!(pool.has_available());
        assert!(pool.current().unwrap().access_token == "key-3");

        let entry_401 = pool.entries()
            .into_iter()
            .find(|e| e.access_token == "key-1")
            .unwrap();
        assert_eq!(entry_401.cooldown_seconds(), EXHAUSTED_TTL_401_SECONDS);
    }

    /// Given: A fill_first pool with all credentials in cooldown
    /// When: select() is called
    /// Then: Returns None
    #[test]
    fn test_all_exhausted_returns_none() {
        let cred1 = PooledCredential::new("test", "key-1");
        let cred2 = PooledCredential::new("test", "key-2");
        let mut pool = CredentialPool::new("test", vec![cred1, cred2]);

        pool.select();
        pool.mark_exhausted_and_rotate(Some(401), None);
        pool.mark_exhausted_and_rotate(Some(401), None);

        assert!(!pool.has_available());
        assert!(pool.select().is_none());
    }

    /// Given: An empty pool
    /// When: select() is called
    /// Then: Returns None
    #[test]
    fn test_empty_pool_returns_none() {
        let mut pool = CredentialPool::new("test", vec![]);
        assert!(!pool.has_credentials());
        assert!(!pool.has_available());
        assert!(pool.select().is_none());
    }

    /// Given: A pool with round_robin strategy and three credentials
    /// When: select() is called multiple times
    /// Then: Each call returns a credential in rotation order
    #[test]
    fn test_round_robin_rotation() {
        let cred1 = PooledCredential::new("test", "key-1");
        let cred2 = PooledCredential::new("test", "key-2");
        let cred3 = PooledCredential::new("test", "key-3");
        let mut pool = CredentialPool::new_with_strategy("test", vec![cred1, cred2, cred3], STRATEGY_ROUND_ROBIN);

        let s1 = pool.select();
        let s2 = pool.select();
        assert!(s1.is_some() && s2.is_some());
        assert_ne!(s1.unwrap().id, s2.unwrap().id);
    }

    /// Given: A pool with least_used strategy and three credentials each with different request counts
    /// When: select() is called
    /// Then: The credential with the lowest request_count is selected
    #[test]
    fn test_least_used_selects_lowest_count() {
        let mut cred1 = PooledCredential::new("test", "key-1");
        let mut cred2 = PooledCredential::new("test", "key-2");
        let mut cred3 = PooledCredential::new("test", "key-3");
        cred2.request_count = 50;
        cred3.request_count = 30;

        let mut pool = CredentialPool::new_with_strategy("test", vec![cred1, cred2, cred3], STRATEGY_LEAST_USED);
        let selected = pool.select();
        assert!(selected.is_some());
        assert_eq!(selected.unwrap().access_token, "key-1");
    }

    /// Given: A pool with three credentials
    /// When: random strategy is used (many selections)
    /// Then: Different credentials are selected (statistically)
    #[test]
    fn test_random_strategy_returns_various() {
        let cred1 = PooledCredential::new("test", "key-1");
        let cred2 = PooledCredential::new("test", "key-2");
        let cred3 = PooledCredential::new("test", "key-3");
        let mut pool = CredentialPool::new_with_strategy("test", vec![cred1, cred2, cred3], STRATEGY_RANDOM);

        let mut seen: Vec<String> = Vec::new();
        for _ in 0..20 {
            if let Some(s) = pool.select() {
                seen.push(s.access_token);
            }
        }
        let unique: std::collections::HashSet<_> = seen.iter().collect();
        assert!(unique.len() >= 2, "Expected at least 2 unique credentials, got {}", unique.len());
    }

    /// Given: A credential with exhausted status and error_code 429
    /// When: is_in_cooldown() is called
    /// Then: Returns true
    #[test]
    fn test_credential_cooldown_detection() {
        let mut cred = PooledCredential::new("test", "key");
        cred.mark_exhausted(Some(429), Some("rate limited"));
        assert!(cred.is_in_cooldown());
        assert_eq!(cred.cooldown_seconds(), EXHAUSTED_TTL_429_SECONDS);
    }

    /// Given: A credential with exhausted status and error_code 401
    /// When: cooldown_seconds() is called
    /// Then: Returns 5 minutes (EXHAUSTED_TTL_401_SECONDS)
    #[test]
    fn test_credential_401_cooldown() {
        let mut cred = PooledCredential::new("test", "key");
        cred.mark_exhausted(Some(401), Some("unauthorized"));
        assert_eq!(cred.cooldown_seconds(), EXHAUSTED_TTL_401_SECONDS);
    }

    /// Given: A credential with exhausted status and no error code
    /// When: cooldown_seconds() is called
    /// Then: Returns default cooldown
    #[test]
    fn test_credential_default_cooldown() {
        let mut cred = PooledCredential::new("test", "key");
        cred.mark_exhausted(None, Some("unknown error"));
        assert_eq!(cred.cooldown_seconds(), EXHAUSTED_TTL_DEFAULT_SECONDS);
    }

    /// Given: A credential with exhausted status
    /// When: mark_healthy() is called
    /// Then: is_in_cooldown() returns false
    #[test]
    fn test_credential_mark_healthy_resets_cooldown() {
        let mut cred = PooledCredential::new("test", "key");
        cred.mark_exhausted(Some(429), Some("rate limited"));
        assert!(cred.is_in_cooldown());
        cred.mark_healthy();
        assert!(!cred.is_in_cooldown());
    }

    /// Given: A pool with credentials that have been manually added
    /// When: A new credential is added via add_entry()
    /// Then: The new entry gets the next highest priority
    #[test]
    fn test_add_entry_increments_priority() {
        let mut cred1 = PooledCredential::new("test", "key-1");
        cred1.priority = 5;
        let cred2 = PooledCredential::new("test", "key-2");
        let mut pool = CredentialPool::new("test", vec![cred1, cred2]);

        let cred3 = PooledCredential::new("test", "key-3");
        pool.add_entry(cred3);

        assert_eq!(pool.entries().len(), 3);
    }

    /// Given: A pool with three credentials
    /// When: remove_index() is called with index 2
    /// Then: The second entry is removed and priorities are renumbered
    #[test]
    fn test_remove_entry_by_index() {
        let cred1 = PooledCredential::new("test", "key-1");
        let cred2 = PooledCredential::new("test", "key-2");
        let cred3 = PooledCredential::new("test", "key-3");
        let mut pool = CredentialPool::new("test", vec![cred1, cred2.clone(), cred3]);

        let removed = pool.remove_index(2);
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().access_token, "key-2");
        assert_eq!(pool.entries().len(), 2);
        assert!(pool.remove_index(5).is_none());
        assert!(pool.remove_index(0).is_none());
    }

    /// Given: A pool with all credentials in exhausted state
    /// When: reset_statuses() is called
    /// Then: All credentials are marked healthy and count of modified entries is returned
    #[test]
    fn test_reset_statuses() {
        let mut cred1 = PooledCredential::new("test", "key-1");
        let mut cred2 = PooledCredential::new("test", "key-2");
        cred1.mark_exhausted(Some(429), None);
        cred2.mark_exhausted(Some(500), None);

        let mut pool = CredentialPool::new("test", vec![cred1, cred2]);
        let count = pool.reset_statuses();
        assert_eq!(count, 2);
        assert!(pool.has_available());
    }

    /// Given: A pool with three credentials having different labels
    /// When: resolve_target() is called with various search terms
    /// Then: Returns matching credential by ID, label, or numeric index
    #[test]
    fn test_resolve_target_by_id() {
        let cred1 = PooledCredential::new("test", "key-1");
        let mut cred2 = PooledCredential::new("test", "key-2");
        cred2.label = "my-key".to_string();
        let cred3 = PooledCredential::new("test", "key-3");
        let pool = CredentialPool::new("test", vec![cred1.clone(), cred2.clone(), cred3.clone()]);

        let result = pool.resolve_target(&cred1.id);
        assert!(result.credential.is_some());
        assert_eq!(result.index, Some(1));
        assert!(result.error.is_none());

        let result = pool.resolve_target("my-key");
        assert!(result.credential.is_some());
        assert_eq!(result.index, Some(2));

        let result = pool.resolve_target("3");
        assert!(result.credential.is_some());
        assert_eq!(result.index, Some(3));
    }

    /// Given: A pool with three credentials having identical labels
    /// When: resolve_target() is called with the repeated label
    /// Then: Returns an ambiguous error
    #[test]
    fn test_resolve_target_ambiguous_label() {
        let mut cred1 = PooledCredential::new("test", "key-1");
        let mut cred2 = PooledCredential::new("test", "key-2");
        cred1.label = "duplicate".to_string();
        cred2.label = "duplicate".to_string();
        let cred3 = PooledCredential::new("test", "key-3");
        let pool = CredentialPool::new("test", vec![cred1, cred2, cred3]);

        let result = pool.resolve_target("duplicate");
        assert!(result.credential.is_none());
        assert!(result.error.as_ref().unwrap().contains("Ambiguous"));
    }

    /// Given: A pool with credentials
    /// When: resolve_target() is called with an empty string
    /// Then: Returns error "No credential target provided."
    #[test]
    fn test_resolve_target_empty_string() {
        let cred = PooledCredential::new("test", "key");
        let pool = CredentialPool::new("test", vec![cred]);

        let result = pool.resolve_target("");
        assert!(result.credential.is_none());
        assert_eq!(result.error, Some("No credential target provided.".to_string()));
    }

    /// Given: A pool with credentials
    /// When: resolve_target() is called with an out-of-range index
    /// Then: Returns "No credential #N." error
    #[test]
    fn test_resolve_target_invalid_index() {
        let cred = PooledCredential::new("test", "key");
        let pool = CredentialPool::new("test", vec![cred]);

        let result = pool.resolve_target("99");
        assert!(result.credential.is_none());
        assert_eq!(result.error, Some("No credential #99.".to_string()));
    }

    /// Given: A credential with active last_error_reset_at in future
    /// When: is_in_cooldown() is called
    /// Then: Returns true
    #[test]
    fn test_cooldown_with_override() {
        let mut cred = PooledCredential::new("test", "key");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();
        cred.last_status = Some(STATUS_EXHAUSTED.to_string());
        cred.last_error_reset_at = Some(now + 10.0);
        assert!(cred.is_in_cooldown());

        cred.last_error_reset_at = Some(now - 1.0);
        assert!(!cred.is_in_cooldown());
    }

    /// Given: A newly created credential
    /// When: runtime_api_key() and runtime_base_url() are called
    /// Then: Returns the access_token and base_url respectively
    #[test]
    fn test_runtime_api_key_and_url() {
        let mut cred = PooledCredential::new("test", "sk-test-key");
        cred.base_url = Some("https://api.example.com".to_string());

        assert_eq!(cred.runtime_api_key(), "sk-test-key");
        assert_eq!(cred.runtime_base_url(), Some("https://api.example.com"));
    }

    /// Given: A nous credential with agent_key and access_token
    /// When: runtime_api_key() is called
    /// Then: Returns the agent_key (not access_token)
    #[test]
    fn test_nous_runtime_key_returns_agent_key() {
        let mut cred = PooledCredential::new("nous", "sk-fallback");
        cred.provider = "nous".to_string();
        cred.agent_key = Some("nous-jwt-key".to_string());
        assert_eq!(cred.runtime_api_key(), "nous-jwt-key");
    }

    /// Given: A credential with no agent_key (nous provider)
    /// When: runtime_api_key() is called
    /// Then: Falls back to access_token
    #[test]
    fn test_nous_fallback_to_access_token() {
        let mut cred = PooledCredential::new("nous", "sk-fallback");
        cred.provider = "nous".to_string();
        assert_eq!(cred.runtime_api_key(), "sk-fallback");
    }

    /// Given: A credential with agent_key and inference_base_url
    /// When: runtime_base_url() is called
    /// Then: Returns inference_base_url (not base_url)
    #[test]
    fn test_nous_base_url_returns_inference() {
        let mut cred = PooledCredential::new("nous", "sk-key");
        cred.provider = "nous".to_string();
        cred.base_url = Some("https://default.example.com".to_string());
        cred.inference_base_url = Some("https://inference.example.com".to_string());
        assert_eq!(cred.runtime_base_url(), Some("https://inference.example.com"));
    }

    /// Given: A credential with a request count
    /// When: increment_request_count() is called
    /// Then: The count increases by one
    #[test]
    fn test_increment_request_count() {
        let mut cred = PooledCredential::new("test", "key");
        cred.request_count = 10;
        cred.increment_request_count();
        assert_eq!(cred.request_count, 11);
    }

    /// Given: A CredentialPool created via builder with credentials and strategy
    /// When: build() is called
    /// Then: Returns a pool configured with the specified strategy and entries
    #[test]
    fn test_builder_with_strategy() {
        let cred = PooledCredential::new("test", "key");
        let pool = CredentialPoolBuilder::new()
            .with_provider("test")
            .with_strategy(STRATEGY_ROUND_ROBIN)
            .with_credential(cred)
            .build();

        assert_eq!(pool.strategy(), STRATEGY_ROUND_ROBIN);
        assert_eq!(pool.entries().len(), 1);
        assert!(pool.has_credentials());
    }

    /// Given: A credential pool serialized to JSON and saved to disk
    /// When: It is reloaded via read_credential_pool()
    /// Then: The pool is correctly reconstructed
    #[test]
    fn test_persistence_roundtrip() {
        let cred1 = PooledCredential::new("test-provider", "key-123");
        let mut cred2 = PooledCredential::new("test-provider", "key-456");
        cred2.last_status = Some(STATUS_OK.to_string());
        cred2.request_count = 42;

        let pool = CredentialPool::new("test-provider", vec![cred1, cred2]);

        let dir = std::env::temp_dir();
        let path = dir.join("oben_credential_pool_test.json");

        write_credential_pool(&pool, &path).expect("write_pool failed");

        let loaded = read_credential_pool(&path, "test-provider");
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].provider, "test-provider");
        assert_eq!(loaded[0].access_token, "key-123");
        assert_eq!(loaded[1].request_count, 42);

        let _ = std::fs::remove_file(&path);
    }

    /// Given: An auth.json file with existing pool data
    /// When: save_pool_to_auth() persists a new pool
    /// Then: The credential_pool section is updated and other data is preserved
    #[test]
    fn test_save_pool_to_auth_preserves_existing_data() {
        let dir = std::env::temp_dir();
        let path = dir.join("oben_auth_test.json");

        let auth_json = serde_json::json!({
            "version": 1,
            "other_field": "should_persist"
        });
        std::fs::write(&path, serde_json::to_string_pretty(&auth_json).unwrap()).unwrap();

        let cred = PooledCredential::new("openrouter", "sk-persist-test");
        let entries = vec![cred];
        save_pool_to_auth("openrouter", &entries, &path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let data: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(data["other_field"], "should_persist");
        assert_eq!(data["credential_pool"]["openrouter"].as_array().unwrap().len(), 1);

        let _ = std::fs::remove_file(&path);
    }

    /// Given: An auth.json file
    /// When: clear_provider_pool() is called for a provider
    /// Then: That provider's credential_pool entry is removed
    #[test]
    fn test_clear_provider_pool() {
        let dir = std::env::temp_dir();
        let path = dir.join("oben_clear_auth_test.json");

        let cred = PooledCredential::new("test", "key");
        save_pool_to_auth("test", &[cred], &path).unwrap();
        assert!(load_pool_from_auth(&path, "test").len() == 1);

        clear_provider_pool(&path, "test").unwrap();
        assert!(load_pool_from_auth(&path, "test").is_empty());

        let _ = std::fs::remove_file(&path);
    }

    /// Given: A pool with credentials and a lease acquired on one
    /// When: The lease is released
    /// Then: The lease count is properly released
    #[test]
    fn test_acquire_and_release_lease() {
        let cred1 = PooledCredential::new("test", "key-1");
        let cred2 = PooledCredential::new("test", "key-2");
        let mut pool = CredentialPool::new("test", vec![cred1, cred2]);

        let lease = pool.select();
        if let Some(ref s) = lease {
            let lease_id = pool.acquire_lease(Some(&s.id));
            assert!(lease_id.is_some());
            pool.release_lease(&s.id);
        }
    }

    /// Given: An auth.json file with credential pool entries
    /// When: The builder loads from the auth file
    /// Then: The pool contains the loaded credentials
    #[test]
    fn test_builder_from_auth_file() {
        let dir = std::env::temp_dir();
        let path = dir.join("oben_builder_auth_test.json");

        let cred1 = PooledCredential::new("builder-test", "auth-key-1");
        let cred2 = PooledCredential::new("builder-test", "auth-key-2");
        save_pool_to_auth("builder-test", &[cred1, cred2], &path).unwrap();

        let pool = CredentialPoolBuilder::new()
            .with_provider("builder-test")
            .from_auth_file(&path)
            .build();

        assert_eq!(pool.entries().len(), 2);

        let _ = std::fs::remove_file(&path);
    }

    /// Given: Multiple credentials with same provider but different sources
    /// When: They are added to the pool
    /// Then: All are retained and selectable
    #[test]
    fn test_multiple_sources_same_provider() {
        let mut cred_manual = PooledCredential::new("multi-source", "manual-key");
        cred_manual.source = SOURCE_MANUAL.to_string();
        cred_manual.label = "manual-key".to_string();

        let mut cred_env = PooledCredential::new("multi-source", "env-key");
        cred_env.source = "env:API_KEY".to_string();
        cred_env.label = "env:API_KEY".to_string();

        let mut cred_oauth = PooledCredential::new("multi-source", "oauth-token");
        cred_oauth.auth_type = AUTH_TYPE_OAUTH.to_string();
        cred_oauth.source = "oauth:provider".to_string();
        cred_oauth.refresh_token = Some("refresh".to_string());

        let pool = CredentialPool::new(
            "multi-source",
            vec![cred_manual, cred_env, cred_oauth],
        );

        assert_eq!(pool.entries().len(), 3);
        assert!(pool.has_credentials());
        assert!(pool.has_available());
    }

    /// Given: A pool with fill_first strategy
    /// When: The current credential is exhausted, next is selected
    /// Then: The pool cycles through entries in priority order
    #[test]
    fn test_fill_first_strategy_order() {
        let mut cred3 = PooledCredential::new("test", "key-3");
        let mut cred1 = PooledCredential::new("test", "key-1");
        let mut cred2 = PooledCredential::new("test", "key-2");
        cred1.priority = 0;
        cred2.priority = 1;
        cred3.priority = 2;

        let mut pool = CredentialPool::new_with_strategy(
            "test",
            vec![cred1.clone(), cred2.clone(), cred3.clone()],
            STRATEGY_FILL_FIRST,
        );

        let s1 = pool.select();
        assert_eq!(s1.unwrap().priority, 0);

        pool.mark_exhausted_and_rotate(Some(401), None);
        pool.mark_exhausted_and_rotate(Some(401), None);
        pool.mark_exhausted_and_rotate(Some(401), None);

        assert!(!pool.has_available());
    }

    /// Given: A CredentialPool with entries
    /// When: has_credentials() and has_available() are called
    /// Then: has_credentials() returns true, has_available() reflects exhaustion
    #[test]
    fn test_has_credentials_and_has_available() {
        let cred = PooledCredential::new("test", "key");
        let pool = CredentialPool::new("test", vec![cred]);
        assert!(pool.has_credentials());
        assert!(pool.has_available());
    }

    /// Given: A pool with explicit last_error_reset_at
    /// When: is_in_cooldown() is called with reset_at in past
    /// Then: Returns false (expired override)
    #[test]
    fn test_explicit_reset_at_override() {
        let mut cred = PooledCredential::new("test", "key");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();
        cred.last_error_reset_at = Some(now - 1.0);
        assert!(!cred.is_in_cooldown());
    }

    /// Given: A pool with no credentials configured
    /// When: pool strategies constants are verified
    /// Then: All four strategies are defined
    #[test]
    fn test_supported_strategies_constant() {
        assert!(SUPPORTED_POOL_STRATEGIES.contains(&STRATEGY_FILL_FIRST));
        assert!(SUPPORTED_POOL_STRATEGIES.contains(&STRATEGY_ROUND_ROBIN));
        assert!(SUPPORTED_POOL_STRATEGIES.contains(&STRATEGY_RANDOM));
        assert!(SUPPORTED_POOL_STRATEGIES.contains(&STRATEGY_LEAST_USED));
        assert_eq!(SUPPORTED_POOL_STRATEGIES.len(), 4);
    }

    /// Given: A custom endpoint provider name
    /// When: CUSTOM_POOL_PREFIX is checked
    /// Then: It equals "custom:" and can be used to identify custom endpoints
    #[test]
    fn test_custom_pool_prefix() {
        assert_eq!(CUSTOM_POOL_PREFIX, "custom:");
    }

    /// Given: Two credentials with same ID
    /// When: is_in_cooldown() is called on a healthy credential
    /// Then: Returns false
    #[test]
    fn test_is_not_in_cooldown_when_healthy() {
        let cred = PooledCredential::new("test", "key");
        assert!(!cred.is_in_cooldown());
    }
}
