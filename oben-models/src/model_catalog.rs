/// Remote model catalog (M.7 + C.1).
///
/// Fetches provider model metadata from a remote manifest, caches it on disk
/// with a 24h TTL, and provides lookup APIs. Falls back to the built-in
/// PROVIDER_META when the remote fetch fails.
///
/// **Reference:** `hermes-agent/hermes_cli/model_catalog.py`

use std::collections::hash_map::Keys;
use std::io::Read;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::Result;
use chrono::DateTime;
use chrono::Utc;
use once_cell::sync::OnceCell;
use serde::Deserialize;
use serde::Serialize;

/// Remote catalog manifest for a single provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCatalog {
    /// Free-form provider metadata.
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
    /// List of models provided by this provider.
    #[serde(default)]
    pub models: Vec<RemoteModel>,
}

/// A single model entry from the remote catalog.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteModel {
    /// e.g. "anthropic/claude-sonnet-4-20250514".
    pub id: String,
    /// Human-readable description / recommendation status.
    #[serde(default)]
    pub description: Option<String>,
    /// Arbitrary extra metadata.
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

/// The root catalog manifest fetched from the remote URL.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogManifest {
    /// Schema version (we expect 1).
    pub version: u32,
    /// When the manifest was last updated remotely.
    #[serde(default, deserialize_with = "opt_datetime")]
    pub updated_at: Option<DateTime<Utc>>,
    /// Global catalog metadata.
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
    /// Per-provider catalog data keyed by provider canonical name.
    #[serde(default)]
    pub providers: std::collections::HashMap<String, ProviderCatalog>,
}

fn opt_datetime<'de, D>(deserializer: D) -> Result<Option<DateTime<Utc>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s: Option<String> = Option::deserialize(deserializer)?;
    match s {
        Some(val) => Ok(Some(chrono::DateTime::parse_from_rfc3339(&val)
            .map_err(serde::de::Error::custom)?
            .with_timezone(&Utc))),
        None => Ok(None),
    }
}

impl CatalogManifest {
    /// Default manifest URL (mirrors Hermes-Agent).
    pub const DEFAULT_URL: &'static str =
        "https://hermes-agent.nousresearch.com/docs/api/model-catalog.json";
    /// Cache TTL in seconds (24h).
    pub const TTL_SECS: u64 = 24 * 3600;
    /// Fetch timeout in seconds.
    pub const FETCH_TIMEOUT_SECS: u64 = 8;

    /// Cache directory path.
    pub fn cache_dir() -> PathBuf {
        dirs::config_dir()
            .map(|d| d.join("oben").join("cache"))
            .unwrap_or_else(|| PathBuf::from("/tmp/oben/cache"))
    }

    /// Cache file path.
    pub fn cache_file() -> PathBuf {
        Self::cache_dir().join("model_catalog.json")
    }

    /// Find a model by its ID across all providers.
    pub fn find_model(&self, model_id: &str) -> Option<&RemoteModel> {
        self.providers.values().flat_map(|p| p.models.iter()).find(|m| m.id == model_id)
    }

    /// Returns all model IDs (one per provider).
    pub fn provider_names(&self) -> Keys<String, ProviderCatalog> {
        self.providers.keys()
    }

    /// Fetch the catalog from the remote URL.
    ///
    /// Returns None on any network or parse error.
    pub fn fetch(url: &str) -> Option<CatalogManifest> {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(Self::FETCH_TIMEOUT_SECS))
            .no_proxy()
            .build()
            .ok()?;
        let resp = client.get(url).send().ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let bytes = resp.bytes().ok()?;
        Self::parse(&bytes)
    }

    /// Parse a CatalogManifest from raw JSON bytes.
    pub fn parse(bytes: &[u8]) -> Option<CatalogManifest> {
        let data: serde_json::Value = serde_json::from_slice(bytes).ok()?;
        if !Self::validate(&data) {
            return None;
        }
        serde_json::from_value(data).ok()
    }

    /// Validate the manifest schema (version = 1, has providers with models).
    pub fn validate(data: &serde_json::Value) -> bool {
        let Some(map) = data.as_object() else { return false };
        // version must be present and == 1
        #[allow(clippy::cast_possible_truncation)]
        let version = if let Some(v) = map.get("version").and_then(|v| v.as_u64()) {
            v as u32
        } else {
            return false;
        };
        if version != 1 {
            return false;
        }
        let Some(providers) = map.get("providers").and_then(|v| v.as_object()) else {
            return false;
        };
        for (name, block) in providers {
            if !name.is_empty() {
                let models_block = if let Some(models) = block.get("models").and_then(|v| v.as_array()) {
                    models
                } else {
                    return false;
                };
                for model in models_block {
                    let Some(model_obj) = model.as_object() else { continue };
                    if !model_obj.contains_key("id") {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// Load (and optionally fetch) the model catalog.
    ///
    /// Strategy:
    /// - Check in-memory cache first.
    /// - If disk cache exists and is younger than TTL, use it.
    /// - Fetch from network and cache.
    /// - On any failure, fall back to disk cache or return empty.
    ///
    /// If `force_refresh` is true, skip disk cache and fetch immediately.
    pub fn load(force_refresh: bool) -> CatalogManifest {
        // 1. Try in-memory cache.
        if let Some(manifest) = cache_in_mem() {
            return manifest;
        }

        // 2. Try disk cache.
        if !force_refresh {
            if let Some((data, age)) = cache_disk() {
                if age < Duration::from_secs(CatalogManifest::TTL_SECS) {
                    // Cache is fresh — parse, store in memory, and return.
                    if let Some(manifest) = Self::parse(&data) {
                        INST_CACHE.set((data, Instant::now())).ok();
                        return manifest;
                    }
                }
            }
        }

        // 3. Try fetching.
        let manifest = Self::fetch(Self::DEFAULT_URL);
        if let Some(ref m) = manifest {
            let json = serde_json::to_vec(&m).unwrap_or_default();
            write_cache(&json);
            INST_CACHE.set((json, Instant::now())).ok();
            return m.clone();
        }

        // 4. Fallback: return empty manifest on any failure.
        CatalogManifest {
            version: 1,
            updated_at: None,
            metadata: None,
            providers: std::collections::HashMap::new(),
        }
    }

    /// Load and return in-memory cache (for callers that already have a Mutex handle).
    pub fn get() -> CatalogManifest {
        Self::load(false)
    }

    /// Force reload from remote (ignoring disk cache).
    pub fn refresh() -> CatalogManifest {
        Self::load(true)
    }
}

/// In-memory cache for the manifest.
static INST_CACHE: OnceCell<(Vec<u8>, Instant)> = OnceCell::new();

/// Try the in-memory cache first (hit on repeated calls within the same process).
fn cache_in_mem() -> Option<CatalogManifest> {
    INST_CACHE.get().map(|(bytes, _)| {
        serde_json::from_slice(bytes).expect("in-memory cache was validated on write")
    })
}

/// Try the disk cache and return (parsed_data, file_mtime).
fn cache_disk() -> Option<(Vec<u8>, Duration)> {
    let path = CatalogManifest::cache_file();
    let mut data = Vec::new();
    if let Ok(mut f) = std::fs::File::open(&path) {
        if f.read_to_end(&mut data).is_err() {
            return None;
        }
    } else {
        return None;
    }
    if data.is_empty() {
        return None;
    }
    let meta = path.metadata().ok()?;
    let mtime = meta.modified().ok()?;
    let now = std::time::SystemTime::now();
    let age = now.duration_since(mtime).unwrap_or_else(|_| Duration::ZERO);
    Some((data, age))
}

/// Write data to the cache file atomically.
fn write_cache(data: &[u8]) {
    let cache_file = CatalogManifest::cache_file();
    let tmp = cache_file.with_extension("json.tmp");
    if let Err(e) = std::fs::write(&tmp, data) {
        tracing::warn!("Failed to write model catalog cache ({e})");
        return;
    }
    if let Err(e) = std::fs::rename(&tmp, &cache_file) {
        tracing::warn!("Failed to rename cache file ({e})");
        return;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_validate_valid_catalog() {
        let data = json!({
            "version": 1,
            "providers": {
                "openrouter": {
                    "models": [{"id": "a/b", "description": "desc"}]
                }
            }
        });
        assert!(CatalogManifest::validate(&data));
    }

    #[test]
    fn test_validate_missing_version() {
        let data = json!({"providers": {"openrouter": {"models": []}}});
        assert!(!CatalogManifest::validate(&data));
    }

    #[test]
    fn test_validate_bad_version() {
        let data = json!({"version": 2, "providers": {}});
        assert!(!CatalogManifest::validate(&data));
    }

    #[test]
    fn test_parse_valid_catalog() {
        let data = json!({
            "version": 1,
            "updated_at": "2026-05-26T20:49:36Z",
            "providers": {
                "openrouter": {
                    "models": [
                        {"id": "a/b", "description": "recommended"},
                        {"id": "x/y"}
                    ]
                },
                "nous": { "models": [] }
            }
        });
        let manifest = CatalogManifest::parse(&serde_json::to_vec(&data).unwrap()).unwrap();
        assert_eq!(manifest.providers.len(), 2);
        let openrouter = manifest.providers.get("openrouter").unwrap();
        assert_eq!(openrouter.models.len(), 2);
        assert_eq!(openrouter.models[0].description.as_deref(), Some("recommended"));
    }

    #[test]
    fn test_find_model() {
        let data = json!({
            "version": 1,
            "providers": {
                "openrouter": {
                    "models": [{"id": "a/b", "description": "recommended"}]
                }
            }
        });
        let manifest = CatalogManifest::parse(&serde_json::to_vec(&data).unwrap()).unwrap();
        assert!(manifest.find_model("a/b").is_some());
        assert!(manifest.find_model("no/such").is_none());
    }
}
