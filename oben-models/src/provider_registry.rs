use serde::{Deserialize, Serialize};

/// Transport protocol types supported by oben.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TransportType {
    OpenAIChat,
    AnthropicMessages,
    BedrockConverse,
    CodexResponses,
}

impl TransportType {
    pub fn as_str(&self) -> &str {
        match self {
            TransportType::OpenAIChat => "openai_chat",
            TransportType::AnthropicMessages => "anthropic_messages",
            TransportType::BedrockConverse => "bedrock_converse",
            TransportType::CodexResponses => "codex_responses",
        }
    }
}

/// Resolved provider information: canonical name, transport type, base URL.
#[derive(Debug, Clone)]
pub struct ProviderInfo<'a> {
    pub canonical: &'a str,
    pub transport_type: TransportType,
    pub base_url: &'a str,
}

/// Alias list: (human-friendly name, provider canonical id)
///
/// Maps to `HERMES_OVERLAYS` + `ALIASES` from `providers.py`.
pub const ALIAS_CANONICAL_PAIRS: &[(&str, &str)] = &[
    // Anthropic
    ("anthropic", "anthropic"),
    ("claude", "anthropic"),
    ("claude-code", "anthropic"),
    ("claude_code", "anthropic"),
    // OpenAI
    ("openai", "openai"),
    ("gpt", "openai"),
    // Gemini
    ("google-gemini-cli", "google-gemini-cli"),
    ("gemini", "google-gemini-cli"),
    ("gemini-cli", "google-gemini-cli"),
    ("gemini-oauth", "google-gemini-cli"),
    // OpenRouter
    ("openrouter", "openrouter"),
    // Nous Portal
    ("nous", "nous"),
    // Zai/智谱
    ("glm", "zai"),
    ("zai", "zai"),
    ("z-ai", "zai"),
    ("z.ai", "zai"),
    ("zhipu", "zai"),
    // XAI
    ("xai", "xai"),
    ("x-ai", "xai"),
    ("x.ai", "xai"),
    ("grok", "xai"),
    ("grok-oauth", "xai-oauth"),
    ("xai-oauth", "xai-oauth"),
    ("x-ai-oauth", "xai-oauth"),
    ("xai-grok-oauth", "xai-oauth"),
    // Kimi
    ("kimi-for-coding", "kimi-for-coding"),
    ("kimi", "kimi-for-coding"),
    ("kimi-coding", "kimi-for-coding"),
    ("kimi-coding-cn", "kimi-for-coding"),
    ("moonshot", "kimi-for-coding"),
    // DeepSeek
    ("deepseek", "deepseek"),
    ("deep-seek", "deepseek"),
    // Alibaba/Qwen
    ("qwen", "alibaba"),
    ("dashscope", "alibaba"),
    ("alibaba", "alibaba"),
    ("aliyun", "alibaba"),
    ("alibaba-coding", "alibaba-coding-plan"),
    ("alibaba_coding", "alibaba-coding-plan"),
    ("alibaba-coding-plan", "alibaba-coding-plan"),
    ("alibaba_coding_plan", "alibaba-coding-plan"),
    // StepFun
    ("stepfun", "stepfun"),
    ("step", "stepfun"),
    ("stepfun-coding-plan", "stepfun"),
    // MiniMax
    ("minimax", "minimax"),
    ("minimax-oauth", "minimax-oauth"),
    ("minimax-cn", "minimax-cn"),
    ("minimax-china", "minimax-cn"),
    ("minimax_cn", "minimax-cn"),
    // Tencent
    ("tencent-tokenhub", "tencent-tokenhub"),
    ("tencent", "tencent-tokenhub"),
    ("tokenhub", "tencent-tokenhub"),
    ("tencent-cloud", "tencent-tokenhub"),
    ("tencentmaas", "tencent-tokenhub"),
    // NVIDIA
    ("nvidia", "nvidia"),
    ("nim", "nvidia"),
    ("nvidia-nim", "nvidia"),
    ("build-nvidia", "nvidia"),
    ("nemotron", "nvidia"),
    // AWS Bedrock
    ("bedrock", "bedrock"),
    ("aws", "bedrock"),
    ("aws-bedrock", "bedrock"),
    ("amazon-bedrock", "bedrock"),
    ("amazon", "bedrock"),
    // LM Studio
    ("lmstudio", "lmstudio"),
    ("lm-studio", "lmstudio"),
    ("lm_studio", "lmstudio"),
    // Vercel AI Gateway
    ("vercel", "vercel"),
    ("ai-gateway", "vercel"),
    ("aigateway", "vercel"),
    ("vercel-ai-gateway", "vercel"),
    // OpenCode Zen
    ("opencode", "opencode"),
    ("opencode-zen", "opencode"),
    ("zen", "opencode"),
    // OpenCode Go
    ("opencode-go", "opencode-go"),
    ("go", "opencode-go"),
    ("opencode-go-sub", "opencode-go"),
    // KiloCode
    ("kilo", "kilo"),
    ("kilocode", "kilo"),
    ("kilo-code", "kilo"),
    ("kilo-gateway", "kilo"),
    // HuggingFace
    ("huggingface", "huggingface"),
    ("hf", "huggingface"),
    ("hugging-face", "huggingface"),
    ("huggingface-hub", "huggingface"),
    // NovitaAI
    ("novita", "novita"),
    ("novita-ai", "novita"),
    ("novitaai", "novita"),
    // Xiaomi
    ("xiaomi", "xiaomi"),
    ("mimo", "xiaomi"),
    ("xiaomi-mimo", "xiaomi"),
    // Arcee
    ("arcee", "arcee"),
    ("arcee-ai", "arcee"),
    ("arceeai", "arcee"),
    // GMI Cloud
    ("gmi", "gmi"),
    ("gmi-cloud", "gmi"),
    ("gmicloud", "gmi"),
    // Ollama Cloud
    ("ollama-custom", "ollama-custom"),
    ("ollama", "ollama-custom"),
    ("ollama-cloud", "ollama-custom"),
    // Local
    ("local", "local"),
    ("vllm", "local"),
    ("llamacpp", "local"),
    ("llama.cpp", "local"),
    ("llama-cpp", "local"),
];

/// Provider-specific metadata for known providers.
/// Maps to `HERMES_OVERLAYS` from `providers.py`.
pub(crate) const PROVIDER_META: &[(&str, TransportType, &'static str)] = &[
    ("anthropic", TransportType::AnthropicMessages, "https://api.anthropic.com/v1"),
    ("openai", TransportType::OpenAIChat, ""),
    ("openrouter", TransportType::OpenAIChat, "https://openrouter.ai/api/v1"),
    ("google-gemini-cli", TransportType::OpenAIChat, "cloudcode-pa://google"),
    ("zai", TransportType::OpenAIChat, "https://open.bigmodel.cn/api/paas/v4/"),
    ("kimi-for-coding", TransportType::OpenAIChat, "https://api.moonshot.cn/v1"),
    ("deepseek", TransportType::OpenAIChat, ""),
    ("alibaba", TransportType::OpenAIChat, "https://dashscope.aliyuncs.com/compatible-mode/v1"),
    ("alibaba-coding-plan", TransportType::OpenAIChat, ""),
    ("stepfun", TransportType::OpenAIChat, "https://api.stepfun.ai/step_plan/v1"),
    ("minimax", TransportType::AnthropicMessages, ""),
    ("minimax-oauth", TransportType::AnthropicMessages, "https://api.minimax.io/anthropic"),
    ("minimax-cn", TransportType::AnthropicMessages, ""),
    ("tencent-tokenhub", TransportType::OpenAIChat, ""),
    ("xai", TransportType::CodexResponses, ""),
    ("xai-oauth", TransportType::CodexResponses, ""),
    ("nvidia", TransportType::OpenAIChat, "https://integrate.api.nvidia.com/v1"),
    ("baidu", TransportType::OpenAIChat, "https://aip.baidubce.com/rpc/2.0/ai_custom/v1/"),
    ("lmstudio", TransportType::OpenAIChat, "http://127.0.0.1:1234/v1"),
    ("nous", TransportType::OpenAIChat, "https://inference-api.nousresearch.com/v1"),
    ("vercel", TransportType::OpenAIChat, ""),
    ("opencode", TransportType::OpenAIChat, ""),
    ("opencode-go", TransportType::OpenAIChat, ""),
    ("kilo", TransportType::OpenAIChat, ""),
    ("huggingface", TransportType::OpenAIChat, ""),
    ("novita", TransportType::OpenAIChat, ""),
    ("xiaomi", TransportType::OpenAIChat, ""),
    ("arcee", TransportType::OpenAIChat, "https://api.arcee.ai/api/v1"),
    ("gmi", TransportType::OpenAIChat, "https://api.gmi-serving.com/v1"),
    ("ollama-custom", TransportType::OpenAIChat, "https://ollama.com/v1"),
    ("local", TransportType::OpenAIChat, ""),
];

/// Canonical provider API key environment variable fallback chains.
///
/// Each entry maps a canonical provider name to an ordered list of env vars to
/// try.  The first non-empty value wins.  An empty slice means the provider has
/// no env-var fallback (OAuth-based, cloud-only, or local).
const PROVIDER_API_KEY_CHAINS: &[(&str, &'static [&'static str])] = &[
    ("openai",            &["OPENAI_API_KEY", "OPENAI_TOKEN"]),
    ("openrouter",        &["OPENROUTER_API_KEY"]),
    ("anthropic",         &["ANTHROPIC_API_KEY", "ANTHROPIC_TOKEN", "CLAUDE_CODE_OAUTH_TOKEN"]),
    ("bedrock",           &[]),
    ("google-gemini-cli", &["GEMINI_API_KEY", "GOOGLE_API_KEY"]),
    ("lmstudio",          &["LM_API_KEY"]),
    ("deepseek",          &["DEEPSEEK_API_KEY"]),
    ("alibaba",           &["DASHSCOPE_API_KEY"]),
    ("alibaba-coding-plan", &["ALIBABA_CODING_PLAN_API_KEY", "DASHSCOPE_API_KEY"]),
    ("stepfun",           &["STEPFUN_API_KEY"]),
    ("minimax",           &["MINIMAX_API_KEY"]),
    ("minimax-oauth",     &[]),
    ("minimax-cn",        &["MINIMAX_CN_API_KEY"]),
    ("tencent-tokenhub",  &["TOKENHUB_API_KEY"]),
    ("xai",               &["XAI_API_KEY"]),
    ("xai-oauth",         &[]),
    ("nvidia",            &["NVIDIA_API_KEY"]),
    ("nous",              &[]),
    ("vercel",            &["AI_GATEWAY_API_KEY"]),
    ("opencode",          &["OPENCODE_ZEN_API_KEY"]),
    ("opencode-go",       &["OPENCODE_GO_API_KEY"]),
    ("kilo",              &["KILOCODE_API_KEY"]),
    ("huggingface",       &["HF_TOKEN"]),
    ("novita",            &["NOVITA_API_KEY"]),
    ("xiaomi",            &["XIAOMI_API_KEY"]),
    ("arcee",             &["ARCEEAI_API_KEY"]),
    ("gmi",               &["GMI_API_KEY"]),
    ("ollama-custom",     &["OLLAMA_API_KEY"]),
    ("zai",               &["GLM_API_KEY", "ZAI_API_KEY"]),
    ("kimi",              &["MOONSHOT_API_KEY"]),
    ("baidu",             &["BAIDU_API_KEY"]),
    ("local",             &[]),
    ("custom",            &[]),
];

/// Provider base URL environment variable mapping.
///
/// Maps canonical provider names to the env var that may hold a custom base URL.
const PROVIDER_BASE_URL_ENV_VARS: &[(&str, &str)] = &[
    ("openai", "OPENAI_BASE_URL"),
    ("openrouter", "OPENROUTER_BASE_URL"),
    ("anthropic", "ANTHROPIC_BASE_URL"),
    ("bedrock", "BEDROCK_BASE_URL"),
    ("google-gemini-cli", "GEMINI_BASE_URL"),
    ("lmstudio", "LM_BASE_URL"),
    ("deepseek", "DEEPSEEK_BASE_URL"),
    ("alibaba", "DASHSCOPE_BASE_URL"),
    ("alibaba-coding-plan", "ALIBABA_CODING_PLAN_BASE_URL"),
    ("stepfun", "STEPFUN_BASE_URL"),
    ("minimax", "MINIMAX_BASE_URL"),
    ("minimax-cn", "MINIMAX_CN_BASE_URL"),
    ("tencent-tokenhub", "TOKENHUB_BASE_URL"),
    ("xai", "XAI_BASE_URL"),
    ("xai-oauth", "XAI_BASE_URL"),
    ("nvidia", "NVIDIA_BASE_URL"),
    ("vercel", "AI_GATEWAY_BASE_URL"),
    ("opencode", "OPENCODE_ZEN_BASE_URL"),
    ("opencode-go", "OPENCODE_GO_BASE_URL"),
    ("kilo", "KILOCODE_BASE_URL"),
    ("huggingface", "HF_BASE_URL"),
    ("novita", "NOVITA_BASE_URL"),
    ("xiaomi", "XIAOMI_BASE_URL"),
    ("arcee", "ARCEE_BASE_URL"),
    ("gmi", "GMI_BASE_URL"),
    ("ollama-custom", "OLLAMA_BASE_URL"),
    ("zai", "GLM_BASE_URL"),
    ("kimi", "KIMI_BASE_URL"),
    ("baidu", "BAIDU_BASE_URL"),
];

/// Look up the API key env var chain for a canonical provider name.
pub fn resolve_api_key_env_chain(canonical: &str) -> &'static [&'static str] {
    PROVIDER_API_KEY_CHAINS
        .iter()
        .find(|(id, _)| *id == canonical)
        .map(|(_, chain)| *chain)
        .unwrap_or(&[])
}

/// Resolve an API key for a canonical provider from environment variables.
///
/// Iterates the chain in priority order; returns the first non-empty value or
/// `None` if none are set or the provider has no env-var chain.
pub fn resolve_api_key_from_env(canonical: &str) -> Option<String> {
    for env_var in resolve_api_key_env_chain(canonical) {
        if let Ok(val) = std::env::var(env_var) {
            let val = val.trim().to_string();
            if !val.is_empty() {
                return Some(val);
            }
        }
    }
    None
}

/// Return the env var name for a provider's base URL, if configured.
pub fn resolve_base_url_env_var(canonical: &str) -> Option<&str> {
    PROVIDER_BASE_URL_ENV_VARS
        .iter()
        .find(|(id, _)| *id == canonical)
        .map(|(_, var_name)| *var_name)
}

/// Resolve a provider name (or alias) to its canonical provider info.
///
/// Resolution order:
/// 1. Case-insensitive, normalized lookup in alias map
/// 2. If found in alias, follow to canonical provider metadata
/// 3. Return full ProviderInfo or None
pub fn resolve_provider_info(name: &str) -> Option<ProviderInfo<'_>> {
    let normalized = name.trim().to_lowercase();

    // Find the canonical provider id via the alias map
    let canonical = ALIAS_CANONICAL_PAIRS
        .iter()
        .find(|(alias, _)| *alias == normalized)
        .map(|(_, canonical)| *canonical);

    // Look up metadata for the canonical provider
    canonical.and_then(|canonical| {
        PROVIDER_META
            .iter()
            .find(|(id, _, _)| *id == canonical)
            .map(|(id, transport, base_url)| ProviderInfo {
                canonical: id,
                transport_type: transport.clone(),
                base_url,
            })
    })
}

/// Map a ProviderKind enum value to the matching transport type.
///
/// This is the runtime dispatch equivalent to the old
/// `ProviderKind::Anthropic => "anthropic_messages"` match arm.
///
/// Note: this function returns `None` for `ProviderKind::Custom` since the
/// transport depends on the user-provided base URL.
pub fn provider_kind_to_transport(kind: crate::providers::ProviderKind) -> Option<TransportType> {
    Some(match kind {
        crate::providers::ProviderKind::Anthropic |
        crate::providers::ProviderKind::MiniMax |
        crate::providers::ProviderKind::MiniMaxOAuth |
        crate::providers::ProviderKind::MiniMaxCN => TransportType::AnthropicMessages,
        crate::providers::ProviderKind::Bedrock => TransportType::BedrockConverse,
        crate::providers::ProviderKind::XAI |
        crate::providers::ProviderKind::XAIOAuth => TransportType::CodexResponses,
        crate::providers::ProviderKind::OpenAI |
        crate::providers::ProviderKind::OpenRouter |
        crate::providers::ProviderKind::Gemini |
        crate::providers::ProviderKind::LMStudio |
        crate::providers::ProviderKind::Custom |
        crate::providers::ProviderKind::DeepSeek |
        crate::providers::ProviderKind::Alibaba |
        crate::providers::ProviderKind::AlibabaCodingPlan |
        crate::providers::ProviderKind::StepFun |
        crate::providers::ProviderKind::TencentTokenHub |
        crate::providers::ProviderKind::NVIDIA |
        crate::providers::ProviderKind::Nous |
        crate::providers::ProviderKind::Vercel |
        crate::providers::ProviderKind::OpenCode |
        crate::providers::ProviderKind::OpenCodeGo |
        crate::providers::ProviderKind::Kilo |
        crate::providers::ProviderKind::HuggingFace |
        crate::providers::ProviderKind::Novita |
        crate::providers::ProviderKind::Xiaomi |
        crate::providers::ProviderKind::Arcee |
        crate::providers::ProviderKind::GMI |
        crate::providers::ProviderKind::OllamaCloud |
            crate::providers::ProviderKind::Local |
            crate::providers::ProviderKind::Zai |
            crate::providers::ProviderKind::Kimi |
            crate::providers::ProviderKind::Baidu => TransportType::OpenAIChat,
    })

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anthropic_transports_as_str() {
        assert_eq!(TransportType::AnthropicMessages.as_str(), "anthropic_messages");
        assert_eq!(TransportType::OpenAIChat.as_str(), "openai_chat");
        assert_eq!(TransportType::BedrockConverse.as_str(), "bedrock_converse");
        assert_eq!(TransportType::CodexResponses.as_str(), "codex_responses");
    }

    #[test]
    fn provider_kind_map_anthropic() {
        assert_eq!(
            provider_kind_to_transport(crate::providers::ProviderKind::Anthropic),
            Some(TransportType::AnthropicMessages)
        );
    }

    #[test]
    fn provider_kind_map_openai() {
        assert_eq!(
            provider_kind_to_transport(crate::providers::ProviderKind::OpenAI),
            Some(TransportType::OpenAIChat)
        );
    }

    #[test]
    fn provider_kind_map_bedrock() {
        assert_eq!(
            provider_kind_to_transport(crate::providers::ProviderKind::Bedrock),
            Some(TransportType::BedrockConverse)
        );
    }
}
