use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const PROMPT_SUGGESTION_DEFAULT_MAX_CHARS: usize = 240;
pub const PROMPT_SUGGESTION_MAX_CHARS_LIMIT: usize = 2_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptSuggestionAcceptanceKey {
    Tab,
    RightArrow,
}

impl PromptSuggestionAcceptanceKey {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tab => "tab",
            Self::RightArrow => "right_arrow",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PromptSuggestionsConfig {
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(deserialize_with = "deserialize_max_chars")]
    pub max_chars: usize,
    pub acceptance_keys: Vec<PromptSuggestionAcceptanceKey>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub workspaces: BTreeMap<String, PromptSuggestionsWorkspaceOverride>,
}

impl Default for PromptSuggestionsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            model: None,
            reasoning_effort: Some("low".to_string()),
            max_chars: PROMPT_SUGGESTION_DEFAULT_MAX_CHARS,
            acceptance_keys: vec![
                PromptSuggestionAcceptanceKey::Tab,
                PromptSuggestionAcceptanceKey::RightArrow,
            ],
            workspaces: BTreeMap::new(),
        }
    }
}

impl PromptSuggestionsConfig {
    pub fn for_workspace(&self, workspace: &str) -> PromptSuggestionsResolvedConfig {
        let mut resolved = PromptSuggestionsResolvedConfig::from(self);
        let key = normalize_prompt_suggestion_workspace(workspace);
        if let Some(override_cfg) = self.workspaces.get(&key) {
            resolved.apply_override(override_cfg);
        }
        resolved
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PromptSuggestionsWorkspaceOverride {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_max_chars")]
    pub max_chars: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acceptance_keys: Option<Vec<PromptSuggestionAcceptanceKey>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptSuggestionsResolvedConfig {
    pub enabled: bool,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub max_chars: usize,
    pub acceptance_keys: Vec<PromptSuggestionAcceptanceKey>,
}

impl From<&PromptSuggestionsConfig> for PromptSuggestionsResolvedConfig {
    fn from(config: &PromptSuggestionsConfig) -> Self {
        Self {
            enabled: config.enabled,
            model: config.model.clone(),
            reasoning_effort: config.reasoning_effort.clone(),
            max_chars: config.max_chars,
            acceptance_keys: config.acceptance_keys.clone(),
        }
    }
}

impl PromptSuggestionsResolvedConfig {
    fn apply_override(&mut self, override_cfg: &PromptSuggestionsWorkspaceOverride) {
        if let Some(enabled) = override_cfg.enabled {
            self.enabled = enabled;
        }
        if let Some(model) = &override_cfg.model {
            self.model = Some(model.clone());
        }
        if let Some(reasoning_effort) = &override_cfg.reasoning_effort {
            self.reasoning_effort = Some(reasoning_effort.clone());
        }
        if let Some(max_chars) = override_cfg.max_chars {
            self.max_chars = max_chars;
        }
        if let Some(acceptance_keys) = &override_cfg.acceptance_keys {
            self.acceptance_keys = acceptance_keys.clone();
        }
    }
}

pub fn normalize_prompt_suggestion_workspace(workspace: &str) -> String {
    workspace
        .trim()
        .trim_end_matches(['/', '\\'])
        .to_ascii_lowercase()
}

fn deserialize_max_chars<'de, D>(deserializer: D) -> Result<usize, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = usize::deserialize(deserializer)?;
    Ok(clamp_max_chars(value))
}

fn deserialize_optional_max_chars<'de, D>(deserializer: D) -> Result<Option<usize>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<usize>::deserialize(deserializer).map(|value| value.map(clamp_max_chars))
}

fn clamp_max_chars(value: usize) -> usize {
    value.clamp(1, PROMPT_SUGGESTION_MAX_CHARS_LIMIT)
}
