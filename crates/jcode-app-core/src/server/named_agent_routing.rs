use super::comm_session::{SwarmSpawnSelection, resolve_coordinator_spawn_identity};
use super::{SessionAgents, SwarmMember};
use crate::provider::{ModelRoute, MultiProvider, Provider};
use jcode_swarm_core::AgentRoutingSelection;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Resolve only before dispatch. Catalog availability is not a live inference probe.
pub(super) fn resolve(
    config: &crate::config::AgentsConfig,
    requested_role: Option<&str>,
    routes: &[ModelRoute],
) -> anyhow::Result<Option<AgentRoutingSelection>> {
    config.validate_roles().map_err(anyhow::Error::msg)?;
    let Some(name) = requested_role.or(config.default_role.as_deref()) else {
        return Ok(None);
    };
    let role = config.roles.get(name).ok_or_else(|| {
        anyhow::anyhow!("Unknown agent_role '{name}': define agents.roles.{name} in config.toml")
    })?;
    let mut unavailable = Vec::new();
    for candidate in std::iter::once(&role.model).chain(&role.fallbacks) {
        let mut matches = routes
            .iter()
            .filter(|route| route.available && route_matches(candidate, route))
            .collect::<Vec<_>>();
        matches.sort_by(|a, b| {
            (&a.api_method, &a.provider, &a.model).cmp(&(&b.api_method, &b.provider, &b.model))
        });
        matches.dedup_by(|a, b| {
            a.api_method == b.api_method && a.provider == b.provider && a.model == b.model
        });
        if matches.len() > 1 {
            anyhow::bail!(
                "Ambiguous model '{candidate}' for agent_role '{name}': use a provider/auth-qualified model from swarm list_models"
            );
        }
        if let Some(route) = matches.first() {
            return Ok(Some(AgentRoutingSelection {
                agent_role: name.to_string(),
                model: route.model.clone(),
                provider: route.provider.clone(),
                api_method: route.api_method.clone(),
                provider_key: Some(
                    jcode_provider_core::RouteSelection::from_model_route(route)
                        .runtime_key
                        .stable_id(),
                ),
            }));
        }
        unavailable.push(candidate.as_str());
    }
    anyhow::bail!(
        "No available route for agent_role '{name}'. Tried in order: {}. Check swarm list_models and provider authentication. No task was dispatched.",
        unavailable.join(", ")
    )
}

pub(super) fn route_matches(candidate: &str, route: &ModelRoute) -> bool {
    let selected = MultiProvider::default_model_selection_from_route(
        &route.model,
        &route.api_method,
        &route.provider,
    );
    let request = MultiProvider::model_switch_request_for_session_route(
        &route.model,
        selected.provider_key.as_deref(),
        Some(&route.api_method),
    );
    candidate == request
        || candidate
            == jcode_provider_core::RouteSelection::from_model_route(route).routed_model_spec()
        || candidate == selected.model_spec
        || candidate == route.model
}

pub(super) fn load(
    requested_role: Option<&str>,
    provider: &dyn Provider,
) -> anyhow::Result<Option<AgentRoutingSelection>> {
    let config = crate::config::Config::load_strict()?;
    resolve(&config.agents, requested_role, &provider.model_routes())
}

pub(super) fn spawn_selection(route: &AgentRoutingSelection) -> SwarmSpawnSelection {
    SwarmSpawnSelection {
        model: Some(if route.api_method == "openrouter" {
            exact_selection(route).routed_model_spec()
        } else {
            route.model.clone()
        }),
        provider_key: route.provider_key.clone(),
        route_api_method: Some(route.api_method.clone()),
    }
}

/// A historical label alone is not proof: verify the worker's current model/auth too.
pub(super) async fn matches_worker(
    session: &str,
    route: &AgentRoutingSelection,
    sessions: &SessionAgents,
    members: &Arc<RwLock<HashMap<String, SwarmMember>>>,
) -> bool {
    if members
        .read()
        .await
        .get(session)
        .and_then(|m| m.routing.as_ref())
        != Some(route)
    {
        return false;
    }
    if let Some(agent) = sessions.read().await.get(session).cloned() {
        if let Ok(agent) = agent.try_lock() {
            return verify_provider(agent.provider_handle().as_ref(), &exact_selection(route))
                .is_ok();
        }
    }
    let identity = resolve_coordinator_spawn_identity(session, sessions).await;
    identity.model.as_deref() == Some(route.model.as_str())
        && identity.route_api_method.as_deref() == Some(route.api_method.as_str())
        && identity.provider_key == route.provider_key
        && route.api_method != "openrouter" // A busy provider's upstream pin cannot be proven from legacy session fields.
}

pub(super) async fn eligible_members(
    route: Option<&AgentRoutingSelection>,
    sessions: &SessionAgents,
    members: &Arc<RwLock<HashMap<String, SwarmMember>>>,
) -> Arc<RwLock<HashMap<String, SwarmMember>>> {
    let Some(route) = route else {
        return Arc::clone(members);
    };
    let mut eligible = members.read().await.clone();
    let ids = eligible.keys().cloned().collect::<Vec<_>>();
    for id in ids {
        if !matches_worker(&id, route, sessions, members).await {
            eligible.remove(&id);
        }
    }
    Arc::new(RwLock::new(eligible))
}

/// Check the actual fork, not the session metadata just written by selection.
pub(super) fn verify_provider(
    provider: &dyn Provider,
    route: &jcode_provider_core::RouteSelection,
) -> anyhow::Result<()> {
    use jcode_provider_core::{ResolvedCredential, RuntimeKey};
    let name = provider.name().to_ascii_lowercase();
    let runtime_matches = match &route.runtime_key {
        RuntimeKey::ClaudeOAuth | RuntimeKey::AnthropicApiKey => {
            matches!(name.as_str(), "claude" | "anthropic")
        }
        RuntimeKey::OpenAIOAuth | RuntimeKey::OpenAIApiKey => {
            matches!(name.as_str(), "openai" | "codex")
        }
        RuntimeKey::OpenAiCompatible { .. } => provider
            .direct_openai_compatible_route_parts()
            .is_some_and(|(_, method, _)| method == route.api_method),
        RuntimeKey::OpenRouter => {
            provider.supports_provider_routing_features()
                && (route.provider_label.eq_ignore_ascii_case("auto")
                    || provider
                        .explicit_provider_pin_for_current_model()
                        .as_deref()
                        == Some(route.provider_label.as_str()))
        }
        _ => {
            name == route.runtime_key.stable_id().to_ascii_lowercase()
                || provider
                    .runtime_display_name()
                    .eq_ignore_ascii_case(&route.provider_label)
        }
    };
    let credential = match route.runtime_key {
        RuntimeKey::ClaudeOAuth | RuntimeKey::OpenAIOAuth => Some(ResolvedCredential::Oauth),
        RuntimeKey::AnthropicApiKey | RuntimeKey::OpenAIApiKey => Some(ResolvedCredential::ApiKey),
        _ => None,
    };
    if !runtime_matches
        || credential
            .is_some_and(|expected| provider.active_resolved_credential() != Some(expected))
        || provider.model() != route.model
    {
        anyhow::bail!(
            "Requested route {} / {} ({}), but runtime is {} / {} ({:?}); refusing silent provider/auth fallback",
            route.model,
            route.provider_label,
            route.api_method,
            provider.model(),
            provider.name(),
            provider.active_resolved_credential()
        );
    }
    Ok(())
}

pub(super) fn exact_selection(
    route: &AgentRoutingSelection,
) -> jcode_provider_core::RouteSelection {
    jcode_provider_core::RouteSelection {
        model: route.model.clone(),
        provider_label: route.provider.clone(),
        api_method: route.api_method.clone(),
        detail: String::new(),
        runtime_key: jcode_provider_core::RuntimeKey::from_api_method(
            &jcode_provider_core::ModelRouteApiMethod::parse(&route.api_method),
            &route.provider,
        ),
    }
}

#[cfg(test)]
#[path = "named_agent_routing_tests.rs"]
mod tests;
