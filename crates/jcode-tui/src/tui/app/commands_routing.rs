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
    lines.push(
        "/routing: open a role to set the default, primary model, and fallbacks · /routing edit: edit config in $EDITOR"
            .into(),
    );
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
        [role] => {
            if Config::load_strict().is_err() {
                app.push_display_message(DisplayMessage::error(
                    "Usage: /routing [show|edit|<role>]",
                ));
            } else {
                app.open_routing_role_picker(role);
            }
        }
        _ => app.push_display_message(DisplayMessage::error("Usage: /routing [show|edit|<role>]")),
    }
    true
}

fn option(name: &str, detail: &str, policy: &str) -> PickerOption {
    PickerOption {
        provider: name.into(),
        api_method: policy.into(),
        available: true,
        detail: detail.into(),
        estimated_reference_cost_micros: None,
    }
}

#[cfg(test)]
fn blank_entry(name: &str, action: PickerAction) -> PickerEntry {
    PickerEntry {
        name: name.into(),
        options: vec![],
        action,
        selected_option: 0,
        is_current: false,
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
        let mut entries = vec![PickerEntry {
            name: "No default role".into(),
            options: vec![option(
                "Explicit roles remain available",
                "Clear the default worker role",
                "Worker policy",
            )],
            action: PickerAction::RoutingDefaultRole(None),
            selected_option: 0,
            is_current: config.agents.default_role.is_none(),
            is_default: false,
            is_favorite: false,
            recommended: false,
            recommendation_rank: usize::MAX,
            usage_score: 0,
            old: false,
            created_date: None,
            effort: None,
        }];
        for (name, policy) in &config.agents.roles {
            let detail = if policy.fallbacks.is_empty() {
                "No fallbacks · ↵ open to pick models".into()
            } else {
                format!(
                    "Fallbacks: {} · ↵ open to pick models",
                    policy.fallbacks.join(" → ")
                )
            };
            entries.push(PickerEntry {
                name: name.clone(),
                options: vec![PickerOption {
                    provider: policy.model.clone(),
                    api_method: "Worker policy".into(),
                    available: true,
                    detail,
                    estimated_reference_cost_micros: None,
                }],
                action: PickerAction::RoutingRole(name.clone()),
                selected_option: 0,
                is_current: config.agents.default_role.as_ref() == Some(name),
                is_default: false,
                is_favorite: false,
                recommended: false,
                recommendation_rank: usize::MAX,
                usage_score: 0,
                old: false,
                created_date: None,
                effort: None,
            });
        }
        if config.agents.roles.is_empty() {
            self.push_display_message(DisplayMessage::system(
                "No worker roles are configured yet. Use /routing edit to add an [agents.roles.<name>] section.",
            ));
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

    pub(crate) fn open_routing_role_picker(&mut self, role: &str) {
        if super::super::commands_dispatch::ssh_local_action_blocked(
            self,
            "Worker routing settings",
        ) {
            return;
        }
        let role = role.trim();
        self.inline_interactive_state = None;
        if role.is_empty() {
            self.push_display_message(DisplayMessage::error("Usage: /routing [show|edit|<role>]"));
            return;
        }
        let config = match Config::load_strict() {
            Ok(config) => config,
            Err(error) => {
                self.push_display_message(DisplayMessage::error(format!(
                    "Worker routing configuration error: {error}\nUse /routing edit to repair it."
                )));
                return;
            }
        };
        let Some(policy) = config.agents.roles.get(role) else {
            self.push_display_message(DisplayMessage::error(format!(
                "Unknown worker role '{role}'. Use /routing to list configured roles."
            )));
            return;
        };
        let is_default = config.agents.default_role.as_deref() == Some(role);
        let fallback_summary = if policy.fallbacks.is_empty() {
            "No fallbacks".to_string()
        } else {
            policy.fallbacks.join(" → ")
        };
        let mut entries = vec![
            PickerEntry {
                name: if is_default {
                    "Default role (active)".to_string()
                } else {
                    "Set as default role".to_string()
                },
                options: vec![option(
                    if is_default {
                        "Future workers use this role"
                    } else {
                        "Route future workers through this role"
                    },
                    "Save immediately",
                    "Worker policy",
                )],
                action: PickerAction::RoutingDefaultRole(Some(role.to_string())),
                selected_option: 0,
                is_current: is_default,
                is_default: false,
                is_favorite: false,
                recommended: false,
                recommendation_rank: usize::MAX,
                usage_score: 0,
                old: false,
                created_date: None,
                effort: None,
            },
            PickerEntry {
                name: "Primary model".into(),
                options: vec![option(
                    &policy.model,
                    "Choose from the model catalog",
                    "Primary route",
                )],
                action: PickerAction::RoutingRolePrimaryModel {
                    role: role.to_string(),
                },
                selected_option: 0,
                is_current: true,
                is_default: false,
                is_favorite: false,
                recommended: false,
                recommendation_rank: usize::MAX,
                usage_score: 0,
                old: false,
                created_date: None,
                effort: None,
            },
            PickerEntry {
                name: "Toggle fallback".into(),
                options: vec![option(
                    &fallback_summary,
                    "Pick a catalog model to add or remove it",
                    "Ordered fallbacks",
                )],
                action: PickerAction::RoutingRoleFallbackToggle {
                    role: role.to_string(),
                },
                selected_option: 0,
                is_current: false,
                is_default: false,
                is_favorite: false,
                recommended: false,
                recommendation_rank: usize::MAX,
                usage_score: 0,
                old: false,
                created_date: None,
                effort: None,
            },
        ];
        if !policy.fallbacks.is_empty() {
            entries.push(PickerEntry {
                name: "Clear fallbacks".into(),
                options: vec![option(
                    &fallback_summary,
                    "Remove every fallback for this role",
                    "Ordered fallbacks",
                )],
                action: PickerAction::RoutingRoleFallbackClear {
                    role: role.to_string(),
                },
                selected_option: 0,
                is_current: false,
                is_default: false,
                is_favorite: false,
                recommended: false,
                recommendation_rank: usize::MAX,
                usage_score: 0,
                old: false,
                created_date: None,
                effort: None,
            });
        }
        self.push_display_message(DisplayMessage::system(format!(
            "Worker role: {role}\nPrimary route: {}\nOrdered fallbacks: {fallback_summary}\nPick a row to change it. Esc goes back to the role list.",
            policy.model,
        )));
        self.inline_view_state = None;
        self.inline_interactive_state = Some(InlineInteractiveState {
            kind: PickerKind::Model,
            filtered: (0..entries.len()).collect(),
            entries,
            selected: 0,
            column: 0,
            filter: String::new(),
            preview: false,
        });
        self.input.clear();
        self.cursor_pos = 0;
    }

    /// Open the shared model catalog retagged as a routing-model picker.
    ///
    /// `fallback` selects toggle-fallback mode; otherwise the choice becomes
    /// the role's primary model. The current primary/fallback membership is
    /// marked with `is_current` but never applied to the main conversation.
    pub(crate) fn open_routing_model_picker(&mut self, role: &str, fallback: bool) {
        if super::super::commands_dispatch::ssh_local_action_blocked(
            self,
            "Worker routing settings",
        ) {
            return;
        }
        let config = match Config::load_strict() {
            Ok(config) => config,
            Err(error) => {
                self.push_display_message(DisplayMessage::error(format!(
                    "Worker routing configuration error: {error}\nUse /routing edit to repair it."
                )));
                return;
            }
        };
        let Some(policy) = config.agents.roles.get(role) else {
            self.push_display_message(DisplayMessage::error(format!(
                "Unknown worker role '{role}'. Use /routing to list configured roles."
            )));
            return;
        };
        let primary = policy.model.clone();
        let fallbacks = policy.fallbacks.clone();
        let role = role.to_string();
        self.open_model_picker();
        let load_started = std::time::Instant::now();
        while self.pending_model_picker_load.is_some()
            && load_started.elapsed() < std::time::Duration::from_secs(2)
        {
            if self.poll_model_picker_load() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        let Some(picker) = self.inline_interactive_state.as_mut() else {
            self.push_display_message(DisplayMessage::error(
                "Model catalog is still loading. Reopen the role to pick a model.",
            ));
            self.open_routing_role_picker(&role);
            return;
        };
        if picker.entries.is_empty() {
            self.push_display_message(DisplayMessage::system(
                crate::tui::app::model_context::no_models_available_message(self.is_remote),
            ));
            self.open_routing_role_picker(&role);
            return;
        }
        for entry in &mut picker.entries {
            let spec = super::super::inline_interactive::helpers::model_entry_saved_spec(entry);
            entry.action = PickerAction::RoutingModelChoice {
                role: role.clone(),
                fallback,
            };
            entry.is_current = if fallback {
                fallbacks.iter().any(|item| item == &spec)
            } else {
                spec == primary
            };
            entry.is_default = false;
        }
        picker.filtered = (0..picker.entries.len()).collect();
        picker.selected = picker
            .entries
            .iter()
            .position(|entry| entry.is_current)
            .unwrap_or(0);
        picker.column = 0;
        picker.filter.clear();
        self.push_display_message(DisplayMessage::system(if fallback {
            format!(
                "Role {role}: pick a catalog model to add or remove it from fallbacks (primary stays {primary}). Esc goes back."
            )
        } else {
            format!("Role {role}: pick a catalog model as the new primary route. Esc goes back.")
        }));
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
            entries: vec![blank_entry(
                "No default role",
                PickerAction::RoutingDefaultRole(None),
            )],
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

    #[test]
    fn routing_role_detail_picker_uses_setting_schema() {
        let picker = InlineInteractiveState {
            kind: PickerKind::Model,
            entries: vec![blank_entry(
                "Primary model",
                PickerAction::RoutingRolePrimaryModel {
                    role: "implementer".to_string(),
                },
            )],
            filtered: vec![0],
            selected: 0,
            column: 0,
            filter: String::new(),
            preview: false,
        };
        assert!(picker.is_routing_picker());
        assert!(picker.is_routing_role_detail_picker());
        assert_eq!(picker.primary_label(), "SETTING");
    }

    #[test]
    fn routing_model_picker_uses_model_schema() {
        let picker = InlineInteractiveState {
            kind: PickerKind::Model,
            entries: vec![blank_entry(
                "some-model",
                PickerAction::RoutingModelChoice {
                    role: "implementer".to_string(),
                    fallback: false,
                },
            )],
            filtered: vec![0],
            selected: 0,
            column: 0,
            filter: String::new(),
            preview: false,
        };
        assert!(picker.is_routing_picker());
        assert!(picker.is_routing_model_picker());
        assert_eq!(picker.primary_label(), "MODEL");
    }
}
