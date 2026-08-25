#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::tui::app) struct PromptSuggestionState {
    session_id: Option<String>,
    generation: u64,
    suggestion: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::tui::app) enum PromptSuggestionUpdate {
    Applied,
    Ignored,
}

impl PromptSuggestionState {
    pub(in crate::tui::app) fn set(
        &mut self,
        active_session_id: Option<&str>,
        event_session_id: &str,
        generation: u64,
        suggestion: Option<String>,
    ) -> PromptSuggestionUpdate {
        if active_session_id != Some(event_session_id) || generation < self.generation {
            return PromptSuggestionUpdate::Ignored;
        }
        self.session_id = Some(event_session_id.to_string());
        self.generation = generation;
        self.suggestion = suggestion.and_then(|text| {
            let text = text.trim().to_string();
            (!text.is_empty()).then_some(text)
        });
        PromptSuggestionUpdate::Applied
    }

    pub(in crate::tui::app) fn clear(&mut self) {
        self.suggestion = None;
    }

    pub(in crate::tui::app) fn clear_for_session_change(&mut self) {
        self.session_id = None;
        self.generation = 0;
        self.suggestion = None;
    }

    pub(in crate::tui::app) fn is_compatible(
        &self,
        active_session_id: Option<&str>,
        input: &str,
    ) -> bool {
        input.is_empty()
            && self.session_id.as_deref() == active_session_id
            && self.suggestion.is_some()
    }

    pub(in crate::tui::app) fn suggestion(
        &self,
        active_session_id: Option<&str>,
        input: &str,
    ) -> Option<&str> {
        self.is_compatible(active_session_id, input)
            .then(|| self.suggestion.as_deref())
            .flatten()
    }

    pub(in crate::tui::app) fn accept(
        &mut self,
        active_session_id: Option<&str>,
        input: &str,
    ) -> Option<String> {
        if !self.is_compatible(active_session_id, input) {
            return None;
        }
        let suggestion = self.suggestion.take()?;
        Some(suggestion)
    }
}

impl crate::tui::app::App {
    pub(in crate::tui::app) fn active_prompt_suggestion_session_id(&self) -> Option<String> {
        if self.is_remote {
            self.remote_session_id.clone()
        } else {
            Some(self.session.id.clone())
        }
    }

    pub(in crate::tui::app) fn set_prompt_suggestion(
        &mut self,
        session_id: String,
        generation: u64,
        suggestion: Option<String>,
    ) -> bool {
        let active = self.active_prompt_suggestion_session_id();
        matches!(
            self.prompt_suggestion
                .set(active.as_deref(), &session_id, generation, suggestion),
            PromptSuggestionUpdate::Applied
        )
    }

    pub(in crate::tui::app) fn clear_prompt_suggestion(&mut self) {
        self.prompt_suggestion.clear();
    }

    pub(in crate::tui::app) fn clear_prompt_suggestion_for_session_change(&mut self) {
        self.prompt_suggestion.clear_for_session_change();
    }

    pub(crate) fn visible_prompt_suggestion(&self) -> Option<&str> {
        let active = self.active_prompt_suggestion_session_id();
        self.prompt_suggestion
            .suggestion(active.as_deref(), &self.input)
    }

    pub(in crate::tui::app) fn accept_prompt_suggestion(&mut self) -> bool {
        let active = self.active_prompt_suggestion_session_id();
        let Some(suggestion) = self
            .prompt_suggestion
            .accept(active.as_deref(), &self.input)
        else {
            return false;
        };
        self.remember_input_undo_state();
        self.input = suggestion;
        self.cursor_pos = self.input.len();
        self.reset_tab_completion();
        self.sync_model_picker_preview_from_input();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_only_active_session_and_newer_generations() {
        let mut state = PromptSuggestionState::default();
        assert_eq!(
            state.set(Some("a"), "b", 1, Some("no".into())),
            PromptSuggestionUpdate::Ignored
        );
        assert_eq!(state.suggestion(Some("a"), ""), None);

        assert_eq!(
            state.set(Some("a"), "a", 2, Some(" next ".into())),
            PromptSuggestionUpdate::Applied
        );
        assert_eq!(state.suggestion(Some("a"), ""), Some("next"));

        assert_eq!(
            state.set(Some("a"), "a", 1, Some("stale".into())),
            PromptSuggestionUpdate::Ignored
        );
        assert_eq!(state.suggestion(Some("a"), ""), Some("next"));
    }

    #[test]
    fn clearing_replacement_and_acceptance() {
        let mut state = PromptSuggestionState::default();
        state.set(Some("s"), "s", 1, Some("first".into()));
        assert_eq!(
            state.set(Some("s"), "s", 2, None),
            PromptSuggestionUpdate::Applied
        );
        assert_eq!(state.suggestion(Some("s"), ""), None);

        state.set(Some("s"), "s", 3, Some("second".into()));
        assert_eq!(state.suggestion(Some("s"), "typed"), None);
        assert_eq!(state.accept(Some("s"), ""), Some("second".into()));
        assert_eq!(state.suggestion(Some("s"), ""), None);
    }
}
