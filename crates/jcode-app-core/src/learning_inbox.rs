use crate::tool::ToolOutput;
use anyhow::Result;
use chrono::{DateTime, Utc};
use std::time::Duration;

const AUTOMATIC_SCAN_INTERVAL: Duration = Duration::from_secs(60 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LearningSuggestionKind {
    Workflow,
    Refine,
    Merge,
    Retire,
}

impl LearningSuggestionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Workflow => "workflow",
            Self::Refine => "refine",
            Self::Merge => "merge",
            Self::Retire => "retire",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LearningInboxItem {
    pub suggestion_id: String,
    pub suggestion_kind: LearningSuggestionKind,
    pub workflow_text: String,
    pub evidence_count: usize,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug)]
pub struct LearningInboxCommandResult {
    pub suggestion_id: Option<String>,
    pub suggestion_kind: Option<LearningSuggestionKind>,
    pub review_prompt: Option<String>,
    pub output: ToolOutput,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefreshOutcome {
    RateLimited,
    Empty,
    Unchanged,
    New(LearningInboxItem),
}

pub fn refresh_automatic() -> Result<RefreshOutcome> {
    refresh_automatic_with_interval(AUTOMATIC_SCAN_INTERVAL)
}

pub fn refresh_automatic_with_interval(interval: Duration) -> Result<RefreshOutcome> {
    let _lock = crate::tool::skill::crystallization::acquire_operation_lock()?;
    let workflow = crate::tool::skill::discovery::refresh_automatic(interval);
    let evolution = crate::tool::skill::evolution::refresh_automatic(interval);

    let mut new_items = Vec::new();
    let mut unchanged = false;
    let mut empty = false;
    let mut rate_limited = false;
    let mut errors = Vec::new();

    match workflow {
        Ok(crate::tool::skill::discovery::AutomaticRefresh::New(suggestion)) => {
            new_items.push(workflow_item(&suggestion));
        }
        Ok(crate::tool::skill::discovery::AutomaticRefresh::Unchanged(_)) => unchanged = true,
        Ok(crate::tool::skill::discovery::AutomaticRefresh::Empty) => empty = true,
        Ok(crate::tool::skill::discovery::AutomaticRefresh::RateLimited) => rate_limited = true,
        Err(error) => errors.push(format!("workflow discovery: {error}")),
    }
    match evolution {
        Ok(crate::tool::skill::evolution::AutomaticRefresh::New(suggestion)) => {
            new_items.push(evolution_item(&suggestion));
        }
        Ok(crate::tool::skill::evolution::AutomaticRefresh::Unchanged(_)) => unchanged = true,
        Ok(crate::tool::skill::evolution::AutomaticRefresh::Empty) => empty = true,
        Ok(crate::tool::skill::evolution::AutomaticRefresh::RateLimited) => rate_limited = true,
        Err(error) => errors.push(format!("skill evolution: {error}")),
    }

    if let Some(item) = newest_item(new_items) {
        return Ok(RefreshOutcome::New(item));
    }
    if unchanged {
        return Ok(RefreshOutcome::Unchanged);
    }
    if !errors.is_empty() {
        anyhow::bail!(errors.join("; "))
    }
    if empty {
        return Ok(RefreshOutcome::Empty);
    }
    if rate_limited {
        return Ok(RefreshOutcome::RateLimited);
    }
    Ok(RefreshOutcome::Empty)
}

pub fn latest() -> Result<Option<LearningInboxItem>> {
    let _lock = crate::tool::skill::crystallization::acquire_operation_lock()?;
    latest_unlocked()
}

fn latest_unlocked() -> Result<Option<LearningInboxItem>> {
    let mut items = Vec::new();
    let mut errors = Vec::new();
    match crate::tool::skill::discovery::latest_pending() {
        Ok(Some(suggestion)) => items.push(workflow_item(&suggestion)),
        Ok(None) => {}
        Err(error) => errors.push(format!("workflow discovery: {error}")),
    }
    match crate::tool::skill::evolution::latest_pending() {
        Ok(Some(suggestion)) => items.push(evolution_item(&suggestion)),
        Ok(None) => {}
        Err(error) => errors.push(format!("skill evolution: {error}")),
    }
    if let Some(item) = newest_item(items) {
        return Ok(Some(item));
    }
    if errors.is_empty() {
        Ok(None)
    } else {
        anyhow::bail!(errors.join("; "))
    }
}

pub fn mark_surfaced(suggestion_id: &str) -> Result<()> {
    let _lock = crate::tool::skill::crystallization::acquire_operation_lock()?;
    match resolve_source(suggestion_id)? {
        SuggestionSource::Workflow(_) => {
            crate::tool::skill::discovery::mark_surfaced(suggestion_id)
        }
        SuggestionSource::Evolution(_) => {
            crate::tool::skill::evolution::mark_surfaced(suggestion_id)
        }
    }
}

pub fn inbox_output() -> Result<LearningInboxCommandResult> {
    let _lock = crate::tool::skill::crystallization::acquire_operation_lock()?;
    let Some(item) = latest_unlocked()? else {
        return Ok(LearningInboxCommandResult {
            suggestion_id: None,
            suggestion_kind: None,
            review_prompt: None,
            output: ToolOutput::new("Learning Inbox is empty.").with_title("Learning Inbox"),
        });
    };
    let output = match resolve_source(&item.suggestion_id)? {
        SuggestionSource::Workflow(suggestion) => {
            crate::tool::skill::discovery::suggestion_output(&suggestion, "inbox")
        }
        SuggestionSource::Evolution(suggestion) => {
            crate::tool::skill::evolution::suggestion_output(&suggestion, "inbox")
        }
    };
    Ok(LearningInboxCommandResult {
        suggestion_id: Some(item.suggestion_id),
        suggestion_kind: Some(item.suggestion_kind),
        review_prompt: None,
        output,
    })
}

pub fn review_output(suggestion_id: Option<&str>) -> Result<LearningInboxCommandResult> {
    let _lock = crate::tool::skill::crystallization::acquire_operation_lock()?;
    let suggestion_id = resolve_pending_id(suggestion_id)?;
    match resolve_source(&suggestion_id)? {
        SuggestionSource::Workflow(_) => {
            let reviewed = crate::tool::skill::discovery::review(&suggestion_id)?;
            let prompt = format!(
                "The user chose Review for Learning Inbox suggestion {}. Call skill_manage review_crystallization for this suggestion, inspect its evidence, draft one focused global skill, and call skill_manage crystallize. Do not approve or install it. Present the pending proposal for explicit user approval.",
                reviewed.suggestion_id
            );
            Ok(LearningInboxCommandResult {
                suggestion_id: Some(reviewed.suggestion_id.clone()),
                suggestion_kind: Some(LearningSuggestionKind::Workflow),
                review_prompt: Some(prompt),
                output: crate::tool::skill::discovery::suggestion_output(&reviewed, "reviewed"),
            })
        }
        SuggestionSource::Evolution(_) => {
            let reviewed = crate::tool::skill::evolution::review(&suggestion_id)?;
            Ok(LearningInboxCommandResult {
                suggestion_id: Some(reviewed.suggestion_id.clone()),
                suggestion_kind: Some(evolution_kind(reviewed.kind)),
                review_prompt: Some(crate::tool::skill::evolution::review_prompt(&reviewed)),
                output: crate::tool::skill::evolution::suggestion_output(&reviewed, "reviewed"),
            })
        }
    }
}

pub fn dismiss_output(suggestion_id: Option<&str>) -> Result<LearningInboxCommandResult> {
    state_output(suggestion_id, false)
}

pub fn suppress_output(suggestion_id: Option<&str>) -> Result<LearningInboxCommandResult> {
    state_output(suggestion_id, true)
}

fn state_output(suggestion_id: Option<&str>, suppress: bool) -> Result<LearningInboxCommandResult> {
    let _lock = crate::tool::skill::crystallization::acquire_operation_lock()?;
    let suggestion_id = resolve_pending_id(suggestion_id)?;
    match resolve_source(&suggestion_id)? {
        SuggestionSource::Workflow(_) => {
            let suggestion = if suppress {
                crate::tool::skill::discovery::suppress(&suggestion_id)?
            } else {
                crate::tool::skill::discovery::dismiss(&suggestion_id)?
            };
            Ok(LearningInboxCommandResult {
                suggestion_id: Some(suggestion.suggestion_id.clone()),
                suggestion_kind: Some(LearningSuggestionKind::Workflow),
                review_prompt: None,
                output: crate::tool::skill::discovery::state_output(
                    &suggestion,
                    if suppress { "suppressed" } else { "dismissed" },
                ),
            })
        }
        SuggestionSource::Evolution(_) => {
            let suggestion = if suppress {
                crate::tool::skill::evolution::suppress(&suggestion_id)?
            } else {
                crate::tool::skill::evolution::dismiss(&suggestion_id)?
            };
            Ok(LearningInboxCommandResult {
                suggestion_id: Some(suggestion.suggestion_id.clone()),
                suggestion_kind: Some(evolution_kind(suggestion.kind)),
                review_prompt: None,
                output: crate::tool::skill::evolution::state_output(
                    &suggestion,
                    if suppress { "suppressed" } else { "dismissed" },
                ),
            })
        }
    }
}

fn resolve_pending_id(suggestion_id: Option<&str>) -> Result<String> {
    if let Some(suggestion_id) = suggestion_id.filter(|value| !value.trim().is_empty()) {
        resolve_source(suggestion_id)?;
        return Ok(suggestion_id.to_string());
    }
    latest_unlocked()?
        .map(|suggestion| suggestion.suggestion_id)
        .ok_or_else(|| anyhow::anyhow!("Learning Inbox is empty"))
}

enum SuggestionSource {
    Workflow(crate::tool::skill::discovery::Suggestion),
    Evolution(crate::tool::skill::evolution::Suggestion),
}

fn resolve_source(suggestion_id: &str) -> Result<SuggestionSource> {
    match crate::tool::skill::evolution::load_suggestion(suggestion_id) {
        Ok(suggestion) => return Ok(SuggestionSource::Evolution(suggestion)),
        Err(evolution_error) => match crate::tool::skill::discovery::load_suggestion(suggestion_id)
        {
            Ok(suggestion) => Ok(SuggestionSource::Workflow(suggestion)),
            Err(workflow_error) => anyhow::bail!(
                "Learning Inbox suggestion was not found (evolution: {evolution_error}; workflow: {workflow_error})"
            ),
        },
    }
}

fn workflow_item(suggestion: &crate::tool::skill::discovery::Suggestion) -> LearningInboxItem {
    LearningInboxItem {
        suggestion_id: suggestion.suggestion_id.clone(),
        suggestion_kind: LearningSuggestionKind::Workflow,
        workflow_text: suggestion.workflow_text.clone(),
        evidence_count: suggestion.evidence.len(),
        created_at: suggestion.created_at,
    }
}

fn evolution_item(suggestion: &crate::tool::skill::evolution::Suggestion) -> LearningInboxItem {
    LearningInboxItem {
        suggestion_id: suggestion.suggestion_id.clone(),
        suggestion_kind: evolution_kind(suggestion.kind),
        workflow_text: suggestion.summary.clone(),
        evidence_count: suggestion.outcome_ids.len(),
        created_at: suggestion.created_at,
    }
}

fn evolution_kind(kind: crate::tool::skill::evolution::EvolutionKind) -> LearningSuggestionKind {
    match kind {
        crate::tool::skill::evolution::EvolutionKind::Refine => LearningSuggestionKind::Refine,
        crate::tool::skill::evolution::EvolutionKind::Merge => LearningSuggestionKind::Merge,
        crate::tool::skill::evolution::EvolutionKind::Retire => LearningSuggestionKind::Retire,
    }
}

fn newest_item(items: Vec<LearningInboxItem>) -> Option<LearningInboxItem> {
    items.into_iter().max_by(|a, b| {
        a.created_at
            .cmp(&b.created_at)
            .then_with(|| b.suggestion_id.cmp(&a.suggestion_id))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{ContentBlock, Role};

    struct TestHome {
        previous: Option<std::ffi::OsString>,
        dir: tempfile::TempDir,
    }

    impl TestHome {
        fn new() -> Self {
            let dir = tempfile::tempdir().expect("temp home");
            let previous = std::env::var_os("JCODE_HOME");
            unsafe { std::env::set_var("JCODE_HOME", dir.path()) };
            Self { previous, dir }
        }
    }

    impl Drop for TestHome {
        fn drop(&mut self) {
            if let Some(previous) = self.previous.take() {
                unsafe { std::env::set_var("JCODE_HOME", previous) };
            } else {
                unsafe { std::env::remove_var("JCODE_HOME") };
            }
        }
    }

    fn save_session(id: &str, text: &str) {
        let mut session = crate::session::Session::create_with_id(id.to_string(), None, None);
        session.add_message(
            Role::User,
            vec![ContentBlock::Text {
                text: text.to_string(),
                cache_control: None,
            }],
        );
        session.save().expect("save evidence session");
    }

    #[test]
    fn automatic_refresh_surfaces_once_and_persists_rate_limit() {
        let _env_lock = crate::storage::lock_test_env();
        let _home = TestHome::new();
        let workflow = "Run the release checklist, verify each artifact, and summarize failures.";
        for suffix in ["a", "b", "c"] {
            save_session(&format!("learning-refresh-{suffix}"), workflow);
        }

        let RefreshOutcome::New(first) =
            refresh_automatic_with_interval(Duration::ZERO).expect("first refresh")
        else {
            panic!("first refresh must surface the repeated workflow");
        };
        assert_eq!(first.suggestion_kind, LearningSuggestionKind::Workflow);
        assert_eq!(first.evidence_count, 3);
        assert_eq!(
            latest().unwrap().unwrap().suggestion_id,
            first.suggestion_id
        );
        mark_surfaced(&first.suggestion_id).unwrap();
        assert_eq!(
            refresh_automatic_with_interval(Duration::ZERO).unwrap(),
            RefreshOutcome::Unchanged
        );
        assert_eq!(refresh_automatic().unwrap(), RefreshOutcome::RateLimited);
    }

    #[test]
    fn workflow_controls_return_resolved_ids_and_review_prompts() {
        let _env_lock = crate::storage::lock_test_env();
        let _home = TestHome::new();
        let workflow = "Reproduce the bug, fix its root cause, and run the public acceptance path.";
        for suffix in ["a", "b", "c"] {
            save_session(&format!("learning-control-{suffix}"), workflow);
        }
        let RefreshOutcome::New(first) =
            refresh_automatic_with_interval(Duration::ZERO).expect("discover first suggestion")
        else {
            panic!("expected first suggestion");
        };
        let reviewed = review_output(None).unwrap();
        assert_eq!(
            reviewed.suggestion_id.as_deref(),
            Some(first.suggestion_id.as_str())
        );
        assert_eq!(
            reviewed.suggestion_kind,
            Some(LearningSuggestionKind::Workflow)
        );
        assert!(
            reviewed
                .review_prompt
                .unwrap()
                .contains("skill_manage crystallize")
        );

        let dismissed = dismiss_output(None).unwrap();
        assert_eq!(
            dismissed.suggestion_id.as_deref(),
            Some(first.suggestion_id.as_str())
        );
        assert_eq!(dismissed.output.metadata.unwrap()["status"], "dismissed");
        assert!(latest().unwrap().is_none());
    }

    #[test]
    fn empty_and_corrupt_workflow_source_are_safe_and_visible() {
        let _env_lock = crate::storage::lock_test_env();
        let home = TestHome::new();
        assert_eq!(
            inbox_output().unwrap().output.output,
            "Learning Inbox is empty."
        );

        let discovery_dir = home.dir.path().join("skill-crystallization/discovery");
        std::fs::create_dir_all(&discovery_dir).unwrap();
        std::fs::write(discovery_dir.join("state.json"), b"not json").unwrap();
        let error = refresh_automatic_with_interval(Duration::ZERO).unwrap_err();
        assert!(error.to_string().contains("Invalid discovery state"));
    }
}
