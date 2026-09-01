//! Declarative provider registry.
//!
//! One data table describes a provider's identity, models, and environment
//! wiring. Downstream consumers (static model fallbacks, context limits,
//! pricing lookup, env-key detection) derive their tables from this instead of
//! maintaining parallel hand-written match arms across crates.
//!
//! The registry entry is authoritative; legacy sites read from it. Parity tests
//! in jcode-base fail loudly if hand-written tables drift from the registry.

/// A single model a provider is known to serve, with the metadata jcode needs
/// before the provider's live `/v1/models` catalog is available.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProviderModelEntry {
    pub model_id: &'static str,
    pub context_window: usize,
}

/// Declarative metadata for one OpenAI-compatible provider.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProviderRegistryEntry {
    /// Canonical jcode profile id (matches `OpenAiCompatibleProfile::id`).
    pub id: &'static str,
    /// models.dev provider id used for pricing lookup.
    pub models_dev_id: &'static str,
    /// API-key environment variable that maps to this provider.
    pub api_key_env: &'static str,
    /// Static fallback models with their context windows.
    pub models: &'static [ProviderModelEntry],
}

// =========================================================================
// Per-provider model tables
// =========================================================================

pub const CEREBRAS_MODELS: &[ProviderModelEntry] = &[
    ProviderModelEntry {
        model_id: "gpt-oss-120b",
        context_window: 131_072,
    },
    ProviderModelEntry {
        model_id: "zai-glm-4.7",
        context_window: 200_000,
    },
];

pub const FIREWORKS_MODELS: &[ProviderModelEntry] = &[
    ProviderModelEntry {
        model_id: "accounts/fireworks/routers/kimi-k2p5-turbo",
        context_window: 262_144,
    },
    ProviderModelEntry {
        model_id: "accounts/fireworks/models/kimi-k2p5",
        context_window: 262_144,
    },
    ProviderModelEntry {
        model_id: "accounts/fireworks/models/kimi-k2p6",
        context_window: 262_144,
    },
    ProviderModelEntry {
        model_id: "accounts/fireworks/models/glm-4p7",
        context_window: 200_000,
    },
    ProviderModelEntry {
        model_id: "accounts/fireworks/models/glm-5p1",
        context_window: 200_000,
    },
    ProviderModelEntry {
        model_id: "accounts/fireworks/models/deepseek-v3p2",
        context_window: 163_840,
    },
];

pub const DEEPSEEK_MODELS: &[ProviderModelEntry] = &[
    ProviderModelEntry {
        model_id: "deepseek-v4-flash",
        context_window: 1_000_000,
    },
    ProviderModelEntry {
        model_id: "deepseek-v4-pro",
        context_window: 1_000_000,
    },
];

pub const ZAI_MODELS: &[ProviderModelEntry] = &[
    ProviderModelEntry {
        model_id: "glm-4.5",
        context_window: 128_000,
    },
    ProviderModelEntry {
        model_id: "glm-4.7",
        context_window: 200_000,
    },
    ProviderModelEntry {
        model_id: "glm-5",
        context_window: 200_000,
    },
    ProviderModelEntry {
        model_id: "glm-5.1",
        context_window: 200_000,
    },
    ProviderModelEntry {
        model_id: "glm-4.7-flash",
        context_window: 200_000,
    },
    ProviderModelEntry {
        model_id: "glm-4.7-flashx",
        context_window: 200_000,
    },
];

pub const KIMI_MODELS: &[ProviderModelEntry] = &[
    ProviderModelEntry {
        model_id: "kimi-for-coding",
        context_window: 262_144,
    },
    ProviderModelEntry {
        model_id: "kimi-k2.5",
        context_window: 262_144,
    },
    ProviderModelEntry {
        model_id: "kimi-k2.6",
        context_window: 262_144,
    },
    ProviderModelEntry {
        model_id: "kimi-k2-thinking",
        context_window: 262_144,
    },
    ProviderModelEntry {
        model_id: "kimi-k2-thinking-turbo",
        context_window: 262_144,
    },
];

pub const MINIMAX_MODELS: &[ProviderModelEntry] = &[
    ProviderModelEntry {
        model_id: "MiniMax-M2.7",
        context_window: 204_800,
    },
    ProviderModelEntry {
        model_id: "MiniMax-M2.7-highspeed",
        context_window: 204_800,
    },
    ProviderModelEntry {
        model_id: "MiniMax-M2.5",
        context_window: 204_800,
    },
    ProviderModelEntry {
        model_id: "MiniMax-M2.5-highspeed",
        context_window: 204_800,
    },
    ProviderModelEntry {
        model_id: "MiniMax-M2.1",
        context_window: 204_800,
    },
    ProviderModelEntry {
        model_id: "MiniMax-M2.1-highspeed",
        context_window: 204_800,
    },
    ProviderModelEntry {
        model_id: "MiniMax-M2",
        context_window: 204_800,
    },
];

pub const PERPLEXITY_MODELS: &[ProviderModelEntry] = &[
    ProviderModelEntry {
        model_id: "sonar",
        context_window: 128_000,
    },
    ProviderModelEntry {
        model_id: "sonar-pro",
        context_window: 128_000,
    },
    ProviderModelEntry {
        model_id: "sonar-reasoning-pro",
        context_window: 128_000,
    },
    ProviderModelEntry {
        model_id: "sonar-deep-research",
        context_window: 128_000,
    },
];

pub const DEEPINFRA_MODELS: &[ProviderModelEntry] = &[
    ProviderModelEntry {
        model_id: "moonshotai/Kimi-K2-Instruct",
        context_window: 262_144,
    },
    ProviderModelEntry {
        model_id: "Qwen/Qwen3-Coder-480B-A35B-Instruct",
        context_window: 262_144,
    },
    ProviderModelEntry {
        model_id: "Qwen/Qwen3-Coder-480B-A35B-Instruct-Turbo",
        context_window: 262_144,
    },
    ProviderModelEntry {
        model_id: "zai-org/GLM-4.7",
        context_window: 200_000,
    },
    ProviderModelEntry {
        model_id: "zai-org/GLM-5.1",
        context_window: 200_000,
    },
    ProviderModelEntry {
        model_id: "meta-llama/Llama-3.1-70B-Instruct",
        context_window: 131_072,
    },
];

pub const MOONSHOTAI_MODELS: &[ProviderModelEntry] = &[
    ProviderModelEntry {
        model_id: "kimi-k2.5",
        context_window: 262_144,
    },
    ProviderModelEntry {
        model_id: "kimi-k2.6",
        context_window: 262_144,
    },
    ProviderModelEntry {
        model_id: "kimi-k2-thinking",
        context_window: 262_144,
    },
    ProviderModelEntry {
        model_id: "kimi-k2-thinking-turbo",
        context_window: 262_144,
    },
    ProviderModelEntry {
        model_id: "kimi-k2-turbo-preview",
        context_window: 262_144,
    },
];

pub const BASETEN_MODELS: &[ProviderModelEntry] = &[
    ProviderModelEntry {
        model_id: "zai-org/GLM-4.7",
        context_window: 200_000,
    },
    ProviderModelEntry {
        model_id: "zai-org/GLM-5",
        context_window: 200_000,
    },
    ProviderModelEntry {
        model_id: "openai/gpt-oss-120b",
        context_window: 131_072,
    },
    ProviderModelEntry {
        model_id: "moonshotai/Kimi-K2.6",
        context_window: 262_144,
    },
    ProviderModelEntry {
        model_id: "moonshotai/Kimi-K2.5",
        context_window: 262_144,
    },
    ProviderModelEntry {
        model_id: "deepseek-ai/DeepSeek-V4-Pro",
        context_window: 1_000_000,
    },
];

pub const AI302_MODELS: &[ProviderModelEntry] = &[
    ProviderModelEntry {
        model_id: "qwen3-235b-a22b-instruct-2507",
        context_window: 262_144,
    },
    ProviderModelEntry {
        model_id: "glm-4.7",
        context_window: 200_000,
    },
    ProviderModelEntry {
        model_id: "glm-5.1",
        context_window: 200_000,
    },
    ProviderModelEntry {
        model_id: "MiniMax-M2",
        context_window: 204_800,
    },
    ProviderModelEntry {
        model_id: "kimi-k2-0905-preview",
        context_window: 262_144,
    },
    ProviderModelEntry {
        model_id: "claude-haiku-4-5",
        context_window: 200_000,
    },
];

pub const CORTECS_MODELS: &[ProviderModelEntry] = &[
    ProviderModelEntry {
        model_id: "minimax-m2.7",
        context_window: 204_800,
    },
    ProviderModelEntry {
        model_id: "kimi-k2.5",
        context_window: 262_144,
    },
    ProviderModelEntry {
        model_id: "glm-4.7",
        context_window: 200_000,
    },
    ProviderModelEntry {
        model_id: "glm-5",
        context_window: 200_000,
    },
    ProviderModelEntry {
        model_id: "claude-haiku-4-5",
        context_window: 200_000,
    },
    ProviderModelEntry {
        model_id: "qwen3-235b-a22b-instruct-2507",
        context_window: 262_144,
    },
];

pub const COMTEGRA_MODELS: &[ProviderModelEntry] = &[
    ProviderModelEntry {
        model_id: "gpt-oss-120b",
        context_window: 131_072,
    },
    ProviderModelEntry {
        model_id: "qwen35-122b",
        context_window: 262_144,
    },
    ProviderModelEntry {
        model_id: "gte-qwen2-7b",
        context_window: 32_768,
    },
    ProviderModelEntry {
        model_id: "glm-51-nvfp4",
        context_window: 200_000,
    },
];

pub const FPT_MODELS: &[ProviderModelEntry] = &[
    ProviderModelEntry {
        model_id: "GLM-5.1",
        context_window: 200_000,
    },
    ProviderModelEntry {
        model_id: "GLM-4.7",
        context_window: 200_000,
    },
    ProviderModelEntry {
        model_id: "Llama-3.3-70B-Instruct",
        context_window: 131_072,
    },
];

pub const FIRMWARE_MODELS: &[ProviderModelEntry] = &[
    ProviderModelEntry {
        model_id: "kimi-k2.5",
        context_window: 262_144,
    },
    ProviderModelEntry {
        model_id: "zai-glm-5-1",
        context_window: 200_000,
    },
    ProviderModelEntry {
        model_id: "claude-haiku-4-5",
        context_window: 200_000,
    },
    ProviderModelEntry {
        model_id: "claude-sonnet-4-6",
        context_window: 200_000,
    },
    ProviderModelEntry {
        model_id: "grok-code-fast-1",
        context_window: 256_000,
    },
    ProviderModelEntry {
        model_id: "gemini-2.5-flash",
        context_window: 1_000_000,
    },
];

pub const HUGGINGFACE_MODELS: &[ProviderModelEntry] = &[
    ProviderModelEntry {
        model_id: "Qwen/Qwen3-Coder-480B-A35B-Instruct",
        context_window: 262_144,
    },
    ProviderModelEntry {
        model_id: "Qwen/Qwen3-Coder-Next",
        context_window: 262_144,
    },
    ProviderModelEntry {
        model_id: "zai-org/GLM-4.7",
        context_window: 200_000,
    },
    ProviderModelEntry {
        model_id: "zai-org/GLM-5.1",
        context_window: 200_000,
    },
    ProviderModelEntry {
        model_id: "deepseek-ai/DeepSeek-V3.2",
        context_window: 163_840,
    },
    ProviderModelEntry {
        model_id: "openai/gpt-oss-120b",
        context_window: 131_072,
    },
];

pub const NEBIUS_MODELS: &[ProviderModelEntry] = &[
    ProviderModelEntry {
        model_id: "openai/gpt-oss-120b",
        context_window: 131_072,
    },
    ProviderModelEntry {
        model_id: "Qwen/Qwen3-235B-A22B-Instruct-2507",
        context_window: 262_144,
    },
    ProviderModelEntry {
        model_id: "Qwen/Qwen3.5-397B-A17B",
        context_window: 262_144,
    },
    ProviderModelEntry {
        model_id: "zai-org/GLM-5",
        context_window: 200_000,
    },
    ProviderModelEntry {
        model_id: "meta-llama/Llama-3.3-70B-Instruct",
        context_window: 131_072,
    },
    ProviderModelEntry {
        model_id: "NousResearch/Hermes-4-70B",
        context_window: 131_072,
    },
];

pub const SCALEWAY_MODELS: &[ProviderModelEntry] = &[
    ProviderModelEntry {
        model_id: "qwen3-coder-30b-a3b-instruct",
        context_window: 262_144,
    },
    ProviderModelEntry {
        model_id: "qwen3-235b-a22b-instruct-2507",
        context_window: 262_144,
    },
    ProviderModelEntry {
        model_id: "qwen3.5-397b-a17b",
        context_window: 262_144,
    },
    ProviderModelEntry {
        model_id: "gpt-oss-120b",
        context_window: 131_072,
    },
    ProviderModelEntry {
        model_id: "mistral-small-3.2-24b-instruct-2506",
        context_window: 131_072,
    },
    ProviderModelEntry {
        model_id: "llama-3.3-70b-instruct",
        context_window: 131_072,
    },
];

pub const STACKIT_MODELS: &[ProviderModelEntry] = &[
    ProviderModelEntry {
        model_id: "openai/gpt-oss-120b",
        context_window: 131_072,
    },
    ProviderModelEntry {
        model_id: "Qwen/Qwen3-VL-235B-A22B-Instruct-FP8",
        context_window: 262_144,
    },
    ProviderModelEntry {
        model_id: "cortecs/Llama-3.3-70B-Instruct-FP8-Dynamic",
        context_window: 131_072,
    },
    ProviderModelEntry {
        model_id: "neuralmagic/Meta-Llama-3.1-8B-Instruct-FP8",
        context_window: 131_072,
    },
    ProviderModelEntry {
        model_id: "google/gemma-3-27b-it",
        context_window: 131_072,
    },
];

pub const CELERIS_MODELS: &[ProviderModelEntry] = &[ProviderModelEntry {
    model_id: "celeris-1",
    context_window: 131_072,
}];

pub const XIAOMI_MIMO_MODELS: &[ProviderModelEntry] = &[
    ProviderModelEntry {
        model_id: "mimo-v2.5",
        context_window: 262_144,
    },
    ProviderModelEntry {
        model_id: "mimo-v2.5-pro",
        context_window: 262_144,
    },
    ProviderModelEntry {
        model_id: "mimo-v2-pro",
        context_window: 262_144,
    },
    ProviderModelEntry {
        model_id: "mimo-v2-flash",
        context_window: 262_144,
    },
    ProviderModelEntry {
        model_id: "mimo-v2-omni",
        context_window: 262_144,
    },
];

pub const META_MUSE_MODELS: &[ProviderModelEntry] = &[
    ProviderModelEntry {
        model_id: "muse-spark-1.2",
        context_window: 1_048_576,
    },
    ProviderModelEntry {
        model_id: "muse-spark-1.1",
        context_window: 1_048_576,
    },
];

pub const ALIBABA_CODING_PLAN_MODELS: &[ProviderModelEntry] = &[
    ProviderModelEntry {
        model_id: "qwen3-coder-plus",
        context_window: 262_144,
    },
    ProviderModelEntry {
        model_id: "qwen3.5-plus",
        context_window: 262_144,
    },
    ProviderModelEntry {
        model_id: "qwen3-max-2026-01-23",
        context_window: 262_144,
    },
    ProviderModelEntry {
        model_id: "qwen3-coder-next",
        context_window: 262_144,
    },
    ProviderModelEntry {
        model_id: "glm-5",
        context_window: 200_000,
    },
    ProviderModelEntry {
        model_id: "glm-4.7",
        context_window: 200_000,
    },
    ProviderModelEntry {
        model_id: "kimi-k2.5",
        context_window: 262_144,
    },
    ProviderModelEntry {
        model_id: "MiniMax-M2.5",
        context_window: 204_800,
    },
];

pub const BELVEDIR_MODELS: &[ProviderModelEntry] = &[ProviderModelEntry {
    model_id: "auto",
    context_window: 131_072,
}];

pub const WAFER_MODELS: &[ProviderModelEntry] = &[
    ProviderModelEntry {
        model_id: "GLM-5.2",
        context_window: 1_048_576,
    },
    ProviderModelEntry {
        model_id: "GLM-5.3",
        context_window: 1_048_576,
    },
    ProviderModelEntry {
        model_id: "GLM-5.3-Flash",
        context_window: 1_048_576,
    },
    ProviderModelEntry {
        model_id: "Kimi-K3",
        context_window: 1_048_576,
    },
    ProviderModelEntry {
        model_id: "Kimi-K2.6",
        context_window: 262_144,
    },
    ProviderModelEntry {
        model_id: "DeepSeek-V4-Flash-0731-Fast",
        context_window: 1_048_576,
    },
    ProviderModelEntry {
        model_id: "Qwen3.5-397B-A17B",
        context_window: 262_144,
    },
];

// =========================================================================
// Complete provider registry table
// =========================================================================

pub const REGISTRY: &[ProviderRegistryEntry] = &[
    ProviderRegistryEntry {
        id: "cerebras",
        models_dev_id: "cerebras",
        api_key_env: "CEREBRAS_API_KEY",
        models: CEREBRAS_MODELS,
    },
    ProviderRegistryEntry {
        id: "fireworks",
        models_dev_id: "fireworks-ai",
        api_key_env: "FIREWORKS_API_KEY",
        models: FIREWORKS_MODELS,
    },
    ProviderRegistryEntry {
        id: "deepseek",
        models_dev_id: "deepseek",
        api_key_env: "DEEPSEEK_API_KEY",
        models: DEEPSEEK_MODELS,
    },
    ProviderRegistryEntry {
        id: "zai",
        models_dev_id: "zai",
        api_key_env: "ZHIPU_API_KEY",
        models: ZAI_MODELS,
    },
    ProviderRegistryEntry {
        id: "kimi",
        models_dev_id: "kimi-for-coding",
        api_key_env: "KIMI_API_KEY",
        models: KIMI_MODELS,
    },
    ProviderRegistryEntry {
        id: "minimax",
        models_dev_id: "minimax",
        api_key_env: "MINIMAX_API_KEY",
        models: MINIMAX_MODELS,
    },
    ProviderRegistryEntry {
        id: "perplexity",
        models_dev_id: "perplexity",
        api_key_env: "PERPLEXITY_API_KEY",
        models: PERPLEXITY_MODELS,
    },
    ProviderRegistryEntry {
        id: "deepinfra",
        models_dev_id: "deepinfra",
        api_key_env: "DEEPINFRA_API_KEY",
        models: DEEPINFRA_MODELS,
    },
    ProviderRegistryEntry {
        id: "moonshotai",
        models_dev_id: "moonshotai",
        api_key_env: "MOONSHOT_API_KEY",
        models: MOONSHOTAI_MODELS,
    },
    ProviderRegistryEntry {
        id: "baseten",
        models_dev_id: "baseten",
        api_key_env: "BASETEN_API_KEY",
        models: BASETEN_MODELS,
    },
    ProviderRegistryEntry {
        id: "302ai",
        models_dev_id: "302ai",
        api_key_env: "302AI_API_KEY",
        models: AI302_MODELS,
    },
    ProviderRegistryEntry {
        id: "cortecs",
        models_dev_id: "cortecs",
        api_key_env: "CORTECS_API_KEY",
        models: CORTECS_MODELS,
    },
    ProviderRegistryEntry {
        id: "comtegra",
        models_dev_id: "comtegra",
        api_key_env: "COMTEGRA_API_KEY",
        models: COMTEGRA_MODELS,
    },
    ProviderRegistryEntry {
        id: "fpt",
        models_dev_id: "fpt",
        api_key_env: "FPT_API_KEY",
        models: FPT_MODELS,
    },
    ProviderRegistryEntry {
        id: "firmware",
        models_dev_id: "firmware",
        api_key_env: "FIRMWARE_API_KEY",
        models: FIRMWARE_MODELS,
    },
    ProviderRegistryEntry {
        id: "huggingface",
        models_dev_id: "huggingface",
        api_key_env: "HF_TOKEN",
        models: HUGGINGFACE_MODELS,
    },
    ProviderRegistryEntry {
        id: "nebius",
        models_dev_id: "nebius",
        api_key_env: "NEBIUS_API_KEY",
        models: NEBIUS_MODELS,
    },
    ProviderRegistryEntry {
        id: "scaleway",
        models_dev_id: "scaleway",
        api_key_env: "SCALEWAY_API_KEY",
        models: SCALEWAY_MODELS,
    },
    ProviderRegistryEntry {
        id: "stackit",
        models_dev_id: "stackit",
        api_key_env: "STACKIT_API_KEY",
        models: STACKIT_MODELS,
    },
    ProviderRegistryEntry {
        id: "celeris",
        models_dev_id: "celeris",
        api_key_env: "CELERIS_API_KEY",
        models: CELERIS_MODELS,
    },
    ProviderRegistryEntry {
        id: "xiaomi-mimo",
        models_dev_id: "xiaomi-mimo",
        api_key_env: "XIAOMI_MIMO_API_KEY",
        models: XIAOMI_MIMO_MODELS,
    },
    ProviderRegistryEntry {
        id: "meta-muse",
        models_dev_id: "meta-muse",
        api_key_env: "META_MUSE_API_KEY",
        models: META_MUSE_MODELS,
    },
    ProviderRegistryEntry {
        id: "alibaba-coding-plan",
        models_dev_id: "alibaba",
        api_key_env: "BAILIAN_CODING_PLAN_API_KEY",
        models: ALIBABA_CODING_PLAN_MODELS,
    },
    ProviderRegistryEntry {
        id: "belvedir",
        models_dev_id: "belvedir",
        api_key_env: "BELVEDIR_API_KEY",
        models: BELVEDIR_MODELS,
    },
    ProviderRegistryEntry {
        id: "wafer",
        models_dev_id: "wafer",
        api_key_env: "WAFER_API_KEY",
        models: WAFER_MODELS,
    },
];

/// Look up a provider's registry entry by canonical profile id.
pub fn registry_entry(provider_id: &str) -> Option<&'static ProviderRegistryEntry> {
    let key = provider_id.trim().to_ascii_lowercase();
    REGISTRY.iter().find(|entry| entry.id == key)
}

/// Look up a model's context window from its provider's registry entry.
/// Matching is case-insensitive and exact per registered model id.
pub fn registry_context_limit(provider_id: &str, model: &str) -> Option<usize> {
    let entry = registry_entry(provider_id)?;
    let model = model.trim();
    entry
        .models
        .iter()
        .find(|m| m.model_id.eq_ignore_ascii_case(model))
        .map(|m| m.context_window)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fireworks_registry_entry_resolves_static_models() {
        let entry = registry_entry("fireworks").expect("fireworks in registry");
        assert_eq!(entry.models_dev_id, "fireworks-ai");
        assert_eq!(entry.api_key_env, "FIREWORKS_API_KEY");
        assert!(entry.models.len() >= 6);
    }

    #[test]
    fn cerebras_registry_entry_resolves_static_models() {
        let entry = registry_entry("cerebras").expect("cerebras in registry");
        assert_eq!(entry.models_dev_id, "cerebras");
        assert_eq!(entry.api_key_env, "CEREBRAS_API_KEY");
        assert_eq!(entry.models.len(), 2);
    }

    #[test]
    fn registry_context_limit_matches_registered_ids() {
        assert_eq!(
            registry_context_limit("fireworks", "accounts/fireworks/models/kimi-k2p5"),
            Some(262_144)
        );
        assert_eq!(
            registry_context_limit("cerebras", "gpt-oss-120b"),
            Some(131_072)
        );
        assert_eq!(
            registry_context_limit("deepseek", "deepseek-v4-pro"),
            Some(1_000_000)
        );
        assert_eq!(registry_context_limit("fireworks", "unknown"), None);
    }
}
