#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PromptSuggestionEligibility {
    pub config_enabled: bool,
    pub interactive: bool,
    pub successful_turn: bool,
    pub headless: bool,
    pub scripted: bool,
    pub debug: bool,
}

impl PromptSuggestionEligibility {
    pub fn is_eligible(self) -> bool {
        self.config_enabled
            && self.interactive
            && self.successful_turn
            && !self.headless
            && !self.scripted
            && !self.debug
    }
}

pub fn normalize_prompt_suggestion_output(output: &str, max_chars: usize) -> Option<String> {
    let trimmed = output.trim();
    if trimmed.is_empty() || is_no_prompt_suggestion_sentinel(trimmed) {
        return None;
    }
    Some(truncate_utf8_chars(trimmed, max_chars))
}

pub fn is_no_prompt_suggestion_sentinel(output: &str) -> bool {
    matches!(
        output.trim().to_ascii_lowercase().as_str(),
        "none" | "no suggestion" | "no_suggestion" | "<no suggestion>" | "[no suggestion]"
    )
}

pub fn truncate_utf8_chars(input: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    input.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_empty_and_sentinel_to_none() {
        assert_eq!(normalize_prompt_suggestion_output("  ", 20), None);
        assert_eq!(
            normalize_prompt_suggestion_output("[no suggestion]", 20),
            None
        );
    }

    #[test]
    fn truncates_on_utf8_character_boundaries() {
        assert_eq!(truncate_utf8_chars("aé🦀z", 3), "aé🦀");
        assert_eq!(
            normalize_prompt_suggestion_output("  aé🦀z  ", 2).as_deref(),
            Some("aé")
        );
    }

    #[test]
    fn eligibility_matrix_excludes_unsafe_contexts() {
        let base = PromptSuggestionEligibility {
            config_enabled: true,
            interactive: true,
            successful_turn: true,
            headless: false,
            scripted: false,
            debug: false,
        };
        assert!(base.is_eligible());
        assert!(
            !PromptSuggestionEligibility {
                config_enabled: false,
                ..base
            }
            .is_eligible()
        );
        assert!(
            !PromptSuggestionEligibility {
                interactive: false,
                ..base
            }
            .is_eligible()
        );
        assert!(
            !PromptSuggestionEligibility {
                successful_turn: false,
                ..base
            }
            .is_eligible()
        );
        assert!(
            !PromptSuggestionEligibility {
                headless: true,
                ..base
            }
            .is_eligible()
        );
        assert!(
            !PromptSuggestionEligibility {
                scripted: true,
                ..base
            }
            .is_eligible()
        );
        assert!(
            !PromptSuggestionEligibility {
                debug: true,
                ..base
            }
            .is_eligible()
        );
    }
}
