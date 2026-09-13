use super::Config;

struct HomeGuard(Option<std::ffi::OsString>);

impl Drop for HomeGuard {
    fn drop(&mut self) {
        match self.0.take() {
            Some(value) => crate::env::set_var("JCODE_HOME", value),
            None => crate::env::remove_var("JCODE_HOME"),
        }
        Config::invalidate_cache();
    }
}

fn with_config(contents: &str, check: impl FnOnce(&std::path::Path)) {
    let _lock = crate::storage::lock_test_env();
    let home = tempfile::tempdir().unwrap();
    let _guard = HomeGuard(std::env::var_os("JCODE_HOME"));
    crate::env::set_var("JCODE_HOME", home.path());
    let path = home.path().join("config.toml");
    std::fs::write(&path, contents).unwrap();
    check(&path);
}

const CONFIG: &str = r#"
[provider]
default_model = "gpt-6-astra"
[agents]
swarm_model = "legacy-model"
[agents.roles.implementer]
model = "cerebras:qwen-3.8-27b"
fallbacks = ["cerebras:gpt-oss-120b"]
[agents.roles.reviewer]
model = "openai-api:gpt-6-astra"
"#;

#[test]
fn routing_settings_select_and_clear_preserve_other_settings() {
    with_config(CONFIG, |path| {
        let original: Config = toml::from_str(CONFIG).unwrap();
        for role in [Some("implementer"), Some("reviewer"), None] {
            Config::set_agents_default_role(role).unwrap();
            let saved: Config = toml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
            assert_eq!(saved.agents.default_role.as_deref(), role);
            assert_eq!(
                saved.provider.default_model,
                original.provider.default_model
            );
            assert_eq!(saved.agents.swarm_model, original.agents.swarm_model);
            assert_eq!(
                serde_json::to_value(saved.agents.roles).unwrap(),
                serde_json::to_value(&original.agents.roles).unwrap()
            );
        }
    });
}

#[test]
fn routing_settings_reject_unknown_or_blank_role_without_writing() {
    with_config(CONFIG, |path| {
        for role in ["missing", "", " implementer "] {
            assert!(Config::set_agents_default_role(Some(role)).is_err());
            assert_eq!(std::fs::read_to_string(path).unwrap(), CONFIG);
        }
    });
}

#[test]
fn routing_settings_reject_malformed_config_without_writing() {
    let malformed = "[agents]\ndefault_role = [";
    with_config(malformed, |path| {
        assert!(Config::set_agents_default_role(None).is_err());
        assert_eq!(std::fs::read_to_string(path).unwrap(), malformed);
    });
}

#[test]
fn routing_settings_role_model_update_preserves_other_settings() {
    with_config(CONFIG, |path| {
        Config::set_agents_role_model("reviewer", "claude-api:claude-opus-4-6").unwrap();
        let saved: Config = toml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(
            saved.agents.roles["reviewer"].model,
            "claude-api:claude-opus-4-6"
        );
        assert_eq!(
            saved.agents.roles["implementer"].model,
            "cerebras:qwen-3.8-27b"
        );
        assert_eq!(
            saved.agents.roles["implementer"].fallbacks,
            vec!["cerebras:gpt-oss-120b"]
        );
        assert_eq!(saved.provider.default_model.as_deref(), Some("gpt-6-astra"));
    });
}

#[test]
fn routing_settings_role_fallback_update_dedupes_and_keeps_primary() {
    with_config(CONFIG, |path| {
        Config::set_agents_role_fallbacks(
            "reviewer",
            &[
                "cerebras:gpt-oss-120b".to_string(),
                "  ".to_string(),
                "openai-api:gpt-6-astra".to_string(),
                "cerebras:gpt-oss-120b".to_string(),
            ],
        )
        .unwrap();
        let saved: Config = toml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        // The primary model is dropped from fallbacks while order is preserved.
        assert_eq!(
            saved.agents.roles["reviewer"].fallbacks,
            vec!["cerebras:gpt-oss-120b"]
        );
        Config::set_agents_role_fallbacks("reviewer", &[]).unwrap();
        let cleared: Config = toml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert!(cleared.agents.roles["reviewer"].fallbacks.is_empty());
    });
}

#[test]
fn routing_settings_role_updates_reject_unknown_or_invalid_models() {
    with_config(CONFIG, |path| {
        for result in [
            Config::set_agents_role_model("missing", "openai-api:gpt-6-astra"),
            Config::set_agents_role_model("reviewer", "inherit"),
            Config::set_agents_role_model("reviewer", "  "),
            Config::set_agents_role_fallbacks("missing", &["openai-api:gpt-6-astra".to_string()]),
            Config::set_agents_role_fallbacks("reviewer", &["auto".to_string()]),
        ] {
            assert!(result.is_err());
        }
        assert_eq!(std::fs::read_to_string(path).unwrap(), CONFIG);
    });
}
