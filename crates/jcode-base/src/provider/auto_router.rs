#![allow(dead_code)]

use crate::message::{ContentBlock, Message, Role, ToolDefinition};
use std::collections::VecDeque;
use std::time::Instant;

pub(crate) const AUTO_MODEL_ID: &str = "jcode-auto";
pub(crate) const DECISION_HISTORY_LIMIT: usize = 50;
pub(crate) const UNKNOWN_GATEWAY_CONTEXT_TOKENS: usize = 128_000;

pub(crate) fn is_virtual_model(model: &str) -> bool {
    model.trim() == AUTO_MODEL_ID
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum AutoTier {
    Frontier,
    Implement,
    Fast,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum TurnCategory {
    Planning,
    Implementation,
    Mechanical,
    Unknown,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct AutoDecision {
    pub tier: AutoTier,
    pub model_spec: String,
    pub provider_family: String,
    pub reason: String,
    pub at: Instant,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct AutoRouteState {
    pub conversation_provider_family: Option<String>,
    pub last_category: Option<TurnCategory>,
    pub last_resolved: Option<AutoDecision>,
    pub forced_next_tier: Option<AutoTier>,
    pub decisions: VecDeque<AutoDecision>,
}

impl AutoRouteState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn force_next_tier(&mut self, tier: AutoTier) {
        self.forced_next_tier = Some(tier);
    }

    pub fn take_forced_next_tier(&mut self) -> Option<AutoTier> {
        self.forced_next_tier.take()
    }

    pub fn record_decision(&mut self, category: TurnCategory, decision: AutoDecision) {
        if self.conversation_provider_family.is_none() {
            self.conversation_provider_family = Some(decision.provider_family.clone());
        }
        self.last_category = Some(category);
        self.last_resolved = Some(decision.clone());
        self.decisions.push_back(decision);
        while self.decisions.len() > DECISION_HISTORY_LIMIT {
            self.decisions.pop_front();
        }
    }

    pub fn sticky_decision_for(&self, category: TurnCategory) -> Option<AutoDecision> {
        (self.last_category == Some(category))
            .then(|| self.last_resolved.clone())
            .flatten()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum FastProviderPreference {
    Auto,
    Vercel,
    OpenRouter,
    None,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct AutoRouterOverrides {
    pub frontier: Option<String>,
    pub implement: Option<String>,
    pub fast: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct AutoTierCatalog {
    pub anthropic_oauth_models: Vec<String>,
    pub openai_oauth_models: Vec<String>,
    pub vercel_ai_gateway_models: Vec<String>,
    pub openrouter_models: Vec<String>,
    pub subscription_flash_models: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AutoModelCandidate {
    pub tier: AutoTier,
    pub model_spec: String,
    pub provider_family: String,
    pub context_window_tokens: Option<usize>,
    pub reason: String,
}

impl AutoModelCandidate {
    pub fn new(
        tier: AutoTier,
        model_spec: impl Into<String>,
        provider_family: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            tier,
            model_spec: model_spec.into(),
            provider_family: provider_family.into(),
            context_window_tokens: None,
            reason: reason.into(),
        }
    }

    pub fn with_context_window(mut self, tokens: usize) -> Self {
        self.context_window_tokens = Some(tokens);
        self
    }
}

pub(crate) fn classify_turn(
    messages: &[Message],
    _tools: &[ToolDefinition],
    turn_index: usize,
) -> TurnCategory {
    if latest_message_is_only_tool_results(messages) {
        return TurnCategory::Mechanical;
    }

    let latest_user_text = latest_user_text(messages);
    let first_substantive_user_turn = turn_index == 0 || substantive_user_turns(messages) <= 1;

    if first_substantive_user_turn && latest_user_text.as_deref().is_some_and(has_substance) {
        return TurnCategory::Planning;
    }

    if let Some(text) = latest_user_text.as_deref() {
        if is_short_fix_this_after_large_tool_result(text, messages) {
            return TurnCategory::Unknown;
        }
        if has_planning_signal(text) {
            return TurnCategory::Planning;
        }
        if text.chars().count() < 200 && has_mechanical_signal(text) {
            return TurnCategory::Mechanical;
        }
    }

    if assistant_last_used_planning_tool(messages) {
        return TurnCategory::Planning;
    }

    TurnCategory::Implementation
}

pub(crate) fn history_is_portable(messages: &[Message]) -> bool {
    !messages.iter().any(|message| {
        message.content.iter().any(|block| match block {
            ContentBlock::AnthropicThinking { .. }
            | ContentBlock::OpenAIReasoning { .. }
            | ContentBlock::OpenAICompaction { .. } => true,
            ContentBlock::ToolUse {
                thought_signature, ..
            } => thought_signature
                .as_deref()
                .map(str::trim)
                .is_some_and(|signature| !signature.is_empty()),
            _ => false,
        })
    })
}

pub(crate) fn fast_seed_rank(model: &str) -> Option<usize> {
    let model = model.trim().to_ascii_lowercase();
    if model.is_empty() {
        return None;
    }
    let bare = model.rsplit('/').next().unwrap_or(&model);

    if model.contains("mercury") || bare.contains("mercury") {
        return Some(0);
    }
    if bare.starts_with("glm-") && bare.contains("-flash") {
        return Some(1);
    }
    if bare.starts_with("qwen") && bare.contains("-flash") {
        return Some(2);
    }
    if bare.starts_with("deepseek-v") && bare.contains("flash") {
        return Some(3);
    }
    if bare.starts_with("gemini-") && bare.contains("-flash") {
        return Some(4);
    }
    if bare.starts_with("minimax-h") {
        return Some(5);
    }

    None
}

pub(crate) fn model_matches_fast_seed(model: &str) -> bool {
    fast_seed_rank(model).is_some()
}

pub(crate) fn resolve_tier_candidates(
    catalog: &AutoTierCatalog,
    fast_provider: FastProviderPreference,
    overrides: &AutoRouterOverrides,
) -> Vec<AutoModelCandidate> {
    let mut candidates = Vec::new();
    append_frontier_candidates(&mut candidates, catalog, overrides.frontier.as_deref());
    append_implement_candidates(&mut candidates, catalog, overrides.implement.as_deref());
    append_fast_candidates(
        &mut candidates,
        catalog,
        fast_provider,
        overrides.fast.as_deref(),
    );
    candidates
}

pub(crate) fn select_candidate_for_category(
    state: &mut AutoRouteState,
    category: TurnCategory,
    candidates: &[AutoModelCandidate],
    history_portable: bool,
) -> Option<AutoDecision> {
    if state.forced_next_tier.is_none()
        && let Some(decision) = state.sticky_decision_for(category)
    {
        return Some(decision);
    }

    let desired_tier = state
        .take_forced_next_tier()
        .unwrap_or_else(|| desired_tier_for_category(category, candidates));
    let candidate = pick_candidate_for_tier(state, desired_tier, candidates, history_portable)
        .or_else(|| {
            if desired_tier == AutoTier::Fast {
                pick_candidate_for_tier(state, AutoTier::Implement, candidates, history_portable)
            } else {
                None
            }
        })?;

    let decision = AutoDecision {
        tier: candidate.tier,
        model_spec: candidate.model_spec.clone(),
        provider_family: candidate.provider_family.clone(),
        reason: candidate.reason.clone(),
        at: Instant::now(),
    };
    state.record_decision(category, decision.clone());
    Some(decision)
}

pub(crate) fn filter_candidates_by_context(
    candidates: &[AutoModelCandidate],
    estimated_request_tokens: usize,
) -> Vec<AutoModelCandidate> {
    candidates
        .iter()
        .filter(|candidate| {
            let Some(limit) = candidate.context_window_tokens.or_else(|| {
                is_gateway_family(&candidate.provider_family)
                    .then_some(UNKNOWN_GATEWAY_CONTEXT_TOKENS)
            }) else {
                return true;
            };
            estimated_request_tokens <= limit
        })
        .cloned()
        .collect()
}

fn append_frontier_candidates(
    candidates: &mut Vec<AutoModelCandidate>,
    catalog: &AutoTierCatalog,
    override_model: Option<&str>,
) {
    if let Some(model) = non_empty(override_model) {
        candidates.push(AutoModelCandidate::new(
            AutoTier::Frontier,
            model,
            provider_family_for_model_spec(model).unwrap_or("override"),
            "frontier override",
        ));
        return;
    }

    for model in &catalog.anthropic_oauth_models {
        if is_anthropic_frontier_model(model) {
            candidates.push(AutoModelCandidate::new(
                AutoTier::Frontier,
                format!("claude-oauth:{}", model.trim()),
                "anthropic",
                "Anthropic OAuth frontier model",
            ));
        }
    }
    for model in &catalog.openai_oauth_models {
        if is_openai_frontier_model(model) {
            candidates.push(AutoModelCandidate::new(
                AutoTier::Frontier,
                format!("openai-oauth:{}", model.trim()),
                "openai",
                "OpenAI OAuth frontier model",
            ));
        }
    }
}

fn append_implement_candidates(
    candidates: &mut Vec<AutoModelCandidate>,
    catalog: &AutoTierCatalog,
    override_model: Option<&str>,
) {
    if let Some(model) = non_empty(override_model) {
        candidates.push(AutoModelCandidate::new(
            AutoTier::Implement,
            model,
            provider_family_for_model_spec(model).unwrap_or("override"),
            "implement override",
        ));
        return;
    }

    for model in &catalog.anthropic_oauth_models {
        if is_anthropic_implement_model(model) {
            candidates.push(AutoModelCandidate::new(
                AutoTier::Implement,
                format!("claude-oauth:{}", model.trim()),
                "anthropic",
                "Anthropic OAuth implementation model",
            ));
        }
    }
    for model in &catalog.openai_oauth_models {
        if is_openai_implement_model(model) {
            candidates.push(AutoModelCandidate::new(
                AutoTier::Implement,
                format!("openai-oauth:{}", model.trim()),
                "openai",
                "OpenAI OAuth implementation model",
            ));
        }
    }
}

fn append_fast_candidates(
    candidates: &mut Vec<AutoModelCandidate>,
    catalog: &AutoTierCatalog,
    fast_provider: FastProviderPreference,
    override_model: Option<&str>,
) {
    if let Some(model) = non_empty(override_model) {
        candidates.push(AutoModelCandidate::new(
            AutoTier::Fast,
            model,
            provider_family_for_model_spec(model).unwrap_or("override"),
            "fast override",
        ));
        return;
    }
    if fast_provider == FastProviderPreference::None {
        return;
    }

    let transports: &[(FastProviderPreference, &str, &[String])] = &[
        (
            FastProviderPreference::Vercel,
            "vercel-ai-gateway",
            &catalog.vercel_ai_gateway_models,
        ),
        (
            FastProviderPreference::OpenRouter,
            "openrouter",
            &catalog.openrouter_models,
        ),
    ];

    for (preference, family, models) in transports {
        if fast_provider != FastProviderPreference::Auto && fast_provider != *preference {
            continue;
        }
        append_seeded_fast_models(candidates, models, family);
        if candidates.iter().any(|candidate| {
            candidate.tier == AutoTier::Fast && candidate.provider_family.as_str() == *family
        }) {
            return;
        }
    }

    if matches!(
        fast_provider,
        FastProviderPreference::Auto
            | FastProviderPreference::Vercel
            | FastProviderPreference::OpenRouter
    ) {
        append_seeded_fast_models(candidates, &catalog.subscription_flash_models, "gemini");
    }
}

fn append_seeded_fast_models(
    candidates: &mut Vec<AutoModelCandidate>,
    models: &[String],
    provider_family: &str,
) {
    let mut seeded = models
        .iter()
        .filter_map(|model| fast_seed_rank(model).map(|rank| (rank, model.trim())))
        .filter(|(_, model)| !model.is_empty())
        .collect::<Vec<_>>();
    seeded.sort_by(|(rank_a, model_a), (rank_b, model_b)| {
        rank_a.cmp(rank_b).then(model_a.cmp(model_b))
    });

    for (_rank, model) in seeded {
        let spec = match provider_family {
            "vercel-ai-gateway" => format!("vercel-ai-gateway:{model}"),
            "openrouter" => model.to_string(),
            "gemini" => format!("gemini:{model}"),
            other => format!("{other}:{model}"),
        };
        candidates.push(AutoModelCandidate::new(
            AutoTier::Fast,
            spec,
            provider_family,
            format!("{provider_family} fast seed match"),
        ));
    }
}

fn desired_tier_for_category(
    category: TurnCategory,
    candidates: &[AutoModelCandidate],
) -> AutoTier {
    match category {
        TurnCategory::Planning => AutoTier::Frontier,
        TurnCategory::Mechanical
            if candidates
                .iter()
                .any(|candidate| candidate.tier == AutoTier::Fast) =>
        {
            AutoTier::Fast
        }
        TurnCategory::Mechanical | TurnCategory::Implementation | TurnCategory::Unknown => {
            AutoTier::Implement
        }
    }
}

fn pick_candidate_for_tier<'a>(
    state: &AutoRouteState,
    tier: AutoTier,
    candidates: &'a [AutoModelCandidate],
    history_portable: bool,
) -> Option<&'a AutoModelCandidate> {
    candidates.iter().find(|candidate| {
        candidate.tier == tier
            && state
                .conversation_provider_family
                .as_deref()
                .map(|family| family == candidate.provider_family || history_portable)
                .unwrap_or(true)
    })
}

fn latest_message_is_only_tool_results(messages: &[Message]) -> bool {
    messages.last().is_some_and(|message| {
        !message.content.is_empty()
            && message
                .content
                .iter()
                .all(|block| matches!(block, ContentBlock::ToolResult { .. }))
    })
}

fn latest_user_text(messages: &[Message]) -> Option<String> {
    messages
        .iter()
        .rev()
        .find(|message| message.role == Role::User)
        .map(|message| message_text(message))
        .filter(|text| !text.trim().is_empty())
}

fn message_text(message: &Message) -> String {
    message
        .content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn substantive_user_turns(messages: &[Message]) -> usize {
    messages
        .iter()
        .filter(|message| message.role == Role::User && has_substance(&message_text(message)))
        .count()
}

fn has_substance(text: &str) -> bool {
    !text.trim().is_empty()
}

fn assistant_last_used_planning_tool(messages: &[Message]) -> bool {
    messages
        .iter()
        .rev()
        .find(|message| message.role == Role::Assistant)
        .is_some_and(|message| {
            message.content.iter().any(|block| match block {
                ContentBlock::ToolUse { name, .. } => {
                    name.eq_ignore_ascii_case("todo") || name.eq_ignore_ascii_case("plan")
                }
                _ => false,
            })
        })
}

fn has_planning_signal(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    ["plan", "architect", "design", "refactor", "review", "why"]
        .iter()
        .any(|word| contains_word(&lower, word))
        || contains_phrase(&lower, "how-does")
        || contains_phrase(&lower, "how does")
}

fn has_mechanical_signal(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    contains_word(&lower, "format")
        || contains_word(&lower, "rename")
        || contains_phrase(&lower, "run tests")
        || contains_word(&lower, "lint")
        || contains_phrase(&lower, "fix typo")
        || contains_phrase(&lower, "fix the typo")
}

fn contains_phrase(text: &str, phrase: &str) -> bool {
    text.contains(phrase)
}

fn contains_word(text: &str, word: &str) -> bool {
    let bytes = text.as_bytes();
    let word_bytes = word.as_bytes();
    if word_bytes.is_empty() || word_bytes.len() > bytes.len() {
        return false;
    }

    bytes
        .windows(word_bytes.len())
        .enumerate()
        .any(|(idx, window)| {
            window == word_bytes
                && !is_word_byte(bytes.get(idx.wrapping_sub(1)).copied())
                && !is_word_byte(bytes.get(idx + word_bytes.len()).copied())
        })
}

fn is_word_byte(byte: Option<u8>) -> bool {
    byte.is_some_and(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn is_short_fix_this_after_large_tool_result(text: &str, messages: &[Message]) -> bool {
    let lower = text.trim().to_ascii_lowercase();
    let short_fix_this = matches!(
        lower.as_str(),
        "fix this" | "please fix this" | "fix it" | "please fix it"
    );
    short_fix_this && recent_tool_result_error_log_chars(messages) >= 4_000
}

fn recent_tool_result_error_log_chars(messages: &[Message]) -> usize {
    messages
        .iter()
        .rev()
        .take(4)
        .flat_map(|message| &message.content)
        .filter_map(|block| match block {
            ContentBlock::ToolResult {
                content, is_error, ..
            } => {
                let looks_error =
                    is_error.unwrap_or(false) || content_looks_like_error_log(content);
                looks_error.then_some(content.len())
            }
            _ => None,
        })
        .sum()
}

fn content_looks_like_error_log(content: &str) -> bool {
    let lower = content.to_ascii_lowercase();
    lower.contains("error") || lower.contains("traceback") || lower.contains("failed")
}

fn is_anthropic_frontier_model(model: &str) -> bool {
    let lower = model.to_ascii_lowercase();
    lower.contains("opus") || lower.contains("fable")
}

fn is_anthropic_implement_model(model: &str) -> bool {
    model.to_ascii_lowercase().contains("sonnet")
}

fn is_openai_frontier_model(model: &str) -> bool {
    let lower = model.to_ascii_lowercase();
    lower.contains("codex") && !lower.contains("mini") && !lower.contains("nano")
}

fn is_openai_implement_model(model: &str) -> bool {
    let lower = model.to_ascii_lowercase();
    lower.starts_with("gpt") && !is_openai_frontier_model(model) && !lower.contains("nano")
}

fn provider_family_for_model_spec(model_spec: &str) -> Option<&'static str> {
    let spec = model_spec.trim().to_ascii_lowercase();
    let prefix = spec.split_once(':').map(|(prefix, _)| prefix).unwrap_or("");
    match prefix {
        "claude" | "claude-oauth" | "claude-api" | "anthropic" => Some("anthropic"),
        "openai" | "openai-oauth" | "openai-api" => Some("openai"),
        "vercel" | "vercel-ai-gateway" => Some("vercel-ai-gateway"),
        "openrouter" => Some("openrouter"),
        "gemini" => Some("gemini"),
        _ => None,
    }
}

fn is_gateway_family(provider_family: &str) -> bool {
    matches!(provider_family, "vercel-ai-gateway" | "openrouter")
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::CacheControl;
    use serde_json::json;

    fn user(text: &str) -> Message {
        Message::user(text)
    }

    fn assistant_tool(name: &str) -> Message {
        Message {
            role: Role::Assistant,
            content: vec![ContentBlock::ToolUse {
                id: format!("tool-{name}"),
                name: name.to_string(),
                input: json!({}),
                thought_signature: None,
            }],
            timestamp: None,
            tool_duration_ms: None,
        }
    }

    #[test]
    fn virtual_model_id_is_namespaced() {
        assert!(is_virtual_model(" jcode-auto "));
        assert!(!is_virtual_model("auto"));
        assert!(!is_virtual_model("claude:jcode-auto"));
    }

    #[test]
    fn classifier_truth_table_is_conservative() {
        assert_eq!(
            classify_turn(&[user("build a plan")], &[], 0),
            TurnCategory::Planning
        );
        assert_eq!(
            classify_turn(
                &[
                    user("initial"),
                    Message::assistant_text("ok"),
                    user("please review this diff")
                ],
                &[],
                1,
            ),
            TurnCategory::Planning
        );
        assert_eq!(
            classify_turn(
                &[user("initial"), assistant_tool("todo"), user("continue")],
                &[],
                1,
            ),
            TurnCategory::Planning
        );
        assert_eq!(
            classify_turn(
                &[
                    user("initial"),
                    Message::assistant_text("ok"),
                    user("run tests")
                ],
                &[],
                1,
            ),
            TurnCategory::Mechanical
        );
        assert_eq!(
            classify_turn(
                &[
                    user("initial"),
                    Message::assistant_text("ok"),
                    user("implement the parser")
                ],
                &[],
                1,
            ),
            TurnCategory::Implementation
        );
    }

    #[test]
    fn classifier_treats_tool_result_only_as_mechanical() {
        assert_eq!(
            classify_turn(&[Message::tool_result("tool-1", "ok", false)], &[], 1),
            TurnCategory::Mechanical
        );
    }

    #[test]
    fn classifier_adversarial_large_error_log_fix_this_is_unknown() {
        let huge_error = format!("error: failed\n{}", "stack frame\n".repeat(500));
        assert_eq!(
            classify_turn(
                &[
                    user("initial"),
                    Message::assistant_text("running"),
                    Message::tool_result("tool-1", &huge_error, true),
                    user("fix this"),
                ],
                &[],
                1,
            ),
            TurnCategory::Unknown
        );
    }

    #[test]
    fn fast_seed_matching_accepts_version_drift_and_rejects_unseeded_models() {
        let positives = [
            "inception/mercury-2.5",
            "zai/glm-5.3-flash",
            "glm-4.7-flash",
            "alibaba/qwen3.8-flash",
            "deepseek/deepseek-v4.1-flash",
            "deepseek-v4-flash-0731",
            "google/gemini-3.8-flash",
            "minimax/minimax-h3",
        ];
        for model in positives {
            assert!(model_matches_fast_seed(model), "{model} should match");
        }
        assert!(!model_matches_fast_seed("claude-sonnet-4-6"));
        assert!(!model_matches_fast_seed("glm-5.3"));
    }

    #[test]
    fn tier_resolution_intersects_fast_seeds_with_configured_transport() {
        let catalog = AutoTierCatalog {
            anthropic_oauth_models: vec!["claude-opus-4-8".into(), "claude-sonnet-4-6".into()],
            openai_oauth_models: vec!["gpt-5.5-codex".into(), "gpt-5.4".into()],
            vercel_ai_gateway_models: vec!["glm-5.3".into(), "zai/glm-5.3-flash".into()],
            openrouter_models: vec!["deepseek/deepseek-v4.1-flash".into()],
            subscription_flash_models: vec!["gemini-3.8-flash".into()],
        };

        let candidates = resolve_tier_candidates(
            &catalog,
            FastProviderPreference::Auto,
            &AutoRouterOverrides::default(),
        );
        assert!(
            candidates
                .iter()
                .any(|candidate| candidate.model_spec == "claude-oauth:claude-opus-4-8")
        );
        assert!(
            candidates
                .iter()
                .any(|candidate| candidate.model_spec == "claude-oauth:claude-sonnet-4-6")
        );
        assert!(
            candidates
                .iter()
                .any(|candidate| candidate.model_spec == "vercel-ai-gateway:zai/glm-5.3-flash")
        );
        assert!(
            !candidates
                .iter()
                .any(|candidate| candidate.model_spec.contains("glm-5.3")
                    && !candidate.model_spec.contains("flash"))
        );
    }

    #[test]
    fn fast_tier_absent_when_transport_unconfigured_and_mechanical_collapses_to_implement() {
        let catalog = AutoTierCatalog {
            anthropic_oauth_models: vec!["claude-sonnet-4-6".into()],
            ..AutoTierCatalog::default()
        };
        let candidates = resolve_tier_candidates(
            &catalog,
            FastProviderPreference::None,
            &AutoRouterOverrides::default(),
        );
        assert!(
            !candidates
                .iter()
                .any(|candidate| candidate.tier == AutoTier::Fast)
        );

        let mut state = AutoRouteState::new();
        let decision =
            select_candidate_for_category(&mut state, TurnCategory::Mechanical, &candidates, true)
                .expect("implement fallback");
        assert_eq!(decision.tier, AutoTier::Implement);
        assert_eq!(decision.model_spec, "claude-oauth:claude-sonnet-4-6");
    }

    #[test]
    fn stickiness_reuses_last_decision_for_same_category() {
        let candidates = vec![
            AutoModelCandidate::new(
                AutoTier::Implement,
                "claude-oauth:claude-sonnet-4-6",
                "anthropic",
                "first",
            ),
            AutoModelCandidate::new(
                AutoTier::Implement,
                "openai-oauth:gpt-5.4",
                "openai",
                "second",
            ),
        ];
        let mut state = AutoRouteState::new();
        let first = select_candidate_for_category(
            &mut state,
            TurnCategory::Implementation,
            &candidates,
            true,
        )
        .expect("first decision");
        let second = select_candidate_for_category(
            &mut state,
            TurnCategory::Implementation,
            &candidates[1..],
            true,
        )
        .expect("sticky decision");
        assert_eq!(first.model_spec, second.model_spec);
        assert_eq!(
            state.decisions.len(),
            1,
            "sticky reuse is not a new decision"
        );
    }

    #[test]
    fn category_change_honors_history_portability() {
        let candidates = vec![
            AutoModelCandidate::new(
                AutoTier::Frontier,
                "claude-oauth:claude-opus-4-8",
                "anthropic",
                "frontier",
            ),
            AutoModelCandidate::new(
                AutoTier::Fast,
                "vercel-ai-gateway:zai/glm-5.3-flash",
                "vercel-ai-gateway",
                "fast",
            ),
            AutoModelCandidate::new(
                AutoTier::Implement,
                "claude-oauth:claude-sonnet-4-6",
                "anthropic",
                "implement",
            ),
        ];
        let mut portable = AutoRouteState::new();
        let first =
            select_candidate_for_category(&mut portable, TurnCategory::Planning, &candidates, true)
                .expect("frontier");
        assert_eq!(first.provider_family, "anthropic");
        let fast = select_candidate_for_category(
            &mut portable,
            TurnCategory::Mechanical,
            &candidates,
            true,
        )
        .expect("fast allowed when portable");
        assert_eq!(fast.provider_family, "vercel-ai-gateway");

        let mut pinned = AutoRouteState::new();
        select_candidate_for_category(&mut pinned, TurnCategory::Planning, &candidates, true)
            .expect("frontier");
        let fallback = select_candidate_for_category(
            &mut pinned,
            TurnCategory::Mechanical,
            &candidates,
            false,
        )
        .expect("in-family fallback");
        assert_eq!(fallback.tier, AutoTier::Implement);
        assert_eq!(fallback.provider_family, "anthropic");
    }

    #[test]
    fn forced_tier_is_consumed_exactly_once() {
        let candidates = vec![
            AutoModelCandidate::new(
                AutoTier::Frontier,
                "claude-oauth:claude-opus-4-8",
                "anthropic",
                "frontier",
            ),
            AutoModelCandidate::new(
                AutoTier::Implement,
                "claude-oauth:claude-sonnet-4-6",
                "anthropic",
                "implement",
            ),
        ];
        let mut state = AutoRouteState::new();
        state.force_next_tier(AutoTier::Frontier);
        let forced = select_candidate_for_category(
            &mut state,
            TurnCategory::Implementation,
            &candidates,
            true,
        )
        .expect("forced frontier");
        assert_eq!(forced.tier, AutoTier::Frontier);
        assert!(state.forced_next_tier.is_none());

        let normal =
            select_candidate_for_category(&mut state, TurnCategory::Mechanical, &candidates, true)
                .expect("normal fallback");
        assert_eq!(normal.tier, AutoTier::Implement);
    }

    #[test]
    fn decision_history_is_bounded_to_last_50() {
        let mut state = AutoRouteState::new();
        for idx in 0..60 {
            state.record_decision(
                TurnCategory::Implementation,
                AutoDecision {
                    tier: AutoTier::Implement,
                    model_spec: format!("model-{idx}"),
                    provider_family: "anthropic".to_string(),
                    reason: "test".to_string(),
                    at: Instant::now(),
                },
            );
        }
        assert_eq!(state.decisions.len(), DECISION_HISTORY_LIMIT);
        assert_eq!(state.decisions.front().unwrap().model_spec, "model-10");
    }

    #[test]
    fn portability_scan_accepts_plain_history_and_rejects_signed_blocks() {
        assert!(history_is_portable(&[
            user("hello"),
            Message::assistant_text("hi")
        ]));

        let anthropic = Message {
            role: Role::Assistant,
            content: vec![ContentBlock::AnthropicThinking {
                thinking: "private".into(),
                signature: "sig".into(),
            }],
            timestamp: None,
            tool_duration_ms: None,
        };
        assert!(!history_is_portable(&[anthropic]));

        let openai = Message {
            role: Role::Assistant,
            content: vec![ContentBlock::OpenAIReasoning {
                id: "rs_1".into(),
                summary: vec![],
                encrypted_content: Some("encrypted".into()),
                status: None,
            }],
            timestamp: None,
            tool_duration_ms: None,
        };
        assert!(!history_is_portable(&[openai]));

        let signed_tool = Message {
            role: Role::Assistant,
            content: vec![ContentBlock::ToolUse {
                id: "tool-1".into(),
                name: "bash".into(),
                input: json!({}),
                thought_signature: Some("signed".into()),
            }],
            timestamp: None,
            tool_duration_ms: None,
        };
        assert!(!history_is_portable(&[signed_tool]));
    }

    #[test]
    fn context_filter_uses_metadata_and_gateway_default_only() {
        let candidates = vec![
            AutoModelCandidate::new(
                AutoTier::Fast,
                "vercel-ai-gateway:zai/glm-5.3-flash",
                "vercel-ai-gateway",
                "gateway",
            ),
            AutoModelCandidate::new(
                AutoTier::Implement,
                "local:unknown",
                "custom",
                "no metadata",
            ),
            AutoModelCandidate::new(AutoTier::Implement, "small", "custom", "small")
                .with_context_window(8_000),
            AutoModelCandidate::new(AutoTier::Implement, "large", "custom", "large")
                .with_context_window(256_000),
        ];
        let filtered = filter_candidates_by_context(&candidates, 129_000);
        let specs = filtered
            .iter()
            .map(|candidate| candidate.model_spec.as_str())
            .collect::<Vec<_>>();
        assert!(!specs.contains(&"vercel-ai-gateway:zai/glm-5.3-flash"));
        assert!(specs.contains(&"local:unknown"));
        assert!(!specs.contains(&"small"));
        assert!(specs.contains(&"large"));
    }

    #[test]
    fn overrides_skip_heuristics_for_that_tier() {
        let catalog = AutoTierCatalog {
            anthropic_oauth_models: vec!["claude-opus-4-8".into()],
            ..AutoTierCatalog::default()
        };
        let candidates = resolve_tier_candidates(
            &catalog,
            FastProviderPreference::None,
            &AutoRouterOverrides {
                frontier: Some("openai-oauth:gpt-5.5-codex".into()),
                ..AutoRouterOverrides::default()
            },
        );
        let frontier = candidates
            .iter()
            .filter(|candidate| candidate.tier == AutoTier::Frontier)
            .collect::<Vec<_>>();
        assert_eq!(frontier.len(), 1);
        assert_eq!(frontier[0].model_spec, "openai-oauth:gpt-5.5-codex");
    }

    #[test]
    fn text_with_cache_control_is_still_classified() {
        let message = Message {
            role: Role::User,
            content: vec![ContentBlock::Text {
                text: "rename symbol".into(),
                cache_control: Some(CacheControl::ephemeral(None)),
            }],
            timestamp: None,
            tool_duration_ms: None,
        };
        assert_eq!(
            classify_turn(&[user("initial"), message], &[], 1),
            TurnCategory::Mechanical
        );
    }
}
