/// Plugin kinds — control loading behavior.
///
/// Maps to Hermes' `_VALID_PLUGIN_KINDS` which defines:
/// - `standalone` (default): hooks/tools of its own; opt-in via `plugins.enabled`
/// - `backend`: pluggable backend for existing core tool (auto-load if bundled, opt-in if user)
/// - `exclusive`: category with exactly one active provider (e.g. memory)
/// - `platform`: gateway messaging platform adapter (auto-load if bundled)
/// - `model-provider`: handled by provider discovery (auto-load)

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// Valid plugin kinds that control loading behavior.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginKind {
    /// Standalone plugin: its own hooks/tools; opt-in via `plugins.enabled`.
    #[serde(rename = "standalone")]
    Standalone,

    /// Pluggable backend for an existing core tool (e.g. image_gen).
    /// Bundled backends auto-load; user-installed still gated by `plugins.enabled`.
    #[serde(rename = "backend")]
    Backend,

    /// Category with exactly one active provider (e.g. memory).
    /// Selection via `<category>.provider` config key.
    #[serde(rename = "exclusive")]
    Exclusive,

    /// Gateway messaging platform adapter (e.g. IRC).
    /// Bundled platform plugins auto-load.
    #[serde(rename = "platform")]
    Platform,

    /// Model provider (handled by `oben-transport`/`oben-models` provider discovery).
    #[serde(rename = "model-provider")]
    ModelProvider,
}

impl Default for PluginKind {
    fn default() -> Self {
        PluginKind::Standalone
    }
}

impl PluginKind {
    /// Returns all valid plugin kind variants.
    pub fn all() -> &'static [Self] {
        &[
            Self::Standalone,
            Self::Backend,
            Self::Exclusive,
            Self::Platform,
            Self::ModelProvider,
        ]
    }

    /// Returns true if this kind can be auto-loaded when bundled.
    pub fn auto_load_when_bundled(&self) -> bool {
        matches!(self, Self::Backend | Self::Platform | Self::ModelProvider)
    }

    /// Returns true if this kind has its own dedicated discovery/activation path.
    pub fn is_exclusive(&self) -> bool {
        matches!(self, Self::Exclusive | Self::ModelProvider)
    }
}

impl FromStr for PluginKind {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_lowercase().as_str() {
            "standalone" => Ok(Self::Standalone),
            "backend" => Ok(Self::Backend),
            "exclusive" => Ok(Self::Exclusive),
            "platform" => Ok(Self::Platform),
            "model-provider" => Ok(Self::ModelProvider),
            _ => Err(anyhow!("Unknown plugin kind: '{}'. Valid kinds: {}",
                s,
                Self::all().iter()
                    .map(|k| format!("'{}'", k.as_str()))
                    .collect::<Vec<_>>()
                    .join(", "))),
        }
    }
}

impl PluginKind {
    /// Serialize to string for config storage.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Standalone => "standalone",
            Self::Backend => "backend",
            Self::Exclusive => "exclusive",
            Self::Platform => "platform",
            Self::ModelProvider => "model-provider",
        }
    }
}

/// Check if a kind string is valid.
pub fn is_valid_plugin_kind(kind: &str) -> bool {
    kind.trim().to_lowercase() == "standalone"
        || kind.trim().to_lowercase() == "backend"
        || kind.trim().to_lowercase() == "exclusive"
        || kind.trim().to_lowercase() == "platform"
        || kind.trim().to_lowercase() == "model-provider"
}

/// Convert a kind string to PluginKind, defaulting to Standalone for unknown kinds.
/// Returns the parsed kind, or Standalone if the kind is unknown (with a warning).
pub fn parse_plugin_kind(s: &str) -> PluginKind {
    match s.trim().to_lowercase().as_str() {
        "standalone" => PluginKind::Standalone,
        "backend" => PluginKind::Backend,
        "exclusive" => PluginKind::Exclusive,
        "platform" => PluginKind::Platform,
        "model-provider" => PluginKind::ModelProvider,
        _ => {
            tracing::warn!("Unknown plugin kind '{}', defaulting to 'standalone'", s);
            PluginKind::Standalone
        }
    }
}
