//! Per-provider model name normalization.
//!
//! Different LLM providers expect model identifiers in different formats:
//! - Aggregators (OpenRouter, Nous, etc.) need `vendor/model` slugs like `anthropic/claude-sonnet-4.6`.
//! - Anthropic native API expects bare names with dots replaced by hyphens: `claude-sonnet-4-6`.
//! - DeepSeek accepts `deepseek-chat` (V3), `deepseek-reasoner` (R1-family), and first-class V-series IDs.
//! - Custom providers pass the name through as-is.
//!
//! Maps to `hermes_cli/model_normalize.py`.

use regex::Regex;
use std::sync::LazyLock;

// ---------------------------------------------------------------------------
// Vendor prefix mapping
// ---------------------------------------------------------------------------

/// Maps the first hyphen-delimited token of a bare model name to the vendor slug.
static VENDOR_PREFIXES: LazyLock<std::collections::HashMap<&'static str, &'static str>> =
    LazyLock::new(|| {
        let mut m = std::collections::HashMap::new();
        m.insert("claude", "anthropic");
        m.insert("gpt", "openai");
        m.insert("o1", "openai");
        m.insert("o3", "openai");
        m.insert("o4", "openai");
        m.insert("gemini", "google");
        m.insert("gemma", "google");
        m.insert("deepseek", "deepseek");
        m.insert("glm", "z-ai");
        m.insert("kimi", "moonshotai");
        m.insert("minimax", "minimax");
        m.insert("grok", "x-ai");
        m.insert("qwen", "qwen");
        m.insert("mimo", "xiaomi");
        m.insert("trinity", "arcee-ai");
        m.insert("nemotron", "nvidia");
        m.insert("llama", "meta-llama");
        m.insert("step", "stepfun");
        m
    });

// Provider classifications
const AGGREGATOR_PROVIDERS: &[&str] = &[
    "openrouter",
    "nous",
    "ai-gateway",
    "kilocode",
    "opencode-zen",
    "opencode-go",
];
const DOT_TO_HYPHEN_PROVIDERS: &[&str] = &["anthropic"];
const STRIP_VENDOR_ONLY_PROVIDERS: &[&str] = &["copilot", "copilot-acp", "openai-codex"];
const AUTHORITATIVE_NATIVE_PROVIDERS: &[&str] = &["gemini", "huggingface"];
const MATCHING_PREFIX_STRIP_PROVIDERS: &[&str] = &[
    "zai",
    "kimi-coding",
    "kimi-coding-cn",
    "minimax",
    "minimax-oauth",
    "minimax-cn",
    "alibaba",
    "qwen-oauth",
    "xiaomi",
    "arcee",
    "ollama-cloud",
    "custom",
];
const LOWERCASE_MODEL_PROVIDERS: &[&str] = &["xiaomi"];

// ---------------------------------------------------------------------------
// DeepSeek special handling
// ---------------------------------------------------------------------------

static DEEPSEEK_V_SERIES_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^deepseek-v(\d+)([-.].+)?$").unwrap());

fn normalize_for_deepseek(model_name: &str) -> String {
    let bare = strip_vendor_prefix(model_name).to_lowercase();

    // Canonical models
    if bare == "deepseek-chat"
        || bare == "deepseek-reasoner"
        || bare == "deepseek-v4-pro"
        || bare == "deepseek-v4-flash"
    {
        return bare;
    }

    // V-series first-class IDs (v4+, not v3+)
    if let Some(m) = DEEPSEEK_V_SERIES_RE.captures(&bare) {
        let version = m
            .get(1)
            .and_then(|g| g.as_str().parse::<u32>().ok())
            .unwrap_or(0);
        if version >= 4 {
            return bare;
        }
    }

    let reasoner_keywords = ["reasoner", "r1", "think", "reasoning", "cot"];
    for kw in &reasoner_keywords {
        if bare.contains(kw) {
            return "deepseek-reasoner".to_string();
        }
    }

    "deepseek-chat".to_string()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn strip_vendor_prefix(model_name: &str) -> &str {
    if let Some((_, after)) = model_name.split_once('/') {
        after.trim()
    } else {
        model_name
    }
}

fn dots_to_hyphens(model_name: &str) -> String {
    model_name.replace('.', "-")
}

fn normalize_provider_alias(provider_name: &str) -> String {
    provider_name.trim().to_lowercase()
}

fn normalize_copilot_model_id(model_name: &str) -> String {
    let name = model_name.trim().to_lowercase();
    if name.is_empty() {
        return name;
    }

    let mut candidates = vec![model_name.to_string()];

    // Add bare name after vendor/
    if let Some((_, bare)) = model_name.split_once('/') {
        candidates.push(bare.trim().to_string());
        let trimmed = bare.trim();
        // Strip known suffixes
        let trimmed = trimmed
            .strip_suffix("-mini")
            .or_else(|| trimmed.strip_suffix("-nano"))
            .or_else(|| trimmed.strip_suffix("-chat"))
            .unwrap_or(trimmed);
        candidates.push(trimmed.to_string());
    }

    // Alias lookup (simplified — in prod this would query the copilot catalog)
    for candidate in &candidates {
        let c = candidate.to_lowercase();
        if c.contains("gpt-5") || c.contains("o3") || c.contains("o1") {
            if c.contains("mini") || c.contains("nano") {
                return "gpt-5-mini".to_string();
            }
            return "gpt-5.2".to_string();
        }
    }

    // Final fallback
    if model_name.contains('/') {
        let (_, bare) = model_name.split_once('/').unwrap_or(("", model_name));
        return bare.trim().to_string();
    }
    model_name.trim().to_string()
}

// ---------------------------------------------------------------------------
// Main normalizers
// ---------------------------------------------------------------------------

/// Detect the vendor slug from a bare model name (N.7).
pub fn detect_vendor(model_name: &str) -> Option<&'static str> {
    let name = model_name.trim();
    if name.is_empty() {
        return None;
    }

    // Already vendor-prefixed
    if let Some(prefix) = name.split_once('/') {
        return Some(prefix.0.to_lowercase().leak());
    }

    // Exact match on first hyphen-delimited token
    let first_token = name.split('-').next().unwrap_or(name).to_lowercase();
    if let Some(vendor) = VENDOR_PREFIXES.get(first_token.as_str()) {
        return Some(*vendor);
    }

    // Prefix starts-with match (e.g. "qwen3.5+" starts with "qwen")
    for (prefix, vendor) in VENDOR_PREFIXES.iter() {
        if name.to_lowercase().starts_with(prefix) {
            return Some(*vendor);
        }
    }

    None
}

/// Prepend detected vendor/ prefix to model name (N.1).
pub fn prepend_vendor(model_name: &str) -> String {
    if model_name.contains('/') {
        return model_name.to_string();
    }
    if let Some(vendor) = detect_vendor(model_name) {
        return format!("{}/{}", vendor, model_name);
    }
    model_name.to_string()
}

/// Strip vendor/ prefix if it matches the target provider (N.5).
pub fn strip_matching_provider_prefix(model_name: &str, target_provider: &str) -> String {
    let model_name = model_name.trim();
    if !model_name.contains('/') {
        return model_name.to_string();
    }

    let (prefix, remainder) = model_name.split_once('/').unwrap_or(("", model_name));
    if prefix.trim().is_empty() || remainder.trim().is_empty() {
        return model_name.to_string();
    }

    if normalize_provider_alias(prefix) == normalize_provider_alias(target_provider) {
        remainder.trim().to_string()
    } else {
        model_name.to_string()
    }
}

/// Normalize a model name for the target provider.
///
/// This is the primary entry point. Accepts any user-facing model identifier
/// and transforms it for the specific provider's API.
///
/// Implements N.1-N.7 from PRD-models-parity.md.
pub fn normalize_model_for_provider(model_input: &str, target_provider: &str) -> String {
    let name = model_input.trim();
    if name.is_empty() {
        return name.to_string();
    }

    let provider = normalize_provider_alias(target_provider);

    // Aggregators: need vendor/model format (N.1)
    if AGGREGATOR_PROVIDERS.contains(&provider.as_str()) {
        return prepend_vendor(name);
    }

    // OpenCode Zen / Go: strip vendor prefix, dots preserved except Claude
    if provider == "opencode-zen" || provider == "opencode-go" {
        let name_after = if let Some((_, bare)) = name.split_once('/') {
            bare.trim().to_string()
        } else {
            name.to_string()
        };
        if provider == "opencode-zen" && name_after.to_lowercase().starts_with("claude-") {
            return dots_to_hyphens(&name_after);
        }
        return name_after;
    }

    // Anthropic: strip matching prefix, dots -> hyphens (N.2)
    if DOT_TO_HYPHEN_PROVIDERS.contains(&provider.as_str()) {
        let bare = strip_matching_provider_prefix(name, &provider);
        if bare.contains('/') {
            return bare;
        }
        return dots_to_hyphens(&bare);
    }

    // Copilot: special handling
    if provider == "copilot" || provider == "copilot-acp" {
        return normalize_copilot_model_id(name);
    }

    // Copilot / Codex: strip matching prefix, dots preserved
    if STRIP_VENDOR_ONLY_PROVIDERS.contains(&provider.as_str()) {
        let stripped = strip_matching_provider_prefix(name, &provider);
        if stripped == name && name.starts_with("openai/") {
            return name
                .split_once('/')
                .unwrap_or(("", ""))
                .1
                .trim()
                .to_string();
        }
        return stripped;
    }

    // DeepSeek: canonical mapping (N.3)
    if provider == "deepseek" {
        let bare = strip_matching_provider_prefix(name, &provider);
        if bare.contains('/') {
            return bare;
        }
        return normalize_for_deepseek(&bare);
    }

    // Direct providers: strip matching prefix only (N.5)
    if MATCHING_PREFIX_STRIP_PROVIDERS.contains(&provider.as_str()) {
        let result = strip_matching_provider_prefix(name, &provider);
        if LOWERCASE_MODEL_PROVIDERS.contains(&provider.as_str()) {
            return result.to_lowercase();
        }
        return result;
    }

    // Authoritative native providers: pass through (N.6 for lowercase cases handled above)
    if AUTHORITATIVE_NATIVE_PROVIDERS.contains(&provider.as_str()) {
        return name.to_string();
    }

    // Custom & all others: pass through
    name.to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // N.7: detect_vendor
    #[test]
    fn test_detect_vendor_claude() {
        assert_eq!(detect_vendor("claude-sonnet-4.6"), Some("anthropic"));
    }

    #[test]
    fn test_detect_vendor_gpt() {
        assert_eq!(detect_vendor("gpt-5.4"), Some("openai"));
    }

    #[test]
    fn test_detect_vendor_qwen() {
        assert_eq!(detect_vendor("qwen3.5-plus"), Some("qwen"));
        assert_eq!(detect_vendor("qwen"), Some("qwen"));
    }

    #[test]
    fn test_detect_vendor_deepseek() {
        assert_eq!(detect_vendor("deepseek-chat"), Some("deepseek"));
    }

    #[test]
    fn test_detect_vendor_already_prefixed() {
        assert_eq!(
            detect_vendor("anthropic/claude-sonnet-4.6"),
            Some("anthropic")
        );
    }

    #[test]
    fn test_detect_vendor_none() {
        assert_eq!(detect_vendor("my-custom-model"), None);
    }

    // N.1: prepend_vendor
    #[test]
    fn test_prepend_vendor_claude() {
        assert_eq!(
            prepend_vendor("claude-sonnet-4.6"),
            "anthropic/claude-sonnet-4.6"
        );
    }

    #[test]
    fn test_prepend_vendor_already_prefixed() {
        assert_eq!(
            prepend_vendor("anthropic/claude-sonnet-4.6"),
            "anthropic/claude-sonnet-4.6"
        );
    }

    #[test]
    fn test_prepend_vendor_no_match() {
        assert_eq!(prepend_vendor("my-model"), "my-model");
    }

    // N.2: dots-to-hyphens for Anthropic
    #[test]
    fn test_dots_to_hyphens() {
        assert_eq!(dots_to_hyphens("claude-sonnet-4.6"), "claude-sonnet-4-6");
    }

    #[test]
    fn test_normalize_for_anthropic() {
        assert_eq!(
            normalize_model_for_provider("claude-sonnet-4.6", "anthropic"),
            "claude-sonnet-4-6"
        );
        assert_eq!(
            normalize_model_for_provider("anthropic/claude-sonnet-4.6", "anthropic"),
            "claude-sonnet-4-6"
        );
    }

    // N.3: DeepSeek canonical mapping
    #[test]
    fn test_deepseek_v3() {
        assert_eq!(
            normalize_model_for_provider("deepseek-v3", "deepseek"),
            "deepseek-chat"
        );
    }

    #[test]
    fn test_deepseek_r1() {
        assert_eq!(
            normalize_model_for_provider("deepseek-r1", "deepseek"),
            "deepseek-reasoner"
        );
    }

    #[test]
    fn test_deepseek_v4_pro_passthrough() {
        assert_eq!(
            normalize_model_for_provider("deepseek-v4-pro", "deepseek"),
            "deepseek-v4-pro"
        );
    }

    // N.1: aggregator vendor/model format
    #[test]
    fn test_aggregator_claude() {
        assert_eq!(
            normalize_model_for_provider("claude-sonnet-4.6", "openrouter"),
            "anthropic/claude-sonnet-4.6"
        );
    }

    #[test]
    fn test_aggregator_deepseek() {
        assert_eq!(
            normalize_model_for_provider("deepseek-v4-pro", "openrouter"),
            "deepseek/deepseek-v4-pro"
        );
    }

    // N.5: provider prefix stripping
    #[test]
    fn test_strip_matching_prefix() {
        assert_eq!(
            normalize_model_for_provider("zai/glm-5.1", "zai"),
            "glm-5.1"
        );
    }

    // N.6: lowercase for xiaomi
    #[test]
    fn test_xiaomi_lowercase() {
        assert_eq!(
            normalize_model_for_provider("MiMo-V2.5-Pro", "xiaomi"),
            "mimo-v2.5-pro"
        );
    }

    // Copilot special handling
    #[test]
    fn test_copilot_strip_prefix() {
        assert_eq!(
            normalize_copilot_model_id("openai/gpt-5-mini"),
            "gpt-5-mini"
        );
    }

    // DeepSeek reasoner keyword
    #[test]
    fn test_deepseek_reasoner_keywords() {
        assert_eq!(
            normalize_model_for_provider("deepseek-think", "deepseek"),
            "deepseek-reasoner"
        );
        assert_eq!(
            normalize_model_for_provider("deepseek-cot", "deepseek"),
            "deepseek-reasoner"
        );
    }

    // Pass-through for custom
    #[test]
    fn test_custom_passthrough() {
        assert_eq!(
            normalize_model_for_provider("my-custom-model", "custom"),
            "my-custom-model"
        );
    }

    // Authoritative native: Gemini pass-through
    #[test]
    fn test_gemini_passthrough() {
        assert_eq!(
            normalize_model_for_provider("gemini-2.0-flash", "gemini"),
            "gemini-2.0-flash"
        );
    }
}
