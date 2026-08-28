use crate::tool::ToolOutput;
use anyhow::Result;
use std::time::Duration;

const AUTOMATIC_SCAN_INTERVAL: Duration = Duration::from_secs(60 * 60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LearningInboxItem {
    pub suggestion_id: String,
    pub workflow_text: String,
    pub evidence_count: usize,
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
    Ok(
        match crate::tool::skill::discovery::refresh_automatic(interval)? {
            crate::tool::skill::discovery::AutomaticRefresh::RateLimited => {
                RefreshOutcome::RateLimited
            }
            crate::tool::skill::discovery::AutomaticRefresh::Empty => RefreshOutcome::Empty,
            crate::tool::skill::discovery::AutomaticRefresh::Unchanged(suggestion) => {
                let _ = suggestion;
                RefreshOutcome::Unchanged
            }
            crate::tool::skill::discovery::AutomaticRefresh::New(suggestion) => {
                RefreshOutcome::New(item(&suggestion))
            }
        },
    )
}

pub fn latest() -> Result<Option<LearningInboxItem>> {
    let _lock = crate::tool::skill::crystallization::acquire_operation_lock()?;
    Ok(crate::tool::skill::discovery::latest_pending()?
        .as_ref()
        .map(item))
}

pub fn mark_surfaced(suggestion_id: &str) -> Result<()> {
    let _lock = crate::tool::skill::crystallization::acquire_operation_lock()?;
    crate::tool::skill::discovery::mark_surfaced(suggestion_id)
}

pub fn inbox_output() -> Result<ToolOutput> {
    let _lock = crate::tool::skill::crystallization::acquire_operation_lock()?;
    Ok(match crate::tool::skill::discovery::latest_pending()? {
        Some(suggestion) => crate::tool::skill::discovery::suggestion_output(&suggestion, "inbox"),
        None => ToolOutput::new("Learning Inbox is empty.").with_title("Learning Inbox"),
    })
}

pub fn review_output(suggestion_id: Option<&str>) -> Result<ToolOutput> {
    let _lock = crate::tool::skill::crystallization::acquire_operation_lock()?;
    let suggestion = resolve_for_review(suggestion_id)?;
    let reviewed = crate::tool::skill::discovery::review(&suggestion.suggestion_id)?;
    Ok(crate::tool::skill::discovery::suggestion_output(
        &reviewed, "reviewed",
    ))
}

pub fn dismiss_output(suggestion_id: Option<&str>) -> Result<ToolOutput> {
    let _lock = crate::tool::skill::crystallization::acquire_operation_lock()?;
    let suggestion_id = resolve_pending_id(suggestion_id)?;
    let dismissed = crate::tool::skill::discovery::dismiss(&suggestion_id)?;
    Ok(crate::tool::skill::discovery::state_output(
        &dismissed,
        "dismissed",
    ))
}

pub fn suppress_output(suggestion_id: Option<&str>) -> Result<ToolOutput> {
    let _lock = crate::tool::skill::crystallization::acquire_operation_lock()?;
    let suggestion_id = resolve_pending_id(suggestion_id)?;
    let suppressed = crate::tool::skill::discovery::suppress(&suggestion_id)?;
    Ok(crate::tool::skill::discovery::state_output(
        &suppressed,
        "suppressed",
    ))
}

fn resolve_for_review(
    suggestion_id: Option<&str>,
) -> Result<crate::tool::skill::discovery::Suggestion> {
    if let Some(suggestion_id) = suggestion_id.filter(|value| !value.trim().is_empty()) {
        return crate::tool::skill::discovery::review(suggestion_id);
    }
    crate::tool::skill::discovery::latest_pending()?
        .ok_or_else(|| anyhow::anyhow!("Learning Inbox is empty"))
}

fn resolve_pending_id(suggestion_id: Option<&str>) -> Result<String> {
    if let Some(suggestion_id) = suggestion_id.filter(|value| !value.trim().is_empty()) {
        return Ok(crate::tool::skill::discovery::load_suggestion(suggestion_id)?.suggestion_id);
    }
    crate::tool::skill::discovery::latest_pending()?
        .map(|suggestion| suggestion.suggestion_id)
        .ok_or_else(|| anyhow::anyhow!("Learning Inbox is empty"))
}

fn item(suggestion: &crate::tool::skill::discovery::Suggestion) -> LearningInboxItem {
    LearningInboxItem {
        suggestion_id: suggestion.suggestion_id.clone(),
        workflow_text: suggestion.workflow_text.clone(),
        evidence_count: suggestion.evidence.len(),
    }
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
        assert_eq!(first.evidence_count, 3);
        assert_eq!(
            latest().unwrap().unwrap().suggestion_id,
            first.suggestion_id
        );
        assert!(matches!(
            refresh_automatic_with_interval(Duration::ZERO).unwrap(),
            RefreshOutcome::New(_)
        ));
        mark_surfaced(&first.suggestion_id).unwrap();
        assert_eq!(
            refresh_automatic_with_interval(Duration::ZERO).unwrap(),
            RefreshOutcome::Unchanged
        );
        assert_eq!(refresh_automatic().unwrap(), RefreshOutcome::RateLimited);
    }

    #[test]
    fn public_controls_dismiss_snapshot_and_suppress_pattern() {
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
        assert!(
            review_output(None)
                .unwrap()
                .output
                .contains("Draft a focused skill")
        );

        let session_path = crate::session::session_path("learning-control-a").unwrap();
        let persisted = std::fs::read_to_string(&session_path).unwrap();
        std::fs::write(
            &session_path,
            persisted.replace("root cause", "underlying cause"),
        )
        .unwrap();
        let dismissed = dismiss_output(Some(&first.suggestion_id)).unwrap();
        assert_eq!(dismissed.metadata.unwrap()["status"], "dismissed");
        assert!(latest().unwrap().is_none());

        save_session("learning-control-d", workflow);
        let RefreshOutcome::New(second) =
            refresh_automatic_with_interval(Duration::ZERO).expect("discover newer snapshot")
        else {
            panic!("expected newer evidence snapshot");
        };
        assert_ne!(second.suggestion_id, first.suggestion_id);
        let suppressed = suppress_output(None).unwrap();
        assert_eq!(suppressed.metadata.unwrap()["status"], "suppressed");
        save_session("learning-control-e", workflow);
        assert_eq!(
            refresh_automatic_with_interval(Duration::ZERO).unwrap(),
            RefreshOutcome::Empty
        );
    }

    #[test]
    fn empty_and_corrupt_inbox_state_are_safe_and_visible() {
        let _env_lock = crate::storage::lock_test_env();
        let home = TestHome::new();
        assert_eq!(inbox_output().unwrap().output, "Learning Inbox is empty.");

        let discovery_dir = home.dir.path().join("skill-crystallization/discovery");
        std::fs::create_dir_all(&discovery_dir).unwrap();
        std::fs::write(discovery_dir.join("state.json"), b"not json").unwrap();
        let error = refresh_automatic_with_interval(Duration::ZERO).unwrap_err();
        assert!(error.to_string().contains("Invalid discovery state"));
    }
}
