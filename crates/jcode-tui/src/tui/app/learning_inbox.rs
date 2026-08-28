use super::App;
use crate::bus::{Bus, BusEvent, LearningInboxCommandCompleted, LearningInboxUpdated};
use crate::tui::DisplayMessage;

impl App {
    pub fn set_learning_inbox_store_is_local(&mut self, is_local: bool) {
        self.learning_inbox_store_is_local = is_local;
    }

    pub(super) fn refresh_learning_inbox_after_turn(&self) {
        if !self.learning_inbox_store_is_local {
            return;
        }
        let session_id = self.session.id.clone();
        tokio::spawn(async move {
            let result =
                tokio::task::spawn_blocking(crate::learning_inbox::refresh_automatic).await;
            let update = match result {
                Ok(Ok(crate::learning_inbox::RefreshOutcome::New(item))) => LearningInboxUpdated {
                    session_id,
                    suggestion_id: Some(item.suggestion_id),
                    workflow_text: Some(item.workflow_text),
                    evidence_count: item.evidence_count,
                    error: None,
                },
                Ok(Ok(_)) => return,
                Ok(Err(error)) => LearningInboxUpdated {
                    session_id,
                    suggestion_id: None,
                    workflow_text: None,
                    evidence_count: 0,
                    error: Some(error.to_string()),
                },
                Err(error) => LearningInboxUpdated {
                    session_id,
                    suggestion_id: None,
                    workflow_text: None,
                    evidence_count: 0,
                    error: Some(error.to_string()),
                },
            };
            Bus::global().publish(BusEvent::LearningInboxUpdated(update));
        });
    }

    pub(super) fn handle_learning_inbox_updated(&mut self, update: LearningInboxUpdated) -> bool {
        if update.session_id != self.session.id {
            return false;
        }
        if let Some(error) = update.error {
            crate::logging::warn(&format!("Learning Inbox refresh failed: {error}"));
            return false;
        }
        let Some(workflow) = update.workflow_text else {
            return false;
        };
        let workflow = compact_workflow(&workflow, 160);
        self.push_display_message(DisplayMessage::system(format!(
            "Learning Inbox: repeated workflow found in {} sessions: {}\n\nRun `/learning` to Review, Dismiss, or Never suggest this.",
            update.evidence_count, workflow
        )));
        if let Some(suggestion_id) = update.suggestion_id
            && let Err(error) = crate::learning_inbox::mark_surfaced(&suggestion_id)
        {
            crate::logging::warn(&format!("Learning Inbox acknowledgement failed: {error}"));
        }
        self.set_status_notice("Learning Inbox: 1 suggestion · /learning");
        true
    }

    pub(super) fn handle_learning_inbox_command_completed(
        &mut self,
        update: LearningInboxCommandCompleted,
    ) -> bool {
        if update.session_id != self.session.id {
            return false;
        }
        if let Some(error) = update.error {
            self.push_display_message(DisplayMessage::error(format!("Learning Inbox: {error}")));
            self.set_status_notice("Learning Inbox: action failed");
            return true;
        }
        if let Some(output) = update.output {
            self.push_display_message(DisplayMessage::system(output));
        }
        if update.action == "review"
            && let Some(suggestion_id) = update.suggestion_id
        {
            self.queued_messages.push(format!(
                "The user chose Review for Learning Inbox suggestion {suggestion_id}. Call skill_manage review_crystallization for this suggestion, inspect its evidence, draft one focused global skill, and call skill_manage crystallize. Do not approve or install it. Present the pending proposal for explicit user approval."
            ));
            self.pending_queued_dispatch = true;
            self.set_status_notice("Learning Inbox: drafting proposal");
        } else {
            self.set_status_notice(match update.action.as_str() {
                "dismiss" => "Learning Inbox: dismissed",
                "never" => "Learning Inbox: suppressed",
                _ => "Learning Inbox",
            });
        }
        true
    }
}

fn compact_workflow(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let compact = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{compact}…")
    } else {
        compact
    }
}

pub(super) fn handle_learning_command(app: &mut App, trimmed: &str) -> bool {
    let Some(rest) = trimmed.strip_prefix("/learning") else {
        return false;
    };
    if !rest.is_empty() && !rest.chars().next().is_some_and(char::is_whitespace) {
        return false;
    }
    if !app.learning_inbox_store_is_local {
        app.push_display_message(DisplayMessage::error(
            "Learning Inbox is not available in remote TUI sessions yet because its evidence lives on the server."
                .to_string(),
        ));
        return true;
    }
    let mut parts = rest.split_whitespace();
    let action = parts.next().unwrap_or("inbox");
    let suggestion_id = parts.next();
    if parts.next().is_some() {
        app.push_display_message(DisplayMessage::error(
            "Usage: /learning [review|dismiss|never] [suggestion-id]".to_string(),
        ));
        return true;
    }
    let action = match action {
        "inbox" | "list" => "inbox",
        "review" => "review",
        "dismiss" => "dismiss",
        "never" | "suppress" => "never",
        _ => {
            app.push_display_message(DisplayMessage::error(
                "Usage: /learning [review|dismiss|never] [suggestion-id]".to_string(),
            ));
            return true;
        }
    };
    let session_id = app.session.id.clone();
    let suggestion_id = suggestion_id.map(str::to_string);
    let action = action.to_string();
    app.set_status_notice(format!("Learning Inbox: {action}…"));
    tokio::spawn(async move {
        let worker_action = action.clone();
        let worker_id = suggestion_id.clone();
        let result = tokio::task::spawn_blocking(move || match worker_action.as_str() {
            "inbox" => crate::learning_inbox::inbox_output(),
            "review" => crate::learning_inbox::review_output(worker_id.as_deref()),
            "dismiss" => crate::learning_inbox::dismiss_output(worker_id.as_deref()),
            "never" => crate::learning_inbox::suppress_output(worker_id.as_deref()),
            _ => unreachable!("validated Learning Inbox action"),
        })
        .await;
        let update = match result {
            Ok(Ok(output)) => LearningInboxCommandCompleted {
                session_id,
                action,
                suggestion_id,
                output: Some(output.output),
                error: None,
            },
            Ok(Err(error)) => LearningInboxCommandCompleted {
                session_id,
                action,
                suggestion_id,
                output: None,
                error: Some(error.to_string()),
            },
            Err(error) => LearningInboxCommandCompleted {
                session_id,
                action,
                suggestion_id,
                output: None,
                error: Some(error.to_string()),
            },
        };
        Bus::global().publish(BusEvent::LearningInboxCommandCompleted(update));
    });
    true
}

#[cfg(test)]
mod tests {
    use super::compact_workflow;

    #[test]
    fn compact_notice_is_unicode_safe_and_bounded() {
        assert_eq!(compact_workflow("short workflow", 20), "short workflow");
        assert_eq!(compact_workflow("🧪 verify artifacts", 8), "🧪 verify…");
    }
}
