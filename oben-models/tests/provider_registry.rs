use oben_models::provider_registry::{resolve_provider_info, TransportType};

// ─── Alias resolution ────────────────────────────────────────────────

#[test]
fn claude_alias_resolves_to_anthropic() {
    /// given: the "claude" alias
    /// when: resolve_provider_info("claude") is called
    /// then: canonical is "anthropic" and transport is AnthropicMessages
    let info = resolve_provider_info("claude").expect("claude should resolve");
    assert_eq!(info.canonical, "anthropic");
    assert_eq!(info.transport_type, TransportType::AnthropicMessages);
}

#[test]
fn claude_code_alias_resolves_to_anthropic() {
    /// given: the "claude_code" alias
    /// when: resolve_provider_info("claude_code") is called
    /// then: canonical is "anthropic"
    let info = resolve_provider_info("claude_code").expect("claude_code should resolve");
    assert_eq!(info.canonical, "anthropic");
    assert_eq!(info.transport_type, TransportType::AnthropicMessages);
}

#[test]
fn gpt_alias_resolves_to_openai() {
    /// given: the "gpt" alias
    /// when: resolve_provider_info("gpt") is called
    /// then: canonical is "openai" and transport is OpenAIChat
    let info = resolve_provider_info("gpt").expect("gpt should resolve");
    assert_eq!(info.canonical, "openai");
    assert_eq!(info.transport_type, TransportType::OpenAIChat);
}

#[test]
fn openai_alias_resolves_to_openai() {
    /// given: the bare "openai" name
    /// when: resolve_provider_info("openai") is called
    /// then: canonical is "openai"
    let info = resolve_provider_info("openai").expect("openai should resolve");
    assert_eq!(info.canonical, "openai");
}

#[test]
fn unknown_provider_returns_none() {
    /// given: an unknown provider string
    /// when: resolve_provider_info("nonexistent-provider-xyz") is called
    /// then: returns None
    let result = resolve_provider_info("nonexistent-provider-xyz");
    assert!(result.is_none());
}

// ─── Zai/智谱 aliases ────────────────────────────────────────────────

#[test]
fn zai_glm_alias_resolves() {
    /// given: "glm" alias
    /// when: resolve_provider_info("glm") is called
    /// then: resolves to zai
    let info = resolve_provider_info("glm").expect("glm should resolve");
    assert_eq!(info.canonical, "zai");
}

#[test]
fn zai_zhipu_alias_resolves() {
    /// given: "zhipu" alias
    /// when: resolve_provider_info("zhipu") is called
    /// then: resolves to zai
    let info = resolve_provider_info("zhipu").expect("zhipu should resolve");
    assert_eq!(info.canonical, "zai");
}

// ─── XAI/Grok aliases ────────────────────────────────────────────────

#[test]
fn xai_grok_alias_resolves() {
    /// given: "grok" alias
    /// when: resolve_provider_info("grok") is called
    /// then: resolves to xai
    let info = resolve_provider_info("grok").expect("grok should resolve");
    assert_eq!(info.canonical, "xai");
}

#[test]
fn xai_xai_alias_resolves() {
    /// given: "x-ai" alias
    /// when: resolve_provider_info("x-ai") is called
    /// then: resolves to xai
    let info = resolve_provider_info("x-ai").expect("x-ai should resolve");
    assert_eq!(info.canonical, "xai");
}

// ─── Kimi/Moonshot aliases ──────────────────────────────────────────

#[test]
fn kimi_moonshot_alias_resolves() {
    /// given: "moonshot" alias
    /// when: resolve_provider_info("moonshot") is called
    /// then: resolves to kimi-for-coding
    let info = resolve_provider_info("moonshot").expect("moonshot should resolve");
    assert_eq!(info.canonical, "kimi-for-coding");
}

#[test]
fn kimi_kimi_alias_resolves() {
    /// given: "kimi" alias
    /// when: resolve_provider_info("kimi") is called
    /// then: resolves to kimi-for-coding
    let info = resolve_provider_info("kimi").expect("kimi should resolve");
    assert_eq!(info.canonical, "kimi-for-coding");
}

// ─── Alibaba/Qwen aliases ───────────────────────────────────────────

#[test]
fn alibaba_qwen_alias_resolves() {
    /// given: "qwen" alias
    /// when: resolve_provider_info("qwen") is called
    /// then: resolves to alibaba
    let info = resolve_provider_info("qwen").expect("qwen should resolve");
    assert_eq!(info.canonical, "alibaba");
}

#[test]
fn alibaba_dashscope_alias_resolves() {
    /// given: "dashscope" alias
    /// when: resolve_provider_info("dashscope") is called
    /// then: resolves to alibaba
    let info = resolve_provider_info("dashscope").expect("dashscope should resolve");
    assert_eq!(info.canonical, "alibaba");
}

#[test]
fn alibaba_aliyun_alias_resolves() {
    /// given: "aliyun" alias
    /// when: resolve_provider_info("aliyun") is called
    /// then: resolves to alibaba
    let info = resolve_provider_info("aliyun").expect("aliyun should resolve");
    assert_eq!(info.canonical, "alibaba");
}

// ─── Case-insensitive resolution ────────────────────────────────────

#[test]
fn case_insensitive_claude() {
    /// given: uppercase "CLAUDE"
    /// when: resolve_provider_info("CLAUDE") is called
    /// then: resolves to anthropic
    let info = resolve_provider_info("CLAUDE").expect("CLAUDE should resolve");
    assert_eq!(info.canonical, "anthropic");
}

#[test]
fn case_insensitive_mixed() {
    /// given: mixed case "Claude-Code"
    /// when: resolve_provider_info("Claude-Code") is called
    /// then: resolves to anthropic
    let info = resolve_provider_info("Claude-Code").expect("Claude-Code should resolve");
    assert_eq!(info.canonical, "anthropic");
}

// ─── Base URL resolution ────────────────────────────────────────────

#[test]
fn anthropic_base_url() {
    /// given: "anthropic" canonical name
    /// when: resolve_provider_info("anthropic") is called
    /// then: base_url is "https://api.anthropic.com/v1"
    let info = resolve_provider_info("anthropic").expect("anthropic should resolve");
    assert_eq!(info.base_url, "https://api.anthropic.com/v1");
}

#[test]
fn openrouter_base_url() {
    /// given: "openrouter" canonical name
    /// when: resolve_provider_info("openrouter") is called
    /// then: base_url is set
    let info = resolve_provider_info("openrouter").expect("openrouter should resolve");
    assert_eq!(info.base_url, "https://openrouter.ai/api/v1");
}

#[test]
fn lmstudio_base_url() {
    /// given: "lmstudio" canonical name
    /// when: resolve_provider_info("lmstudio") is called
    /// then: base_url is "http://127.0.0.1:1234/v1"
    let info = resolve_provider_info("lmstudio").expect("lmstudio should resolve");
    assert_eq!(info.base_url, "http://127.0.0.1:1234/v1");
}

// ─── Edge cases ─────────────────────────────────────────────────────

#[test]
fn empty_string_returns_none() {
    /// given: empty string
    /// when: resolve_provider_info("") is called
    /// then: returns None
    let result = resolve_provider_info("");
    assert!(result.is_none());
}

#[test]
fn whitespace_only_returns_none() {
    /// given: whitespace-only string
    /// when: resolve_provider_info("   ") is called
    /// then: returns None
    let result = resolve_provider_info("   ");
    assert!(result.is_none());
}

#[test]
fn whitespace_padded_is_trimmed() {
    /// given: " claude " with spaces
    /// when: resolve_provider_info(" claude ") is called
    /// then: resolves to anthropic
    let info = resolve_provider_info(" claude ").expect("whitespace-padded should resolve");
    assert_eq!(info.canonical, "anthropic");
}

// ─── Provider-specific metadata tests ──────────────────────────────────

#[test]
fn all_non_empty_providers_have_transport() {
    /// given: all canonical provider IDs used by RESOLVED_ALIASES
    /// when: we check each one has valid transport
    /// then: no panic, valid transport for each
    let canonicals = [
        "anthropic",
        "openai",
        "openrouter",
        "google-gemini-cli",
        "zai",
        "kimi-for-coding",
        "deepseek",
        "alibaba",
        "alibaba-coding-plan",
        "stepfun",
        "minimax",
        "minimax-oauth",
        "minimax-cn",
        "tencent-tokenhub",
        "xai",
        "xai-oauth",
        "nvidia",
        "bedrock",
        "lmstudio",
        "nous",
        "vercel",
        "opencode",
        "opencode-go",
        "kilo",
        "huggingface",
        "novita",
        "xiaomi",
        "arcee",
        "gmi",
        "ollama-custom",
        "local",
    ];
    for id in canonicals {
        let info = oben_models::provider_registry::resolve_provider_info(id);
        assert!(info.is_some(), "Canonical '{}' should resolve", id);
        assert_eq!(info.unwrap().canonical, id);
    }
}

#[test]
fn anthropic_has_very_long_alias() {
    /// given: "claude-code-here-is-a-really-long-name-that-probably-does-not-exist"
    /// when: resolve_provider_info is called
    /// then: returns None (not found)
    let result = resolve_provider_info(
        "claude-code-here-is-a-really-long-name-that-probably-does-not-exist",
    );
    assert!(result.is_none());
}

#[test]
fn gemini_resolve() {
    /// given: "gemini-cli" alias
    /// when: resolve_provider_info is called
    /// then: resolves to google-gemini (native API key auth)
    let info = resolve_provider_info("gemini-cli");
    assert!(info.is_some());
    assert_eq!(info.clone().unwrap().canonical, "google-gemini");
}

#[test]
fn gemini_oauth_resolve() {
    /// given: "gemini-oauth" alias (OAuth/CloudCode variant)
    /// when: resolve_provider_info is called
    /// then: resolves to google-gemini-cli
    let info = resolve_provider_info("gemini-oauth");
    assert!(info.is_some());
    assert_eq!(info.unwrap().canonical, "google-gemini-cli");
}

#[test]
fn nous_resolve() {
    /// given: "nous" alias
    /// when: resolve_provider_info is called
    /// then: resolves to nous
    let info = resolve_provider_info("nous");
    assert!(info.is_some());
    assert_eq!(info.as_ref().unwrap().canonical, "nous");
    assert_eq!(
        info.as_ref().unwrap().base_url,
        "https://inference-api.nousresearch.com/v1"
    );
}

#[test]
fn minimax_oauth_resolve() {
    /// given: "minimax-oauth" canonical
    /// when: resolve_provider_info is called
    /// then: transport is AnthropicMessages
    let info = resolve_provider_info("minimax-oauth");
    assert!(info.is_some());
    assert_eq!(
        info.unwrap().transport_type,
        oben_models::provider_registry::TransportType::AnthropicMessages
    );
}

#[test]
fn bedrock_resolve() {
    /// given: "aws-bedrock" alias
    /// when: resolve_provider_info is called
    /// then: resolves to bedrock with BedrockConverse transport
    let info = resolve_provider_info("aws-bedrock");
    assert!(info.is_some());
    assert_eq!(
        info.unwrap().transport_type,
        oben_models::provider_registry::TransportType::BedrockConverse
    );
}

#[test]
fn vercel_resolve() {
    /// given: "ai-gateway" alias
    /// when: resolve_provider_info is called
    /// then: resolves to vercel
    let info = resolve_provider_info("ai-gateway");
    assert!(info.is_some());
    assert_eq!(info.unwrap().canonical, "vercel");
}

#[test]
fn opencode_resolve() {
    /// given: "zen" alias
    /// when: resolve_provider_info is called
    /// then: resolves to opencode
    let info = resolve_provider_info("zen");
    assert!(info.is_some());
    assert_eq!(info.unwrap().canonical, "opencode");
}

#[test]
fn kilo_resolve() {
    /// given: "kilo" canonical
    /// when: resolve_provider_info is called
    /// then: resolves to kilo
    let info = resolve_provider_info("kilo");
    assert!(info.is_some());
    assert_eq!(info.unwrap().canonical, "kilo");
}

#[test]
fn openai_transport_type() {
    /// given: TransportType::OpenAIChat
    /// when: matched against enum
    /// then: variant exists and can be used
    let tt = oben_models::provider_registry::TransportType::OpenAIChat;
    assert_eq!(tt.as_str(), "openai_chat");
}

#[test]
fn codex_responses_transport_type() {
    /// given: TransportType::CodexResponses
    /// when: matched against enum
    /// then: variant exists and can be used
    let tt = oben_models::provider_registry::TransportType::CodexResponses;
    assert_eq!(tt.as_str(), "codex_responses");
}

#[test]
fn tencent_tokenhub_resolve() {
    /// given: "tokenhub" alias
    /// when: resolve_provider_info is called
    /// then: resolves to tencent-tokenhub
    let info = resolve_provider_info("tokenhub");
    assert!(info.is_some());
    assert_eq!(info.unwrap().canonical, "tencent-tokenhub");
}

#[test]
fn minimax_cn_resolve() {
    /// given: "minimax-china" alias
    /// when: resolve_provider_info is called
    /// then: resolves to minimax-cn
    let info = resolve_provider_info("minimax-china");
    assert!(info.is_some());
    assert_eq!(info.unwrap().canonical, "minimax-cn");
}

#[test]
fn alibaba_coding_plan_resolve() {
    /// given: "alibaba-coding" alias
    /// when: resolve_provider_info is called
    /// then: resolves to alibaba-coding-plan
    let info = resolve_provider_info("alibaba-coding");
    assert!(info.is_some());
    assert_eq!(info.unwrap().canonical, "alibaba-coding-plan");
}

#[test]
fn xai_oauth_resolve() {
    /// given: "grok-oauth" alias
    /// when: resolve_provider_info is called
    /// then: resolves to xai-oauth
    let info = resolve_provider_info("grok-oauth");
    assert!(info.is_some());
    assert_eq!(info.unwrap().canonical, "xai-oauth");
}
