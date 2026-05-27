//! Usage pricing — per-model token cost computation and usage tracking.
//!
//! Supports: Claude, GPT-4/4o/o3, Gemini, DeepSeek, Bedrock, MiniMax + local providers.

use std::collections::HashMap;

use std::sync::LazyLock;

/// Input/output token count for a single API call.
#[derive(Debug, Clone, Default)]
pub struct TokenUsage {
    pub input_tokens: usize,
    pub output_tokens: usize,
    pub cache_read_tokens: usize,
    pub cache_write_tokens: usize,
}

impl TokenUsage {
    pub fn prompt_tokens(&self) -> usize {
        self.input_tokens + self.cache_read_tokens + self.cache_write_tokens
    }

    pub fn total_tokens(&self) -> usize {
        self.prompt_tokens() + self.output_tokens
    }
}

/// Pricing entry for a specific model+provider.
/// Costs are in USD per 1 million tokens.
#[derive(Debug, Clone)]
pub struct PricingEntry {
    pub input_cost_per_million: Option<f64>,
    pub output_cost_per_million: Option<f64>,
    pub cache_read_cost_per_million: Option<f64>,
    pub cache_write_cost_per_million: Option<f64>,
    pub request_cost: Option<f64>,
}

/// Result of a cost estimation.
#[derive(Debug, Clone)]
pub struct CostResult {
    pub amount_usd: Option<f64>,
    pub status: CostStatus,
    pub label: String,
}

/// Cost status classification.
#[derive(Debug, Clone, PartialEq)]
pub enum CostStatus {
    Included,    // Free or subscription-included
    Known,       // Has pricing data
    Estimated,   // Estimated from similar models
    Unknown,     // No pricing data available
}

impl std::fmt::Display for CostStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CostStatus::Included => write!(f, "included"),
            CostStatus::Known => write!(f, "known"),
            CostStatus::Estimated => write!(f, "estimated"),
            CostStatus::Unknown => write!(f, "unknown"),
        }
    }
}

/// Route information for billing.
#[derive(Debug, Clone)]
pub struct BillingRoute {
    pub provider: String,
    pub model: String,
}

#[derive(Debug, Clone, Default)]
pub struct PricingStore {
    entries: HashMap<(String, String), PricingEntry>,
}

impl PricingStore {
    pub fn new() -> Self {
        let mut store = Self {
            entries: HashMap::new(),
        };
        store.load_defaults();
        store
    }

    fn load_defaults(&mut self) {
        // ── Anthropic Claude ────────────────────────────────────────
        self.add_pricing("anthropic", "claude-opus-4-20250514", Some(15.0), Some(75.0), Some(1.50), Some(18.75));
        self.add_pricing("anthropic", "claude-sonnet-4-20250514", Some(3.0), Some(15.0), Some(0.30), Some(3.75));
        self.add_pricing("anthropic", "claude-sonnet-4-5", Some(3.0), Some(15.0), Some(0.30), Some(3.75));
        self.add_pricing("anthropic", "claude-sonnet-4-6", Some(3.0), Some(15.0), Some(0.30), Some(3.75));
        self.add_pricing("anthropic", "claude-opus-4-7", Some(5.0), Some(25.0), Some(0.50), Some(6.25));
        self.add_pricing("anthropic", "claude-opus-4-6", Some(5.0), Some(25.0), Some(0.50), Some(6.25));
        self.add_pricing("anthropic", "claude-haiku-4-5", Some(1.0), Some(5.0), Some(0.10), Some(1.25));
        self.add_pricing("anthropic", "claude-3-5-sonnet-20241022", Some(3.0), Some(15.0), Some(0.30), Some(3.75));
        self.add_pricing("anthropic", "claude-3-haiku-20240307", Some(0.25), Some(1.25), Some(0.03), Some(0.30));
        self.add_pricing("anthropic", "claude-3-opus-20240229", Some(15.0), Some(75.0), Some(1.50), Some(18.75));
        self.add_pricing("anthropic", "claude-3-5-haiku-20241022", Some(0.80), Some(4.0), Some(0.08), Some(1.00));

        // ── OpenAI ────────────────────────────────────────────────
        self.add_pricing("openai", "gpt-4o", Some(2.50), Some(10.0), Some(1.25), None);
        self.add_pricing("openai", "gpt-4o-mini", Some(0.15), Some(0.60), Some(0.075), None);
        self.add_pricing("openai", "gpt-4.1", Some(2.00), Some(8.0), Some(0.50), None);
        self.add_pricing("openai", "gpt-4.1-mini", Some(0.40), Some(1.60), Some(0.10), None);
        self.add_pricing("openai", "gpt-4.1-nano", Some(0.10), Some(0.40), Some(0.025), None);
        self.add_pricing("openai", "o3", Some(10.0), Some(40.0), Some(2.50), None);
        self.add_pricing("openai", "o3-mini", Some(1.10), Some(4.40), Some(0.55), None);

        // ── DeepSeek ──────────────────────────────────────────────
        self.add_pricing("deepseek", "deepseek-chat", Some(0.14), Some(0.28), None, None);
        self.add_pricing("deepseek", "deepseek-reasoner", Some(0.55), Some(2.19), None, None);
        self.add_pricing("deepseek", "deepseek-v4-pro", Some(1.74), Some(3.48), Some(0.0145), None);

        // ── Google Gemini ─────────────────────────────────────────
        self.add_pricing("google", "gemini-2.5-pro", Some(1.25), Some(10.0), None, None);
        self.add_pricing("google", "gemini-2.5-flash", Some(0.15), Some(0.60), None, None);
        self.add_pricing("google", "gemini-2.0-flash", Some(0.10), Some(0.40), None, None);

        // ── MiniMax ───────────────────────────────────────────────
        self.add_pricing("minimax", "minimax-m2.7", Some(0.30), Some(1.20), None, None);
        self.add_pricing("minimax-cn", "minimax-m2.7", Some(0.30), Some(1.20), None, None);
    }

    pub fn add_pricing(
        &mut self,
        provider: &str,
        model: &str,
        input: Option<f64>,
        output: Option<f64>,
        cache_read: Option<f64>,
        cache_write: Option<f64>,
    ) {
        let key = (provider.to_lowercase(), model.to_lowercase());
        self.entries.insert(key, PricingEntry {
            input_cost_per_million: input,
            output_cost_per_million: output,
            cache_read_cost_per_million: cache_read,
            cache_write_cost_per_million: cache_write,
            request_cost: None,
        });
    }

    pub fn add_per_request(&mut self, provider: &str, model: &str, cost: f64) {
        let key = (provider.to_lowercase(), model.to_lowercase());
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.request_cost = Some(cost);
        } else {
            self.entries.insert(key, PricingEntry {
                input_cost_per_million: None,
                output_cost_per_million: None,
                cache_read_cost_per_million: None,
                cache_write_cost_per_million: None,
                request_cost: Some(cost),
            });
        }
    }

    pub fn get_pricing(&self, provider: &str, model: &str) -> Option<&PricingEntry> {
        self.entries.get(&(provider.to_lowercase(), model.to_lowercase()))
    }

    /// Infer provider from model name patterns (e.g. "claude-4" -> "anthropic").
    fn infer_provider_from_model(model: &str) -> &'static str {
        let lower = model.to_lowercase();
        if lower.starts_with("claude") {
            return "anthropic";
        }
        if lower.starts_with("opus") || lower.starts_with("sonnet") || lower.starts_with("haiku") || lower.starts_with("codex") {
            return "anthropic";
        }
        if lower.starts_with("gpt") || lower.starts_with("o1") || lower.starts_with("o3") {
            return "openai";
        }
        if lower.starts_with("gemini") {
            return "google";
        }
        if lower.starts_with("deepseek") {
            return "deepseek";
        }
        if lower.starts_with("nova") {
            return "bedrock";
        }
        if lower.starts_with("minimax") {
            return "minimax";
        }
        ""
    }

    /// Resolve a billing route from model name with optional provider prefix.
    /// Automatically infers provider from model name if not given.
    pub fn resolve_route(&self, model_name: &str, provider: Option<&str>) -> BillingRoute {
        let model = model_name.trim();
        let (prov, bare_model);

        // If provider given, use it
        if let Some(pr) = provider {
            let provider_name = pr.trim().to_lowercase();
            if provider_name.is_empty() {
                // Fallback to inference
                let inferred = Self::infer_provider_from_model(model);
                prov = if inferred.is_empty() { "unknown".to_string() } else { inferred.to_string() };
            } else {
                prov = provider_name;
            }
            bare_model = model.split('/').last().unwrap_or(model).to_string();
        } else {
            // Try to infer from model name
            // First check if model has explicit provider prefix
            if model.contains('/') {
                let parts: Vec<_> = model.splitn(2, '/').collect();
                prov = parts.first().unwrap_or(&"unknown").to_string();
                bare_model = parts.get(1).map(|s| s.to_string()).unwrap_or(model.to_string());
            } else {
                // Infer from model name itself
                let inferred = Self::infer_provider_from_model(model);
                prov = if inferred.is_empty() { "unknown".to_string() } else { inferred.to_string() };
                bare_model = model.to_string();
            }
        }

        // Handle special billing modes
        if prov == "openai-codex" || prov == "openrouter" {
            return BillingRoute { provider: prov, model: bare_model };
        }
        if prov == "custom" || prov == "local" {
            return BillingRoute { provider: prov, model: bare_model };
        }

        BillingRoute { provider: prov, model: bare_model }
    }

    /// Compute cost for a single turn, given an optional pricing store.
    /// Returns Some(CostResult) always.
    pub fn cost_for_turn(&self, provider: &str, model: &str, usage: &TokenUsage) -> Option<CostResult> {
        // Handle local/custom/frozen providers
        if provider == "custom" || provider == "local" {
            return Some(CostResult {
                amount_usd: Some(0.0),
                status: CostStatus::Included,
                label: "free".to_string(),
            });
        }
        // OpenRouter and codex use special billing
        if provider == "openrouter" || provider == "openai-codex" {
            // Still look for per-request pricing
            let key = (provider.to_string(), model.to_lowercase());
            if let Some(entry) = self.entries.get(&key) {
                if entry.request_cost.is_some() {
                    let cost = entry.request_cost.unwrap();
                    return Some(CostResult {
                        amount_usd: Some(cost),
                        status: CostStatus::Known,
                        label: format!("${:.2}", cost),
                    });
                }
            }
            return Some(CostResult {
                amount_usd: None,
                status: CostStatus::Unknown,
                label: "n/a".to_string(),
            });
        }

        let entry = self.get_pricing(provider, model);
        if entry.is_none() {
            return Some(CostResult {
                amount_usd: None,
                status: CostStatus::Unknown,
                label: "n/a".to_string(),
            });
        }

        let entry = entry.unwrap();
        let mut amount = 0.0;

        if usage.input_tokens > 0 {
            if let Some(cost) = entry.input_cost_per_million {
                amount += usage.input_tokens as f64 * cost / 1_000_000.0;
            } else {
                return Some(CostResult {
                    amount_usd: None,
                    status: CostStatus::Unknown,
                    label: "n/a".to_string(),
                });
            }
        }

        if usage.output_tokens > 0 {
            if let Some(cost) = entry.output_cost_per_million {
                amount += usage.output_tokens as f64 * cost / 1_000_000.0;
            } else {
                return Some(CostResult {
                    amount_usd: None,
                    status: CostStatus::Unknown,
                    label: "n/a".to_string(),
                });
            }
        }

        if usage.cache_read_tokens > 0 {
            if let Some(cost) = entry.cache_read_cost_per_million {
                amount += usage.cache_read_tokens as f64 * cost / 1_000_000.0;
            }
        }

        if usage.cache_write_tokens > 0 {
            if let Some(cost) = entry.cache_write_cost_per_million {
                amount += usage.cache_write_tokens as f64 * cost / 1_000_000.0;
            }
        }

        if let Some(cost) = entry.request_cost {
            amount += cost; // per-request cost
        }

        Some(CostResult {
            amount_usd: Some(amount),
            status: CostStatus::Known,
            label: format!("${:.4}", amount),
        })
    }

    /// Check if we have known pricing for a model+provider.
    pub fn has_known_pricing(&self, provider: &str, model: &str) -> bool {
        self.get_pricing(provider, model).is_some()
    }

    /// Get a user-friendly provider name.
    pub fn provider_display_name(&self, provider: &str) -> &'static str {
        match provider.to_lowercase().as_str() {
            "anthropic" => "Anthropic",
            "openai" => "OpenAI",
            "deepseek" => "DeepSeek",
            "google" => "Google",
            "bedrock" => "AWS Bedrock",
            "minimax" | "minimax-cn" => "MiniMax",
            "custom" | "local" => "Local",
            _ => "Unknown",
        }
    }
}

static DEFAULT_PRICING: LazyLock<PricingStore> = LazyLock::new(PricingStore::new);

/// Shorthand: compute cost for a model using the default pricing store.
pub fn has_pricing(model: &str) -> bool {
    let route = DEFAULT_PRICING.resolve_route(model, None);
    DEFAULT_PRICING.has_known_pricing(&route.provider, &route.model)
}

/// Format a single cost entry as a displayable line.
pub fn format_cost_line(label: &str, cost: &CostResult) -> String {
    let cost_text = match (&cost.amount_usd, &cost.status) {
        (Some(_), CostStatus::Included) => "included".to_string(),
        (Some(c), _) => format!("${:.4}", c),
        (None, _) => "unknown".to_string(),
    };
    format!("  {:.<width$} {}", label, cost_text, width = 30)
}

/// Compute cost with full token usage including cache.
pub fn compute_cost(model: &str, usage: &TokenUsage) -> Option<CostResult> {
    let route = DEFAULT_PRICING.resolve_route(model, None);
    DEFAULT_PRICING.cost_for_turn(&route.provider, &route.model, usage)
}

/// Shorthand: compute cost with token counts.
pub fn compute_cost_simple(model: &str, input: usize, output: usize) -> Option<CostResult> {
    let route = DEFAULT_PRICING.resolve_route(model, None);
    let usage = TokenUsage {
        input_tokens: input,
        output_tokens: output,
        ..Default::default()
    };
    DEFAULT_PRICING.cost_for_turn(&route.provider, &route.model, &usage)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_claude_sonnet_cost() {
        let store = PricingStore::new();
        let result = store.cost_for_turn("anthropic", "claude-sonnet-4-20250514", &TokenUsage {
            input_tokens: 1000,
            output_tokens: 500,
            ..Default::default()
        });
        assert!(result.is_some());
        let result = result.unwrap();
        assert_eq!(result.status, CostStatus::Known);
        // 1000 * 3/1M + 500 * 15/1M = 0.003 + 0.0075 = 0.0105
        assert!((result.amount_usd.unwrap() - 0.0105).abs() < 0.0001);
    }

    #[test]
    fn test_gpt4o_cost() {
        let store = PricingStore::new();
        let result = store.cost_for_turn("openai", "gpt-4o", &TokenUsage {
            input_tokens: 1000000,
            output_tokens: 1000000,
            ..Default::default()
        });
        assert!(result.is_some());
        // 1M * 2.5/1M + 1M * 10/1M = 2.5 + 10.0 = 12.5
        assert!(result.unwrap().amount_usd.unwrap() == 12.5);
    }

    #[test]
    fn test_cache_write_cost() {
        let store = PricingStore::new();
        let result = store.cost_for_turn("anthropic", "claude-sonnet-4-20250514", &TokenUsage {
            input_tokens: 1000,
            output_tokens: 500,
            cache_write_tokens: 500000,
            ..Default::default()
        });
        // cache_write for sonnet-4-20250514 is 3.75/M
        // 1000*3/1M + 500*15/1M + 500000*3.75/1M = 0.003 + 0.0075 + 1.875 = 1.8855
        assert!(result.is_some());
        let cost = result.unwrap().amount_usd.unwrap();
        assert!((cost - 1.8855).abs() < 0.01);
    }

    #[test]
    fn test_cache_read_cost() {
        let store = PricingStore::new();
        let result = store.cost_for_turn("openai", "gpt-4o", &TokenUsage {
            input_tokens: 1000,
            output_tokens: 500,
            cache_read_tokens: 1000000,
            ..Default::default()
        });
        // cache_read for gpt-4o is 1.25. cache_write is none
        // 1M * 1.25/1M + 1000*2.5/1M + 500*10/1M = 1.25 + 0.0025 + 0.005 = 1.2575
        assert!(result.is_some());
        let cost = result.unwrap().amount_usd.unwrap();
        assert!((cost - 1.2575).abs() < 0.01);
    }

    #[test]
    fn test_deepseek_cost() {
        let store = PricingStore::new();
        let result = store.cost_for_turn("deepseek", "deepseek-chat", &TokenUsage {
            input_tokens: 1000000,
            output_tokens: 1000000,
            ..Default::default()
        });
        assert!(result.is_some());
        // 1M * 0.14/1M + 1M * 0.28/1M = 0.42
        assert!((result.unwrap().amount_usd.unwrap() - 0.42).abs() < 0.001);
    }

    #[test]
    fn test_gemini_cost() {
        let store = PricingStore::new();
        let result = store.cost_for_turn("google", "gemini-2.5-flash", &TokenUsage {
            input_tokens: 1000000,
            output_tokens: 1000000,
            ..Default::default()
        });
        assert!(result.is_some());
        assert!((result.unwrap().amount_usd.unwrap() - 0.75).abs() < 0.001);
    }

    #[test]
    fn test_unknown_model() {
        let store = PricingStore::new();
        let result = store.cost_for_turn("unknown", "random-model", &TokenUsage::default());
        assert!(result.is_some());
        assert_eq!(result.unwrap().status, CostStatus::Unknown);
    }

    #[test]
    fn test_local_model() {
        let store = PricingStore::new();
        let result = store.cost_for_turn("local", "custom-model", &TokenUsage {
            input_tokens: 1000,
            output_tokens: 100,
            ..Default::default()
        });
        assert!(result.is_some());
        assert_eq!(result.unwrap().status, CostStatus::Included);
    }

    #[test]
    fn test_resolve_route_explicit_provider() {
        let store = PricingStore::new();
        assert_eq!(store.resolve_route("gpt-4o", Some("openai")).provider, "openai");
        assert_eq!(store.resolve_route("claude-3-5-sonnet", Some("anthropic")).provider, "anthropic");
        assert_eq!(store.resolve_route("anthropic/claude-sonnet-4", None).provider, "anthropic");
        assert_eq!(store.resolve_route("custom/model", Some("local")).provider, "local");
    }

    #[test]
    fn test_resolve_route_infer_from_name_gpt() {
        let store = PricingStore::new();
        assert_eq!(store.resolve_route("gpt-4o", None).provider, "openai");
        assert_eq!(store.resolve_route("o3-mini", None).provider, "openai");
        assert_eq!(store.resolve_route("gpt-4.1-mini", None).provider, "openai");
    }

    #[test]
    fn test_resolve_route_infer_from_name_claude() {
        let store = PricingStore::new();
        assert_eq!(store.resolve_route("claude-3-5-sonnet-20241022", None).provider, "anthropic");
        assert_eq!(store.resolve_route("claude-sonnet-4-6", None).provider, "anthropic");
    }

    #[test]
    fn test_resolve_route_infer_from_name_gemini() {
        let store = PricingStore::new();
        assert_eq!(store.resolve_route("gemini-2.5-pro", None).provider, "google");
        assert_eq!(store.resolve_route("gemini-2.0-flash", None).provider, "google");
    }

    #[test]
    fn test_resolve_route_infer_from_name_deepseek() {
        let store = PricingStore::new();
        assert_eq!(store.resolve_route("deepseek-chat", None).provider, "deepseek");
        assert_eq!(store.resolve_route("deepseek-reasoner", None).provider, "deepseek");
    }

    #[test]
    fn test_resolve_route_unknown() {
        let store = PricingStore::new();
        assert_eq!(store.resolve_route("random-model", None).provider, "unknown");
    }

    #[test]
    fn test_provider_display_name() {
        let store = PricingStore::new();
        assert_eq!(store.provider_display_name("anthropic"), "Anthropic");
        assert_eq!(store.provider_display_name("openai"), "OpenAI");
        assert_eq!(store.provider_display_name("deepseek"), "DeepSeek");
        assert_eq!(store.provider_display_name("google"), "Google");
        assert_eq!(store.provider_display_name("bedrock"), "AWS Bedrock");
    }

    #[test]
    fn test_shorthand_compute_cost() {
        let result = compute_cost_simple("anthropic/claude-3-5-sonnet-20241022", 500, 200);
        // 500*3/1M + 200*15/1M = 0.0015 + 0.003 = 0.0045
        assert!(result.is_some());
        assert!(result.unwrap().amount_usd.unwrap() < 0.01);
    }

    #[test]
    fn test_shorthand_compute_full_cost() {
        let result = compute_cost("gpt-4o", &TokenUsage {
            input_tokens: 100,
            output_tokens: 50,
            ..Default::default()
        });
        // 100*2.5/1M + 50*10/1M = 0.00025 + 0.0005 = 0.00075
        assert!(result.is_some());
        assert!((result.unwrap().amount_usd.unwrap() - 0.00075).abs() < 0.001);
    }

    #[test]
    fn test_shorthand_has_pricing() {
        assert!(has_pricing("gpt-4o"));
        assert!(has_pricing("anthropic/claude-3-5-sonnet-20241022"));
        assert!(has_pricing("claude-3-5-sonnet-20241022"));
        assert!(has_pricing("gemini-2.5-pro"));
        assert!(has_pricing("deepseek-chat"));
        assert!(!has_pricing("nonexistent/model"));
    }

    #[test]
    fn test_cost_line_formatting() {
        let result = Some(CostResult {
            amount_usd: Some(12.5),
            status: CostStatus::Known,
            label: "$12.50".to_string(),
        });
        let line = format_cost_line("GPT-4o / 1M input + 1M output", &result.unwrap());
        assert!(line.contains("12.5"));
    }

    #[test]
    fn test_token_usage_helpers() {
        let usage = TokenUsage {
            input_tokens: 1000,
            output_tokens: 500,
            cache_read_tokens: 200,
            cache_write_tokens: 300,
        };
        assert_eq!(usage.prompt_tokens(), 1500);
        assert_eq!(usage.total_tokens(), 2000);
    }

    #[test]
    fn test_store_add_pricing() {
        let mut store = PricingStore::new();
        store.add_pricing("test", "model1", Some(0.5), Some(1.0), None, None);
        let entry = store.get_pricing("test", "model1");
        assert!(entry.is_some());
        assert_eq!(entry.unwrap().input_cost_per_million, Some(0.5));
    }

    #[test]
    fn test_cost_zero_tokens() {
        let store = PricingStore::new();
        let result = store.cost_for_turn("openai", "gpt-4o", &TokenUsage::default());
        assert!(result.is_some());
        assert!((result.unwrap().amount_usd.unwrap() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_per_request_cost() {
        let mut store = PricingStore::new();
        // OpenRouter charges per request + tokens
        store.add_per_request("openrouter", "gpt-4o", 0.01);
        let result = store.cost_for_turn("openrouter", "gpt-4o", &TokenUsage {
            input_tokens: 1000,
            output_tokens: 1000,
            ..Default::default()
        });
        assert!(result.is_some());
        let cost = result.unwrap().amount_usd.unwrap();
        // 0.01 (request) + no other costs since openrouter route skips token pricing
        assert!(cost > 0.009 && cost < 0.011);
    }

    #[test]
    fn test_openrouter_inferred_cost() {
        let store = PricingStore::new();
        // openrouter model should return request cost or unknown
        let result = store.cost_for_turn("openrouter", "gpt-4o", &TokenUsage {
            input_tokens: 100,
            output_tokens: 50,
            ..Default::default()
        });
        // No pricing entry for openrouter/gpt-4o in store, so should return Unknown
        assert!(result.is_some());
        assert_eq!(result.unwrap().status, CostStatus::Unknown);
    }

    #[test]
    fn test_large_token_count() {
        let store = PricingStore::new();
        let result = store.cost_for_turn("anthropic", "claude-opus-4-20250514", &TokenUsage {
            input_tokens: 1000000,
            output_tokens: 1000000,
            ..Default::default()
        });
        assert!(result.is_some());
        // 1M * 15/1M + 1M * 75/1M = 90.0
        assert!(result.unwrap().amount_usd.unwrap() == 90.0);
    }

    #[test]
    fn test_unknown_pricing() {
        let store = PricingStore::new();
        let result = store.cost_for_turn("unknown_provider", "nobody-knows", &TokenUsage {
            input_tokens: 1000,
            output_tokens: 100,
            ..Default::default()
        });
        match result {
            Some(CostResult { status: CostStatus::Unknown, .. }) => {},
            other => panic!("Expected CostStatus::Unknown, got {:?}", other),
        }
    }

    #[test]
    fn test_infer_provider_from_model_name() {
        assert_eq!(PricingStore::infer_provider_from_model("claude-3-opus"), "anthropic");
        assert_eq!(PricingStore::infer_provider_from_model("claude-sonnet-4-6"), "anthropic");
        assert_eq!(PricingStore::infer_provider_from_model("gpt-4o"), "openai");
        assert_eq!(PricingStore::infer_provider_from_model("o3-mini"), "openai");
        assert_eq!(PricingStore::infer_provider_from_model("gemini-2.5-pro"), "google");
        assert_eq!(PricingStore::infer_provider_from_model("deepseek-chat"), "deepseek");
        assert_eq!(PricingStore::infer_provider_from_model("nova-pro"), "bedrock");
        assert_eq!(PricingStore::infer_provider_from_model("minimax-m2"), "minimax");
        assert_eq!(PricingStore::infer_provider_from_model("unknown-model"), "");
    }

    #[test]
    fn test_opus_4_7_cost() {
        let store = PricingStore::new();
        let result = store.cost_for_turn("anthropic", "claude-opus-4-7", &TokenUsage {
            input_tokens: 100000,
            output_tokens: 100000,
            cache_write_tokens: 500000,
            ..Default::default()
        });
        assert!(result.is_some());
        // 100K * 5/1M + 100K * 25/1M + 500K * 6.25/1M
        // = 0.5 + 2.5 + 3.125 = 6.125
        let cost = result.unwrap().amount_usd.unwrap();
        assert!((cost - 6.125).abs() < 0.01);
    }

    #[test]
    fn test_custom_provider_included() {
        let store = PricingStore::new();
        let result = store.cost_for_turn("custom", "my-model", &TokenUsage {
            input_tokens: 1000,
            output_tokens: 100,
            ..Default::default()
        });
        assert!(result.is_some());
        assert_eq!(result.unwrap().status, CostStatus::Included);
    }
}
