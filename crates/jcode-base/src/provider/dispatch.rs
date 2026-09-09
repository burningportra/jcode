use super::*;

#[derive(Clone, Copy)]
pub(super) enum CompletionMode<'a> {
    Unified {
        system: &'a str,
    },
    Split {
        system_static: &'a str,
        system_dynamic: &'a str,
    },
}

impl CompletionMode<'_> {
    pub(super) fn log_suffix(self) -> &'static str {
        match self {
            CompletionMode::Unified { .. } => "",
            CompletionMode::Split { .. } => " (split)",
        }
    }

    pub(super) fn switch_log_prefix(self) -> &'static str {
        match self {
            CompletionMode::Unified { .. } => "Auto-fallback",
            CompletionMode::Split { .. } => "Auto-fallback (split)",
        }
    }
}

impl MultiProvider {
    pub(super) fn estimate_request_input(
        messages: &[Message],
        tools: &[ToolDefinition],
        mode: CompletionMode<'_>,
    ) -> (usize, usize) {
        let mut chars = serde_json::to_string(messages)
            .map(|value| value.len())
            .unwrap_or(0)
            + serde_json::to_string(tools)
                .map(|value| value.len())
                .unwrap_or(0);
        match mode {
            CompletionMode::Unified { system } => {
                chars += system.len();
            }
            CompletionMode::Split {
                system_static,
                system_dynamic,
            } => {
                chars += system_static.len() + system_dynamic.len();
            }
        }
        let tokens = chars / 4;
        (chars, tokens)
    }

    pub(super) async fn complete_auto_routed(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        mode: CompletionMode<'_>,
        resume_session_id: Option<&str>,
    ) -> Result<EventStream> {
        let routes = self.fresh_routes_memo_entry().routes;
        let config = crate::config::config().auto_router.clone();
        let (_, estimated_tokens) = Self::estimate_request_input(messages, tools, mode);
        let candidates = auto_router::filter_candidates_by_context(
            &catalog_routes::auto_candidates_for_routes(&routes, &config),
            estimated_tokens,
        );
        if candidates.is_empty() {
            anyhow::bail!("Auto router has no available tier candidates");
        }

        let category = {
            let state = self
                .auto_route_state
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            auto_router::classify_turn(messages, tools, state.decisions.len())
        };
        let history_portable = auto_router::history_is_portable(messages);
        let mut attempted_specs = Vec::new();
        let mut notes = Vec::new();

        let mut decision = self
            .select_auto_decision(category, &candidates, history_portable)
            .ok_or_else(|| anyhow!("Auto router could not resolve a model for this turn"))?;

        loop {
            attempted_specs.push(decision.model_spec.clone());
            let selection = self.route_selection_for_auto_decision(&decision, &routes)?;
            match self
                .complete_on_provider_with_model(
                    &selection,
                    messages,
                    tools,
                    mode,
                    resume_session_id,
                )
                .await
            {
                Ok(stream) => {
                    self.record_provider_activity(Self::active_provider_for_route_selection(
                        &selection,
                    ));
                    return Ok(stream);
                }
                Err(err) => {
                    let summary = Self::summarize_error(&err);
                    let failover = Self::classify_failover_error(&err);
                    crate::logging::info(&format!(
                        "Auto router model {} failed{}: {} (retryable={} decision={})",
                        decision.model_spec,
                        mode.log_suffix(),
                        summary,
                        failover.should_failover(),
                        failover.as_str()
                    ));
                    notes.push(format!("{}: {}", decision.model_spec, summary));
                    if !failover.should_failover() {
                        return Err(err);
                    }

                    let Some(next_decision) = self.select_auto_escalation_decision(
                        category,
                        decision.tier,
                        &candidates,
                        history_portable,
                        &attempted_specs,
                    ) else {
                        return Err(err.context(format!(
                            "Auto router exhausted tier escalation ({})",
                            notes.join("; ")
                        )));
                    };
                    decision = next_decision;
                }
            }
        }
    }

    fn select_auto_decision(
        &self,
        category: auto_router::TurnCategory,
        candidates: &[auto_router::AutoModelCandidate],
        history_portable: bool,
    ) -> Option<auto_router::AutoDecision> {
        let mut state = self
            .auto_route_state
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        auto_router::select_candidate_for_category(
            &mut state,
            category,
            candidates,
            history_portable,
        )
    }

    fn select_auto_escalation_decision(
        &self,
        category: auto_router::TurnCategory,
        from_tier: auto_router::AutoTier,
        candidates: &[auto_router::AutoModelCandidate],
        history_portable: bool,
        attempted_specs: &[String],
    ) -> Option<auto_router::AutoDecision> {
        let tiers: &[auto_router::AutoTier] = match from_tier {
            auto_router::AutoTier::Fast => &[
                auto_router::AutoTier::Implement,
                auto_router::AutoTier::Frontier,
            ],
            auto_router::AutoTier::Implement => &[auto_router::AutoTier::Frontier],
            auto_router::AutoTier::Frontier => &[],
        };

        let mut state = self
            .auto_route_state
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let pinned_family = state
            .last_resolved
            .as_ref()
            .map(|decision| decision.provider_family.as_str())
            .or(state.conversation_provider_family.as_deref());

        let candidate = tiers.iter().find_map(|tier| {
            candidates.iter().find(|candidate| {
                candidate.tier == *tier
                    && !attempted_specs
                        .iter()
                        .any(|spec| spec == &candidate.model_spec)
                    && pinned_family
                        .map(|family| family == candidate.provider_family || history_portable)
                        .unwrap_or(true)
            })
        })?;

        let decision = auto_router::AutoDecision {
            tier: candidate.tier,
            model_spec: candidate.model_spec.clone(),
            provider_family: candidate.provider_family.clone(),
            reason: format!(
                "{}; escalated after retryable {} failure",
                candidate.reason,
                auto_router::tier_label(from_tier)
            ),
            at: std::time::Instant::now(),
        };
        state.record_decision(category, decision.clone());
        Some(decision)
    }

    async fn complete_on_provider_with_model(
        &self,
        selection: &RouteSelection,
        messages: &[Message],
        tools: &[ToolDefinition],
        mode: CompletionMode<'_>,
        resume_session_id: Option<&str>,
    ) -> Result<EventStream> {
        let fork = self.fork_multi_provider_for_request();
        fork.set_route_selection(selection)?;
        let active = fork.active_provider();

        let filtered_messages =
            image_clamp::filter_unsupported_outbound_images(messages, fork.supports_image_input());
        let messages: &[Message] = filtered_messages.as_deref().unwrap_or(messages);
        let clamped_messages = image_clamp::clamp_outbound_images(messages);
        let messages: &[Message] = clamped_messages.as_deref().unwrap_or(messages);

        match mode {
            CompletionMode::Unified { system } => {
                fork.complete_on_provider(active, messages, tools, system, resume_session_id)
                    .await
            }
            CompletionMode::Split {
                system_static,
                system_dynamic,
            } => {
                fork.complete_split_on_provider(
                    active,
                    messages,
                    tools,
                    system_static,
                    system_dynamic,
                    resume_session_id,
                )
                .await
            }
        }
    }

    fn route_selection_for_auto_decision(
        &self,
        decision: &auto_router::AutoDecision,
        routes: &[ModelRoute],
    ) -> Result<RouteSelection> {
        if let Some(selection) = routes
            .iter()
            .filter(|route| route.available)
            .find_map(|route| {
                let selection = RouteSelection::from_model_route(route);
                (selection.routed_model_spec() == decision.model_spec).then_some(selection)
            })
        {
            return Ok(selection);
        }

        if let Some(selection) = routes
            .iter()
            .filter(|route| route.available)
            .find_map(|route| {
                let selection = RouteSelection::from_model_route(route);
                (route.model == decision.model_spec
                    && selection.runtime_key == RuntimeKey::OpenRouter
                    && decision.provider_family == "openrouter")
                    .then_some(selection)
            })
        {
            return Ok(selection);
        }

        Self::route_selection_from_model_spec(&decision.model_spec).ok_or_else(|| {
            anyhow!(
                "Auto router resolved unknown model route '{}'",
                decision.model_spec
            )
        })
    }

    fn route_selection_from_model_spec(model_spec: &str) -> Option<RouteSelection> {
        let (runtime_key, api_method, provider_label, model) =
            if let Some(model) = model_spec.strip_prefix("claude-oauth:") {
                (RuntimeKey::ClaudeOAuth, "claude-oauth", "Anthropic", model)
            } else if let Some(model) = model_spec.strip_prefix("claude-api:") {
                (
                    RuntimeKey::AnthropicApiKey,
                    "anthropic-api-key",
                    "Anthropic",
                    model,
                )
            } else if let Some(model) = model_spec.strip_prefix("claude:") {
                (RuntimeKey::ClaudeOAuth, "claude-oauth", "Anthropic", model)
            } else if let Some(model) = model_spec.strip_prefix("anthropic:") {
                (
                    RuntimeKey::AnthropicApiKey,
                    "anthropic-api-key",
                    "Anthropic",
                    model,
                )
            } else if let Some(model) = model_spec.strip_prefix("openai-oauth:") {
                (RuntimeKey::OpenAIOAuth, "openai-oauth", "OpenAI", model)
            } else if let Some(model) = model_spec.strip_prefix("openai-api:") {
                (RuntimeKey::OpenAIApiKey, "openai-api-key", "OpenAI", model)
            } else if let Some(model) = model_spec.strip_prefix("openai:") {
                (RuntimeKey::OpenAIOAuth, "openai-oauth", "OpenAI", model)
            } else if let Some(model) = model_spec.strip_prefix("vercel-ai-gateway:") {
                (
                    RuntimeKey::OpenAiCompatible {
                        profile_id: Some("vercel-ai-gateway".to_string()),
                    },
                    "openai-compatible:vercel-ai-gateway",
                    "Vercel AI Gateway",
                    model,
                )
            } else if let Some(model) = model_spec.strip_prefix("gemini:") {
                (
                    RuntimeKey::CodeAssistOAuth,
                    "code-assist-oauth",
                    "Gemini",
                    model,
                )
            } else if let Some(model) = model_spec.strip_prefix("openrouter:") {
                (RuntimeKey::OpenRouter, "openrouter", "OpenRouter", model)
            } else {
                (
                    RuntimeKey::OpenRouter,
                    "openrouter",
                    "OpenRouter",
                    model_spec,
                )
            };

        let model = model.trim();
        (!model.is_empty()).then(|| RouteSelection {
            model: model.to_string(),
            runtime_key,
            api_method: api_method.to_string(),
            provider_label: provider_label.to_string(),
            detail: String::new(),
        })
    }

    fn active_provider_for_route_selection(selection: &RouteSelection) -> ActiveProvider {
        match selection.runtime_key {
            RuntimeKey::ClaudeOAuth | RuntimeKey::AnthropicApiKey => ActiveProvider::Claude,
            RuntimeKey::OpenAIOAuth | RuntimeKey::OpenAIApiKey => ActiveProvider::OpenAI,
            RuntimeKey::Copilot => ActiveProvider::Copilot,
            RuntimeKey::Cursor => ActiveProvider::Cursor,
            RuntimeKey::Bedrock => ActiveProvider::Bedrock,
            RuntimeKey::Antigravity => ActiveProvider::Antigravity,
            RuntimeKey::CodeAssistOAuth | RuntimeKey::Gemini => ActiveProvider::Gemini,
            RuntimeKey::OpenRouter
            | RuntimeKey::OpenAiCompatible { .. }
            | RuntimeKey::JcodeSubscription
            | RuntimeKey::RemoteCatalog
            | RuntimeKey::Current
            | RuntimeKey::Auto
            | RuntimeKey::Other(_) => ActiveProvider::OpenRouter,
        }
    }

    pub(super) async fn complete_on_provider(
        &self,
        provider: ActiveProvider,
        messages: &[Message],
        tools: &[ToolDefinition],
        system: &str,
        resume_session_id: Option<&str>,
    ) -> Result<EventStream> {
        self.reconcile_auth_if_provider_missing(provider);
        match provider {
            ActiveProvider::Claude => {
                if let Some(anthropic) = self.anthropic_provider() {
                    anthropic
                        .complete(messages, tools, system, resume_session_id)
                        .await
                } else if let Some(claude) = self.claude_provider() {
                    claude
                        .complete(messages, tools, system, resume_session_id)
                        .await
                } else {
                    Err(anyhow::anyhow!(
                        "Claude credentials not available. Run `claude` to log in."
                    ))
                }
            }
            ActiveProvider::OpenAI => {
                if let Some(openai) = self.openai_provider() {
                    openai
                        .complete(messages, tools, system, resume_session_id)
                        .await
                } else {
                    Err(anyhow::anyhow!(
                        "OpenAI credentials not available. Run `jcode login --provider openai` to log in."
                    ))
                }
            }
            ActiveProvider::Copilot => {
                let copilot = self
                    .copilot_api
                    .read()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .clone();
                if let Some(copilot) = copilot {
                    copilot
                        .complete(messages, tools, system, resume_session_id)
                        .await
                } else {
                    Err(anyhow::anyhow!(
                        "GitHub Copilot is not available. Run `jcode login --provider copilot`."
                    ))
                }
            }
            ActiveProvider::Antigravity => {
                let antigravity = self.antigravity_provider();
                if let Some(antigravity) = antigravity {
                    antigravity
                        .complete(messages, tools, system, resume_session_id)
                        .await
                } else {
                    Err(anyhow::anyhow!(
                        "Antigravity is not available. Run `jcode login --provider antigravity`."
                    ))
                }
            }
            ActiveProvider::Gemini => {
                let gemini = self
                    .gemini
                    .read()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .clone();
                if let Some(gemini) = gemini {
                    gemini
                        .complete(messages, tools, system, resume_session_id)
                        .await
                } else {
                    Err(anyhow::anyhow!(
                        "Gemini is not available. Run `jcode login --provider gemini`."
                    ))
                }
            }
            ActiveProvider::Cursor => {
                let cursor = self
                    .cursor
                    .read()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .clone();
                if let Some(cursor) = cursor {
                    cursor
                        .complete(messages, tools, system, resume_session_id)
                        .await
                } else {
                    Err(anyhow::anyhow!(
                        "Cursor is not available. Run `jcode login --provider cursor`."
                    ))
                }
            }
            ActiveProvider::Bedrock => {
                if let Some(bedrock) = self.bedrock_provider() {
                    bedrock
                        .complete(messages, tools, system, resume_session_id)
                        .await
                } else {
                    Err(anyhow::anyhow!(
                        "AWS Bedrock is not available. Configure AWS credentials and region, or set AWS_PROFILE/AWS_REGION."
                    ))
                }
            }
            ActiveProvider::OpenRouter => {
                let openrouter = self.active_openrouter_execution_provider();
                if let Some(openrouter) = openrouter {
                    openrouter
                        .complete(messages, tools, system, resume_session_id)
                        .await
                } else {
                    Err(anyhow::anyhow!(
                        "OpenRouter credentials not available. Set OPENROUTER_API_KEY environment variable."
                    ))
                }
            }
            ActiveProvider::InferenceNet => {
                if let Some(inference) = self.inference_provider() {
                    inference
                        .complete(messages, tools, system, resume_session_id)
                        .await
                } else {
                    Err(anyhow::anyhow!(
                        "Inference.net credentials not available. Run  to log in."
                    ))
                }
            }
        }
    }

    pub(super) async fn complete_split_on_provider(
        &self,
        provider: ActiveProvider,
        messages: &[Message],
        tools: &[ToolDefinition],
        system_static: &str,
        system_dynamic: &str,
        resume_session_id: Option<&str>,
    ) -> Result<EventStream> {
        self.reconcile_auth_if_provider_missing(provider);
        match provider {
            ActiveProvider::Claude => {
                if let Some(anthropic) = self.anthropic_provider() {
                    anthropic
                        .complete_split(
                            messages,
                            tools,
                            system_static,
                            system_dynamic,
                            resume_session_id,
                        )
                        .await
                } else if let Some(claude) = self.claude_provider() {
                    claude
                        .complete_split(
                            messages,
                            tools,
                            system_static,
                            system_dynamic,
                            resume_session_id,
                        )
                        .await
                } else {
                    Err(anyhow::anyhow!(
                        "Claude credentials not available. Run `claude` to log in."
                    ))
                }
            }
            ActiveProvider::OpenAI => {
                if let Some(openai) = self.openai_provider() {
                    openai
                        .complete_split(
                            messages,
                            tools,
                            system_static,
                            system_dynamic,
                            resume_session_id,
                        )
                        .await
                } else {
                    Err(anyhow::anyhow!(
                        "OpenAI credentials not available. Run `jcode login --provider openai` to log in."
                    ))
                }
            }
            ActiveProvider::Copilot => {
                let copilot = self
                    .copilot_api
                    .read()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .clone();
                if let Some(copilot) = copilot {
                    copilot
                        .complete_split(
                            messages,
                            tools,
                            system_static,
                            system_dynamic,
                            resume_session_id,
                        )
                        .await
                } else {
                    Err(anyhow::anyhow!(
                        "GitHub Copilot is not available. Run `jcode login --provider copilot`."
                    ))
                }
            }
            ActiveProvider::Antigravity => {
                let antigravity = self.antigravity_provider();
                if let Some(antigravity) = antigravity {
                    antigravity
                        .complete_split(
                            messages,
                            tools,
                            system_static,
                            system_dynamic,
                            resume_session_id,
                        )
                        .await
                } else {
                    Err(anyhow::anyhow!(
                        "Antigravity is not available. Run `jcode login --provider antigravity`."
                    ))
                }
            }
            ActiveProvider::Gemini => {
                let gemini = self
                    .gemini
                    .read()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .clone();
                if let Some(gemini) = gemini {
                    gemini
                        .complete_split(
                            messages,
                            tools,
                            system_static,
                            system_dynamic,
                            resume_session_id,
                        )
                        .await
                } else {
                    Err(anyhow::anyhow!(
                        "Gemini is not available. Run `jcode login --provider gemini`."
                    ))
                }
            }
            ActiveProvider::Cursor => {
                let cursor = self
                    .cursor
                    .read()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .clone();
                if let Some(cursor) = cursor {
                    cursor
                        .complete_split(
                            messages,
                            tools,
                            system_static,
                            system_dynamic,
                            resume_session_id,
                        )
                        .await
                } else {
                    Err(anyhow::anyhow!(
                        "Cursor is not available. Run `jcode login --provider cursor`."
                    ))
                }
            }
            ActiveProvider::Bedrock => {
                if let Some(bedrock) = self.bedrock_provider() {
                    bedrock
                        .complete_split(
                            messages,
                            tools,
                            system_static,
                            system_dynamic,
                            resume_session_id,
                        )
                        .await
                } else {
                    Err(anyhow::anyhow!(
                        "AWS Bedrock is not available. Configure AWS credentials and region, or set AWS_PROFILE/AWS_REGION."
                    ))
                }
            }
            ActiveProvider::OpenRouter => {
                let openrouter = self.active_openrouter_execution_provider();
                if let Some(openrouter) = openrouter {
                    openrouter
                        .complete_split(
                            messages,
                            tools,
                            system_static,
                            system_dynamic,
                            resume_session_id,
                        )
                        .await
                } else {
                    Err(anyhow::anyhow!(
                        "OpenRouter credentials not available. Set OPENROUTER_API_KEY environment variable."
                    ))
                }
            }
            ActiveProvider::InferenceNet => {
                if let Some(inference) = self.inference_provider() {
                    inference
                        .complete_split(
                            messages,
                            tools,
                            system_static,
                            system_dynamic,
                            resume_session_id,
                        )
                        .await
                } else {
                    Err(anyhow::anyhow!(
                        "Inference.net credentials not available. Run  to log in."
                    ))
                }
            }
        }
    }
}
