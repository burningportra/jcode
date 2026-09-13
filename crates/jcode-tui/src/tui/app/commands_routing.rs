use super::{App, DisplayMessage};
use crate::config::Config;
use crate::tui::{InlineInteractiveState, PickerAction, PickerEntry, PickerKind, PickerOption};

pub(crate) fn summary() -> String {
    match Config::load_strict() {
        Ok(config) => describe(&config),
        Err(error) => {
            format!("Worker routing: configuration error\n{error}\nUse /routing edit to repair it.")
        }
    }
}

fn describe(config: &Config) -> String {
    let mut lines = vec!["Worker routing".to_string()];
    match config.agents.default_role.as_deref() {
        Some(role) => {
            lines.push(format!("Active default role: {role}"));
            if let Some(policy) = config.agents.roles.get(role) {
                lines.push(format!("Primary route: {}", policy.model));
                lines.push(format!(
                    "Ordered fallbacks: {}",
                    if policy.fallbacks.is_empty() {
                        "none".into()
                    } else {
                        policy.fallbacks.join(" → ")
                    }
                ));
            }
        }
        None => lines.push("No default role. Explicit named roles remain available.".into()),
    }
    lines.push(
        "Applies to future worker dispatch. Main conversation and existing workers are unchanged."
            .into(),
    );
    lines.push("/routing: choose default role · /routing edit: edit models and fallbacks".into());
    lines.join("\n")
}

pub(super) fn handle(app: &mut App, command: &str) -> bool {
    let mut words = command.split_whitespace();
    if words.next() != Some("/routing") {
        return false;
    }
    if super::super::commands_dispatch::ssh_local_action_blocked(app, "Worker routing settings") {
        return true;
    }
    let args: Vec<_> = words.collect();
    match args.as_slice() {
        [] => app.open_routing_picker(),
        ["show"] => app.push_display_message(DisplayMessage::system(summary())),
        ["edit"] => {
            let result = (|| -> anyhow::Result<()> {
                let path = Config::path().ok_or_else(|| anyhow::anyhow!("No config path"))?;
                if !path.exists() {
                    Config::create_default_config_file()?;
                }
                let editor = std::env::var("VISUAL")
                    .or_else(|_| std::env::var("EDITOR"))
                    .unwrap_or_else(|_| "nano".into());
                let mut parts = editor.split_whitespace();
                let bin = parts
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("$VISUAL/$EDITOR is empty"))?;
                let status = super::run_interactive_editor(
                    std::process::Command::new(bin).args(parts).arg(path),
                )?;
                if !status.success() {
                    anyhow::bail!("Editor exited with {status}");
                }
                Config::invalidate_cache();
                Config::load_strict()?;
                Ok(())
            })();
            match result {
                Ok(()) => app.open_routing_picker(),
                Err(error) => app.push_display_message(DisplayMessage::error(format!(
                    "Routing settings: {error}"
                ))),
            }
        }
        _ => app.push_display_message(DisplayMessage::error("Usage: /routing [show|edit]")),
    }
    true
}

impl App {
    pub(crate) fn open_routing_picker(&mut self) {
        if super::super::commands_dispatch::ssh_local_action_blocked(
            self,
            "Worker routing settings",
        ) {
            return;
        }
        self.inline_interactive_state = None;
        let config = match Config::load_strict() {
            Ok(config) => config,
            Err(error) => {
                self.push_display_message(DisplayMessage::error(format!(
                    "Worker routing configuration error: {error}\nUse /routing edit to repair it."
                )));
                return;
            }
        };
        self.push_display_message(DisplayMessage::system(describe(&config)));
        let mut entries = vec![entry(
            None,
            "No default role",
            "Explicit roles remain available",
            config.agents.default_role.is_none(),
        )];
        for (name, policy) in &config.agents.roles {
            let detail = if policy.fallbacks.is_empty() {
                "No fallbacks".into()
            } else {
                format!("Fallbacks: {}", policy.fallbacks.join(" → "))
            };
            let mut row = entry(
                Some(name.clone()),
                name,
                &policy.model,
                config.agents.default_role.as_ref() == Some(name),
            );
            row.options[0].detail = detail;
            entries.push(row);
        }
        let selected = entries
            .iter()
            .position(|entry| entry.is_current)
            .unwrap_or(0);
        self.inline_view_state = None;
        self.inline_interactive_state = Some(InlineInteractiveState {
            kind: PickerKind::Model,
            filtered: (0..entries.len()).collect(),
            entries,
            selected,
            column: 0,
            filter: String::new(),
            preview: false,
        });
        self.input.clear();
        self.cursor_pos = 0;
    }
}

fn entry(role: Option<String>, name: &str, model: &str, current: bool) -> PickerEntry {
    PickerEntry {
        name: name.into(),
        options: vec![PickerOption {
            provider: model.into(),
            api_method: "Worker policy".into(),
            available: true,
            detail: String::new(),
            estimated_reference_cost_micros: None,
        }],
        action: PickerAction::RoutingDefaultRole(role),
        selected_option: 0,
        is_current: current,
        is_default: false,
        is_favorite: false,
        recommended: false,
        recommendation_rank: usize::MAX,
        usage_score: 0,
        old: false,
        created_date: None,
        effort: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routing_settings_summary_distinguishes_active_and_no_default() {
        let mut config = Config::default();
        assert!(describe(&config).contains("Explicit named roles remain available"));
        config.agents = serde_json::from_value(serde_json::json!({"default_role":"implementer","roles":{"implementer":{"model":"cerebras:qwen-3.8-27b","fallbacks":["cerebras:gpt-oss-120b"]}}})).unwrap();
        let text = describe(&config);
        for expected in [
            "Active default role: implementer",
            "cerebras:qwen-3.8-27b",
            "cerebras:gpt-oss-120b",
            "Main conversation and existing workers are unchanged",
        ] {
            assert!(text.contains(expected), "{text}");
        }
    }

    #[test]
    fn routing_settings_picker_has_role_labels_and_no_model_shortcuts() {
        let picker = InlineInteractiveState {
            kind: PickerKind::Model,
            entries: vec![entry(None, "No default role", "", true)],
            filtered: vec![0],
            selected: 0,
            column: 0,
            filter: String::new(),
            preview: false,
        };
        assert_eq!(picker.primary_label(), "ROLE");
        assert!(!picker.shows_default_shortcut_hint());
        assert!(!picker.uses_compact_navigation());
    }
}
