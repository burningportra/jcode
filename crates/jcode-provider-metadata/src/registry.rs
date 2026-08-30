//! Declarative provider registry.
//!
//! One data table describes a provider's identity, models, and environment
//! wiring. Downstream consumers (static model fallbacks, context limits,
//! pricing lookup, env-key detection) derive their tables from this instead of
//! maintaining parallel hand-written match arms across crates.
//!
//! Proof-of-concept: Fireworks. The registry entry is authoritative; the
//! legacy sites read from it. Parity tests in jcode-base fail loudly if the
//! hand-written tables drift from the registry, so adding a Fireworks model
//! becomes a one-line data change.

/// A single model a provider is known to serve, with the metadata jcode needs
/// before the provider's live `/v1/models` catalog is available.
pub struct ProviderModelEntry {
    pub model_id: &'static str,
    pub context_window: usize,
}

/// Declarative metadata for one OpenAI-compatible provider.
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

/// Fireworks AI: OpenAI-compatible endpoint serving Kimi, GLM, and DeepSeek
/// weights under `accounts/fireworks/...` model ids.
///
/// Source: <https://docs.fireworks.ai/tools-sdks/openai-compatibility>
pub const FIREWORKS_MODELS: &[ProviderModelEntry] = &[
    // Router endpoints fan out to the underlying model's serving stack.
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

/// All registry entries. Fireworks is the proof case; other providers migrate
/// incrementally, and a parity test in jcode-base holds the two in sync.
pub const REGISTRY: &[ProviderRegistryEntry] = &[ProviderRegistryEntry {
    id: "fireworks",
    models_dev_id: "fireworks-ai",
    api_key_env: "FIREWORKS_API_KEY",
    models: FIREWORKS_MODELS,
}];

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
    fn registry_context_limit_matches_fireworks_ids() {
        assert_eq!(
            registry_context_limit("fireworks", "accounts/fireworks/models/kimi-k2p5"),
            Some(262_144)
        );
        assert_eq!(
            registry_context_limit("fireworks", "accounts/fireworks/models/deepseek-v3p2"),
            Some(163_840)
        );
        assert_eq!(registry_context_limit("fireworks", "unknown"), None);
        assert_eq!(registry_context_limit("cerebras", "gpt-oss-120b"), None);
    }
}
