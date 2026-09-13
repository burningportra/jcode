use super::*;
use crate::config::AgentsConfig;
use crate::message::{Message, ToolDefinition};
use crate::provider::EventStream;
use jcode_provider_core::ResolvedCredential;

fn config() -> AgentsConfig {
    serde_json::from_value(serde_json::json!({
        "default_role": "implementer",
        "roles": {
            "implementer": {"model": "openai-api:gpt-test", "fallbacks": ["claude-api:claude-test"]},
            "reviewer": {"model": "claude-api:claude-test"}
        }
    })).unwrap()
}

fn route(model: &str, provider: &str, api_method: &str, available: bool) -> ModelRoute {
    ModelRoute {
        model: model.into(),
        provider: provider.into(),
        api_method: api_method.into(),
        available,
        detail: String::new(),
        cheapness: None,
    }
}

fn routes() -> Vec<ModelRoute> {
    vec![
        route("gpt-test", "OpenAI", "openai-api-key", true),
        route("claude-test", "Anthropic", "anthropic-api-key", true),
    ]
}

#[test]
fn named_role_default_and_explicit_role_have_deterministic_precedence() {
    let cfg = config();
    let selected = resolve(&cfg, None, &routes()).unwrap().unwrap();
    assert_eq!(selected.agent_role, "implementer");
    assert_eq!(selected.provider_key.as_deref(), Some("openai-api-key"));
    let explicit = resolve(&cfg, Some("reviewer"), &routes()).unwrap().unwrap();
    assert_eq!(explicit.agent_role, "reviewer");
    assert_eq!(explicit.model, "claude-test");
    let mut reversed = routes();
    reversed.reverse();
    assert_eq!(resolve(&cfg, None, &reversed).unwrap().unwrap(), selected);
}

#[test]
fn named_role_fallbacks_follow_configuration_order_and_skip_unavailable() {
    let mut available = routes();
    available[0].available = false;
    let selected = resolve(&config(), None, &available).unwrap().unwrap();
    assert_eq!(selected.model, "claude-test");
    available[1].available = false;
    let error = resolve(&config(), None, &available)
        .unwrap_err()
        .to_string();
    assert!(error.contains("openai-api:gpt-test, claude-api:claude-test"));
    assert!(error.contains("No task was dispatched"));
}

#[test]
fn named_role_never_substitutes_oauth_for_api_key() {
    let mut cfg = config();
    cfg.roles.get_mut("implementer").unwrap().fallbacks.clear();
    let available = vec![
        route("gpt-test", "OpenAI", "openai-oauth", true),
        route("gpt-test", "OpenAI", "openai-api-key", false),
    ];
    assert!(resolve(&cfg, None, &available).is_err());
}

#[test]
fn named_role_unknown_and_blank_roles_fail_instead_of_inheriting() {
    for name in [
        "missing",
        "",
        " implementer ",
        "inherit",
        "agent",
        "coordinator",
    ] {
        let error = resolve(&config(), Some(name), &routes())
            .unwrap_err()
            .to_string();
        assert!(error.contains("Unknown agent_role"), "{error}");
    }
    assert!(
        resolve(&AgentsConfig::default(), None, &[])
            .unwrap()
            .is_none()
    );
}

#[test]
fn named_role_ambiguous_bare_model_requires_qualification() {
    let mut cfg = config();
    cfg.roles.get_mut("implementer").unwrap().model = "gpt-test".into();
    let available = vec![
        route("gpt-test", "OpenAI", "openai-api-key", true),
        route("gpt-test", "OpenAI", "openai-oauth", true),
    ];
    assert!(
        resolve(&cfg, None, &available)
            .unwrap_err()
            .to_string()
            .contains("Ambiguous")
    );
}

#[test]
fn named_role_preserves_openrouter_upstream_pin() {
    let mut cfg = config();
    cfg.roles.get_mut("implementer").unwrap().model = "openai/gpt-test@Cerebras".into();
    let available = vec![
        route("openai/gpt-test", "Other", "openrouter", true),
        route("openai/gpt-test", "Cerebras", "openrouter", true),
    ];
    let selected = resolve(&cfg, None, &available).unwrap().unwrap();
    assert_eq!(selected.provider, "Cerebras");
    assert_eq!(
        exact_selection(&selected).routed_model_spec(),
        "openai/gpt-test@Cerebras"
    );
    assert_eq!(
        spawn_selection(&selected).model.as_deref(),
        Some("openai/gpt-test@Cerebras")
    );
}

#[test]
fn named_role_preserves_named_compatible_profile() {
    let mut cfg = config();
    cfg.roles.get_mut("implementer").unwrap().model = "lab:gpt-test".into();
    let available = vec![
        route("gpt-test", "Other", "openai-compatible:other", true),
        route("gpt-test", "Lab", "openai-compatible:lab", true),
    ];
    let selected = resolve(&cfg, None, &available).unwrap().unwrap();
    assert_eq!(selected.api_method, "openai-compatible:lab");
    assert_eq!(
        exact_selection(&selected).routed_model_spec(),
        "lab:gpt-test"
    );
}

#[test]
fn named_role_rejects_invalid_configuration_even_when_unused() {
    for invalid in [
        "",
        "inherit",
        "coordinator",
        "auto",
        "jcode-auto",
        " gpt-test",
        "openai-api:",
    ] {
        let mut cfg = config();
        cfg.roles.get_mut("reviewer").unwrap().model = invalid.into();
        assert!(
            resolve(&cfg, None, &routes())
                .unwrap_err()
                .to_string()
                .contains("Invalid model")
        );
    }
    let mut cfg = config();
    cfg.default_role = Some("missing".into());
    assert!(
        resolve(&cfg, Some("reviewer"), &routes())
            .unwrap_err()
            .to_string()
            .contains("agents.default_role")
    );
    let malformed = serde_json::json!({"roles": {"a": {"model": "gpt-test", "fallback": []}}});
    assert!(serde_json::from_value::<AgentsConfig>(malformed).is_err());
}

#[derive(Clone)]
struct Runtime {
    name: &'static str,
    credential: Option<ResolvedCredential>,
    pin: Option<&'static str>,
}
#[async_trait::async_trait]
impl Provider for Runtime {
    async fn complete(
        &self,
        _: &[Message],
        _: &[ToolDefinition],
        _: &str,
        _: Option<&str>,
    ) -> anyhow::Result<EventStream> {
        panic!("routing checks must not run tasks")
    }
    fn name(&self) -> &str {
        self.name
    }
    fn model(&self) -> String {
        "gpt-test".into()
    }
    fn fork(&self) -> Arc<dyn Provider> {
        Arc::new(self.clone())
    }
    fn active_resolved_credential(&self) -> Option<ResolvedCredential> {
        self.credential
    }
    fn supports_provider_routing_features(&self) -> bool {
        self.name == "OpenRouter"
    }
    fn explicit_provider_pin_for_current_model(&self) -> Option<String> {
        self.pin.map(str::to_owned)
    }
}

#[test]
fn named_role_verifies_actual_provider_and_auth_not_just_model() {
    let selection = jcode_provider_core::RouteSelection::from_model_route(&routes()[0]);
    assert!(
        verify_provider(
            &Runtime {
                name: "OpenAI",
                credential: Some(ResolvedCredential::ApiKey),
                pin: None
            },
            &selection
        )
        .is_ok()
    );
    for (name, credential) in [
        ("OpenAI", Some(ResolvedCredential::Oauth)),
        ("OpenAI", None),
        ("Claude", Some(ResolvedCredential::ApiKey)),
    ] {
        assert!(
            verify_provider(
                &Runtime {
                    name,
                    credential,
                    pin: None
                },
                &selection
            )
            .is_err()
        );
    }
}

#[test]
fn named_role_verifies_actual_upstream_pin() {
    let selection = jcode_provider_core::RouteSelection::from_model_route(&route(
        "gpt-test",
        "Cerebras",
        "openrouter",
        true,
    ));
    for pin in [None, Some("Other")] {
        assert!(
            verify_provider(
                &Runtime {
                    name: "OpenRouter",
                    credential: None,
                    pin
                },
                &selection
            )
            .is_err()
        );
    }
    assert!(
        verify_provider(
            &Runtime {
                name: "OpenRouter",
                credential: None,
                pin: Some("Cerebras")
            },
            &selection
        )
        .is_ok()
    );
}
