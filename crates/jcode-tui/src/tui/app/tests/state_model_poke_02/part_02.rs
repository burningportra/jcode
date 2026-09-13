#[test]
fn test_agents_review_picker_saves_config_override() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        configure_test_remote_models(&mut app);
        app.open_agent_model_picker(crate::tui::AgentModelTarget::Review);

        let selected = app
            .inline_interactive_state
            .as_ref()
            .and_then(|picker| {
                picker.filtered.iter().position(|&idx| {
                    matches!(
                        picker.entries[idx].action,
                        crate::tui::PickerAction::AgentModelChoice {
                            target: crate::tui::AgentModelTarget::Review,
                            clear_override: false,
                        }
                    )
                })
            })
            .expect("review picker should include at least one model option");
        app.inline_interactive_state.as_mut().unwrap().selected = selected;
        let selected_model_idx = app.inline_interactive_state.as_ref().unwrap().filtered[selected];
        app.inline_interactive_state.as_mut().unwrap().entries[selected_model_idx].options[0]
            .available = true;

        let expected = {
            let picker = app.inline_interactive_state.as_ref().unwrap();
            let entry = &picker.entries[picker.filtered[selected]];
            let base = if entry.effort.is_some() {
                entry
                    .name
                    .rsplit_once(" (")
                    .map(|(base, _)| base.to_string())
                    .unwrap_or_else(|| entry.name.clone())
            } else {
                entry.name.clone()
            };
            let route = &entry.options[entry.selected_option];
            if route.api_method == "copilot" {
                format!("copilot:{}", base)
            } else if route.api_method == "cursor" {
                format!("cursor:{}", base)
            } else if route.api_method == "openai-oauth" {
                format!("openai-oauth:{}", base)
            } else if route.api_method == "openai-api" {
                format!("openai-api:{}", base)
            } else if route.api_method == "claude-oauth" {
                format!("claude-oauth:{}", base)
            } else if route.api_method == "claude-api" && route.provider == "Anthropic" {
                format!("claude-api:{}", base)
            } else if route.api_method == "bedrock" {
                format!("bedrock:{}", base)
            } else if route.api_method == "openrouter" && route.provider != "auto" {
                let catalog_model = crate::provider::openrouter_catalog_model_id(&base)
                    .unwrap_or_else(|| base.clone());
                format!("{}@{}", catalog_model, route.provider)
            } else {
                base
            }
        };

        app.handle_inline_interactive_key(KeyCode::Enter, KeyModifiers::NONE)
            .expect("save agent model override");

        let cfg = crate::config::Config::load();
        assert_eq!(cfg.autoreview.model.as_deref(), Some(expected.as_str()));
        assert!(app.inline_interactive_state.is_none());
    });
}

#[test]
fn test_model_command_suggestions_include_matching_models() {
    let mut app = create_test_app();
    configure_test_remote_models(&mut app);

    let suggestions = app.get_suggestions_for("/model g52c");
    assert_eq!(
        suggestions.first().map(|(cmd, _)| cmd.as_str()),
        Some("/model gpt-5.2-codex")
    );
}

#[test]
fn test_model_command_trailing_space_shows_model_suggestions() {
    let mut app = create_test_app();
    configure_test_remote_models(&mut app);

    let suggestions = app.get_suggestions_for("/model ");
    assert!(
        suggestions
            .iter()
            .any(|(cmd, _)| cmd == "/model gpt-5.3-codex")
    );
}

#[test]
fn test_model_command_provider_suggestions_include_openrouter_routes() {
    let mut app = create_test_app();
    configure_test_remote_openrouter_provider_routes(&mut app);

    let suggestions = app.get_suggestions_for("/model anthropic/claude-sonnet-4@");
    let commands: Vec<&str> = suggestions.iter().map(|(cmd, _)| cmd.as_str()).collect();

    assert!(commands.contains(&"/model anthropic/claude-sonnet-4@auto"));
    assert!(commands.contains(&"/model anthropic/claude-sonnet-4@Fireworks"));
    assert!(commands.contains(&"/model anthropic/claude-sonnet-4@OpenAI"));
}

#[test]
fn test_model_command_provider_suggestions_rank_matching_provider_prefix() {
    let mut app = create_test_app();
    configure_test_remote_openrouter_provider_routes(&mut app);

    let suggestions = app.get_suggestions_for("/model anthropic/claude-sonnet-4@fi");
    assert_eq!(
        suggestions.first().map(|(cmd, _)| cmd.as_str()),
        Some("/model anthropic/claude-sonnet-4@Fireworks")
    );
}

#[test]
fn test_model_command_provider_suggestions_normalize_bare_openai_model_to_openrouter_catalog_id() {
    let (app, _set_model_calls) = create_openrouter_spec_capture_test_app();

    let suggestions = app.get_suggestions_for("/model gpt-5.4@op");
    assert_eq!(
        suggestions.first().map(|(cmd, _)| cmd.as_str()),
        Some("/model openai/gpt-5.4@OpenAI")
    );
}

#[test]
fn test_model_command_provider_suggestions_include_auto_for_normalized_bare_openai_model() {
    let (app, _set_model_calls) = create_openrouter_spec_capture_test_app();

    let suggestions = app.get_suggestions_for("/model gpt-5.4@");
    let commands: Vec<&str> = suggestions.iter().map(|(cmd, _)| cmd.as_str()).collect();

    assert!(commands.contains(&"/model openai/gpt-5.4@auto"));
    assert!(commands.contains(&"/model openai/gpt-5.4@OpenAI"));
}

#[test]
fn test_remote_fallback_provider_suggestions_normalize_bare_openai_openrouter_routes() {
    with_temp_jcode_home(|| {
        let prev_api_key = std::env::var_os("OPENROUTER_API_KEY");
        crate::env::set_var("OPENROUTER_API_KEY", "test-openrouter-key");
        crate::auth::AuthStatus::invalidate_cache();

        let mut app = create_test_app();
        app.is_remote = true;
        app.remote_provider_model = Some("gpt-5.4".to_string());
        app.remote_available_entries = vec!["gpt-5.4".to_string()];
        app.remote_model_options.clear();

        let suggestions = app.get_suggestions_for("/model gpt-5.4@");
        let commands: Vec<&str> = suggestions.iter().map(|(cmd, _)| cmd.as_str()).collect();

        assert!(commands.contains(&"/model openai/gpt-5.4@auto"));
        assert!(commands.contains(&"/model openai/gpt-5.4@OpenAI"));

        if let Some(prev_api_key) = prev_api_key {
            crate::env::set_var("OPENROUTER_API_KEY", prev_api_key);
        } else {
            crate::env::remove_var("OPENROUTER_API_KEY");
        }
        crate::auth::AuthStatus::invalidate_cache();
    });
}

#[test]
fn test_login_command_suggestions_follow_provider_catalog() {
    let app = create_test_app();
    let suggestions = app.get_suggestions_for("/login ");

    for provider in crate::provider_catalog::tui_login_providers() {
        assert!(
            suggestions
                .iter()
                .any(|(cmd, detail)| cmd == &format!("/login {}", provider.id)
                    && detail == &provider.menu_detail),
            "missing /login suggestion for provider {}",
            provider.id
        );
    }
}

#[test]
fn test_model_autocomplete_completes_unique_match() {
    let mut app = create_test_app();
    configure_test_remote_models(&mut app);
    app.input = "/model g52c".to_string();
    app.cursor_pos = app.input.len();

    assert!(app.autocomplete());
    assert_eq!(app.input(), "/model gpt-5.2-codex");
}

#[test]
fn test_model_autocomplete_completes_unique_provider_match() {
    let mut app = create_test_app();
    configure_test_remote_openrouter_provider_routes(&mut app);

    app.input = "/model anthropic/claude-sonnet-4@fi".to_string();
    app.cursor_pos = app.input.len();

    assert!(app.autocomplete());
    assert_eq!(app.input(), "/model anthropic/claude-sonnet-4@Fireworks");
}

#[test]
fn test_model_picker_preview_stays_open_and_updates_filter() {
    let mut app = create_test_app();
    configure_test_remote_models(&mut app);

    for c in "/model g52c".chars() {
        app.handle_key(KeyCode::Char(c), KeyModifiers::empty())
            .unwrap();
    }

    let picker = app
        .inline_interactive_state
        .as_ref()
        .expect("model picker preview should be open");
    assert!(picker.preview);
    assert_eq!(picker.filter, "g52c");
    assert!(
        picker
            .filtered
            .iter()
            .any(|&i| picker.entries[i].name.starts_with("gpt-5.2-codex ("))
    );
    assert_eq!(app.input(), "/model g52c");
}

#[test]
fn test_model_picker_preview_enter_selects_model() {
    let mut app = create_test_app();
    configure_test_remote_models(&mut app);

    for c in "/model g52c".chars() {
        app.handle_key(KeyCode::Char(c), KeyModifiers::empty())
            .unwrap();
    }
    app.handle_key(KeyCode::Enter, KeyModifiers::empty())
        .unwrap();

    // Enter from preview mode selects the model and closes the picker
    assert!(app.inline_interactive_state.is_none());
    assert!(app.input().is_empty());
    assert_eq!(app.cursor_pos(), 0);
}

#[test]
fn routing_settings_command_select_cancel_and_preserve_main_model() {
    with_temp_jcode_home(|| {
        let mut config = crate::config::Config::default();
        config.agents = serde_json::from_value(serde_json::json!({
            "default_role": "implementer",
            "roles": {"implementer": {"model": "cerebras:qwen-3.8-27b"}}
        }))
        .unwrap();
        config.save().unwrap();
        let mut app = create_test_app();
        let model = app.provider.model();
        app.input = "/routing".into();
        app.submit_input();
        assert!(
            app.inline_interactive_state
                .as_ref()
                .unwrap()
                .is_routing_picker()
        );
        assert!(!app.is_processing);
        app.handle_inline_interactive_key(KeyCode::Char('o'), KeyModifiers::CONTROL)
            .unwrap();
        // Ctrl+O is a no-op in the routing picker (no default-model shortcut),
        // so Esc dismisses the picker entirely here.
        app.handle_inline_interactive_key(KeyCode::Esc, KeyModifiers::NONE)
            .unwrap();
        assert!(app.inline_interactive_state.is_none());
        assert_eq!(
            crate::config::Config::load_strict()
                .unwrap()
                .agents
                .default_role
                .as_deref(),
            Some("implementer")
        );
        // Enter now opens the role detail view instead of immediately saving.
        app.open_routing_picker();
        let role_pos = app
            .inline_interactive_state
            .as_ref()
            .unwrap()
            .filtered
            .iter()
            .position(|&idx| {
                matches!(
                    app.inline_interactive_state.as_ref().unwrap().entries[idx].action,
                    crate::tui::PickerAction::RoutingRole(_)
                )
            })
            .expect("routing list should include the role row");
        app.inline_interactive_state.as_mut().unwrap().selected = role_pos;
        app.handle_inline_interactive_key(KeyCode::Enter, KeyModifiers::NONE)
            .unwrap();
        assert!(
            app.inline_interactive_state
                .as_ref()
                .unwrap()
                .is_routing_role_detail_picker()
        );
        // Esc from the role view returns to the role list without saving.
        app.handle_inline_interactive_key(KeyCode::Esc, KeyModifiers::NONE)
            .unwrap();
        if app.inline_interactive_state.is_none() {
            app.open_routing_picker();
        }
        let picker = app.inline_interactive_state.as_ref().unwrap();
        assert!(picker.is_routing_role_picker());
        assert_eq!(
            crate::config::Config::load_strict()
                .unwrap()
                .agents
                .default_role
                .as_deref(),
            Some("implementer")
        );
        // The explicit no-default row still clears the default immediately.
        app.inline_interactive_state.as_mut().unwrap().selected = 0;
        app.handle_inline_interactive_key(KeyCode::Enter, KeyModifiers::NONE)
            .unwrap();
        assert!(
            crate::config::Config::load_strict()
                .unwrap()
                .agents
                .default_role
                .is_none()
        );
        assert_eq!(app.provider.model(), model);
        assert!(!app.is_processing);
    });
}

#[test]
fn routing_settings_role_view_sets_default_and_suggests_roles() {
    with_temp_jcode_home(|| {
        let mut config = crate::config::Config::default();
        config.agents = serde_json::from_value(serde_json::json!({
            "roles": {
                "implementer": {"model": "cerebras:qwen-3.8-27b"},
                "reviewer": {"model": "openai-api:gpt-6-astra"},
            }
        }))
        .unwrap();
        config.save().unwrap();
        let mut app = create_test_app();
        assert!(
            app.get_suggestions_for("/routing ")
                .iter()
                .any(|(cmd, _)| cmd == "/routing implementer")
        );
        app.input = "/routing reviewer".into();
        app.submit_input();
        assert!(
            app.inline_interactive_state
                .as_ref()
                .unwrap()
                .is_routing_role_detail_picker()
        );
        // First row sets this role as the default worker role.
        app.handle_inline_interactive_key(KeyCode::Enter, KeyModifiers::NONE)
            .unwrap();
        assert_eq!(
            crate::config::Config::load_strict()
                .unwrap()
                .agents
                .default_role
                .as_deref(),
            Some("reviewer")
        );
        // The role view stays open with the row relabeled to active.
        let picker = app.inline_interactive_state.as_ref().unwrap();
        assert!(picker.is_routing_role_detail_picker());
        assert_eq!(picker.entries[0].name, "Default role (active)");
        assert!(picker.entries[0].is_current);
        app.input = "/routing missing".into();
        app.submit_input();
        assert!(app.inline_interactive_state.is_none());
        assert!(
            app.display_messages
                .iter()
                .any(|message| message.content.contains("Unknown worker role"))
        );
    });
}

#[test]
fn routing_settings_primary_model_save_roundtrip_preserves_main_model() {
    with_temp_jcode_home(|| {
        let mut config = crate::config::Config::default();
        config.agents = serde_json::from_value(serde_json::json!({
            "roles": {"implementer": {
                "model": "cerebras:qwen-3.8-27b",
                "fallbacks": ["cerebras:gpt-oss-120b"],
            }}
        }))
        .unwrap();
        config.save().unwrap();
        let mut app = create_test_app();
        configure_test_remote_models(&mut app);
        let main_model = app.provider.model();
        app.open_routing_role_picker("implementer");
        let primary = app
            .inline_interactive_state
            .as_ref()
            .unwrap()
            .filtered
            .iter()
            .position(|&idx| {
                matches!(
                    app.inline_interactive_state.as_ref().unwrap().entries[idx].action,
                    crate::tui::PickerAction::RoutingRolePrimaryModel { .. }
                )
            })
            .expect("role view should offer primary model picking");
        app.inline_interactive_state.as_mut().unwrap().selected = primary;
        app.handle_inline_interactive_key(KeyCode::Enter, KeyModifiers::NONE)
            .unwrap();
        let picker = app.inline_interactive_state.as_ref().unwrap();
        assert!(picker.is_routing_model_picker());
        // The current primary is marked without touching the main model.
        let current = picker
            .entries
            .iter()
            .position(|entry| entry.is_current)
            .expect("catalog should mark the current primary");
        let spec = crate::tui::app::inline_interactive::helpers::model_entry_saved_spec(
            &picker.entries[current],
        );
        assert_eq!(spec, "cerebras:qwen-3.8-27b");
        assert_eq!(app.provider.model(), main_model);
        // Pick a different catalog entry as the new primary.
        let other = picker
            .filtered
            .iter()
            .position(|&idx| {
                crate::tui::app::inline_interactive::helpers::model_entry_saved_spec(
                    &picker.entries[idx],
                ) != "cerebras:qwen-3.8-27b"
            })
            .expect("catalog should offer another route");
        app.inline_interactive_state.as_mut().unwrap().selected = other;
        // Route columns navigate first; advance to the final column if needed.
        for _ in 0..3 {
            let column = app
                .inline_interactive_state
                .as_ref()
                .unwrap()
                .max_navigable_column();
            let current_column = app.inline_interactive_state.as_ref().unwrap().column;
            if current_column >= column {
                break;
            }
            app.handle_inline_interactive_key(KeyCode::Enter, KeyModifiers::NONE)
                .unwrap();
            if app
                .inline_interactive_state
                .as_ref()
                .is_none_or(|picker| picker.is_routing_role_detail_picker())
            {
                break;
            }
        }
        // If navigation is still inside the catalog, one more Enter saves.
        if app
            .inline_interactive_state
            .as_ref()
            .is_some_and(|picker| picker.is_routing_model_picker())
        {
            app.handle_inline_interactive_key(KeyCode::Enter, KeyModifiers::NONE)
                .unwrap();
        }
        let saved = crate::config::Config::load_strict().unwrap();
        let policy = &saved.agents.roles["implementer"];
        assert_ne!(policy.model, "cerebras:qwen-3.8-27b");
        assert!(!policy.fallbacks.iter().any(|item| item == &policy.model));
        assert_eq!(app.provider.model(), main_model);
        assert!(
            app.inline_interactive_state
                .as_ref()
                .unwrap()
                .is_routing_role_detail_picker()
        );
    });
}

#[test]
fn routing_settings_fallback_toggle_add_remove_roundtrip() {
    with_temp_jcode_home(|| {
        let mut config = crate::config::Config::default();
        config.agents = serde_json::from_value(serde_json::json!({
            "roles": {"implementer": {"model": "cerebras:qwen-3.8-27b"}}
        }))
        .unwrap();
        config.save().unwrap();
        let mut app = create_test_app();
        configure_test_remote_models(&mut app);
        let main_model = app.provider.model();

        // Open the fallback catalog from the role view.
        app.open_routing_role_picker("implementer");
        let toggle = app
            .inline_interactive_state
            .as_ref()
            .unwrap()
            .filtered
            .iter()
            .position(|&idx| {
                matches!(
                    app.inline_interactive_state.as_ref().unwrap().entries[idx].action,
                    crate::tui::PickerAction::RoutingRoleFallbackToggle { .. }
                )
            })
            .expect("role view should offer fallback toggling");
        app.inline_interactive_state.as_mut().unwrap().selected = toggle;
        app.handle_inline_interactive_key(KeyCode::Enter, KeyModifiers::NONE)
            .unwrap();
        assert!(
            app.inline_interactive_state
                .as_ref()
                .unwrap()
                .is_routing_model_picker()
        );

        // Choose an available catalog row that is not the primary.
        let target = app
            .inline_interactive_state
            .as_ref()
            .unwrap()
            .filtered
            .iter()
            .position(|&idx| {
                let picker = app.inline_interactive_state.as_ref().unwrap();
                let entry = &picker.entries[idx];
                entry.options.iter().any(|option| option.available)
                    && crate::tui::app::inline_interactive::helpers::model_entry_saved_spec(entry)
                        != "cerebras:qwen-3.8-27b"
            })
            .expect("fallback catalog should offer a non-primary available route");
        let target_idx = app.inline_interactive_state.as_ref().unwrap().filtered[target];
        let spec = crate::tui::app::inline_interactive::helpers::model_entry_saved_spec(
            &app.inline_interactive_state.as_ref().unwrap().entries[target_idx],
        );
        app.inline_interactive_state.as_mut().unwrap().selected = target;
        // Route columns navigate first; advance to the final column if needed.
        for _ in 0..3 {
            let column = app
                .inline_interactive_state
                .as_ref()
                .unwrap()
                .max_navigable_column();
            let current_column = app.inline_interactive_state.as_ref().unwrap().column;
            if current_column >= column {
                break;
            }
            app.handle_inline_interactive_key(KeyCode::Enter, KeyModifiers::NONE)
                .unwrap();
            if app
                .inline_interactive_state
                .as_ref()
                .is_none_or(|picker| picker.is_routing_role_detail_picker())
            {
                break;
            }
        }
        if app
            .inline_interactive_state
            .as_ref()
            .is_some_and(|picker| picker.is_routing_model_picker())
        {
            app.handle_inline_interactive_key(KeyCode::Enter, KeyModifiers::NONE)
                .unwrap();
        }
        // Toggle-on writes the fallback and returns to the role view.
        let saved = crate::config::Config::load_strict().unwrap();
        assert_eq!(
            saved.agents.roles["implementer"].fallbacks,
            vec![spec.clone()]
        );
        assert_eq!(app.provider.model(), main_model);
        assert!(
            app.inline_interactive_state
                .as_ref()
                .unwrap()
                .is_routing_role_detail_picker()
        );

        // Reopen the fallback catalog: the added spec is marked current.
        app.open_routing_role_picker("implementer");
        let toggle = app
            .inline_interactive_state
            .as_ref()
            .unwrap()
            .filtered
            .iter()
            .position(|&idx| {
                matches!(
                    app.inline_interactive_state.as_ref().unwrap().entries[idx].action,
                    crate::tui::PickerAction::RoutingRoleFallbackToggle { .. }
                )
            })
            .expect("role view should still offer fallback toggling");
        app.inline_interactive_state.as_mut().unwrap().selected = toggle;
        app.handle_inline_interactive_key(KeyCode::Enter, KeyModifiers::NONE)
            .unwrap();
        let picker = app.inline_interactive_state.as_ref().unwrap();
        assert!(picker.is_routing_model_picker());
        let marked = picker
            .entries
            .iter()
            .position(|entry| entry.is_current)
            .expect("catalog should mark the added fallback");
        assert_eq!(
            crate::tui::app::inline_interactive::helpers::model_entry_saved_spec(
                &picker.entries[marked]
            ),
            spec
        );
        // Toggle the same row off.
        let selected = picker
            .filtered
            .iter()
            .position(|&idx| idx == marked)
            .unwrap_or(0);
        app.inline_interactive_state.as_mut().unwrap().selected = selected;
        for _ in 0..3 {
            let column = app
                .inline_interactive_state
                .as_ref()
                .unwrap()
                .max_navigable_column();
            let current_column = app.inline_interactive_state.as_ref().unwrap().column;
            if current_column >= column {
                break;
            }
            app.handle_inline_interactive_key(KeyCode::Enter, KeyModifiers::NONE)
                .unwrap();
            if app
                .inline_interactive_state
                .as_ref()
                .is_none_or(|picker| picker.is_routing_role_detail_picker())
            {
                break;
            }
        }
        if app
            .inline_interactive_state
            .as_ref()
            .is_some_and(|picker| picker.is_routing_model_picker())
        {
            app.handle_inline_interactive_key(KeyCode::Enter, KeyModifiers::NONE)
                .unwrap();
        }
        let saved = crate::config::Config::load_strict().unwrap();
        assert!(saved.agents.roles["implementer"].fallbacks.is_empty());
        assert_eq!(
            saved.agents.roles["implementer"].model,
            "cerebras:qwen-3.8-27b"
        );
        assert_eq!(app.provider.model(), main_model);
        assert!(
            app.inline_interactive_state
                .as_ref()
                .unwrap()
                .is_routing_role_detail_picker()
        );
    });
}

#[test]
fn routing_settings_role_fallback_toggle_and_clear_roundtrip() {
    with_temp_jcode_home(|| {
        let mut config = crate::config::Config::default();
        config.agents = serde_json::from_value(serde_json::json!({
            "roles": {"implementer": {"model": "cerebras:qwen-3.8-27b"}}
        }))
        .unwrap();
        config.save().unwrap();
        let mut app = create_test_app();
        configure_test_remote_models(&mut app);
        app.open_routing_role_picker("implementer");
        let toggle = app
            .inline_interactive_state
            .as_ref()
            .unwrap()
            .filtered
            .iter()
            .position(|&idx| {
                matches!(
                    app.inline_interactive_state.as_ref().unwrap().entries[idx].action,
                    crate::tui::PickerAction::RoutingRoleFallbackToggle { .. }
                )
            })
            .expect("role view should offer fallback toggling");
        app.inline_interactive_state.as_mut().unwrap().selected = toggle;
        app.handle_inline_interactive_key(KeyCode::Enter, KeyModifiers::NONE)
            .unwrap();
        let picker = app.inline_interactive_state.as_ref().unwrap();
        assert!(picker.is_routing_model_picker());
        let main_model = app.provider.model();
        assert!(
            !crate::config::Config::load_strict().unwrap().agents.roles["implementer"]
                .fallbacks
                .is_empty()
                || picker.entries.iter().any(|entry| {
                    crate::tui::app::inline_interactive::helpers::model_entry_saved_spec(entry)
                        != "cerebras:qwen-3.8-27b"
                })
        );
        // Esc from the catalog returns to the role without touching the main model.
        app.handle_inline_interactive_key(KeyCode::Esc, KeyModifiers::NONE)
            .unwrap();
        assert!(
            app.inline_interactive_state
                .as_ref()
                .unwrap()
                .is_routing_role_detail_picker()
        );
        assert_eq!(app.provider.model(), main_model);
        crate::config::Config::set_agents_role_fallbacks(
            "implementer",
            &["openai-api:gpt-6-astra".to_string()],
        )
        .unwrap();
        app.open_routing_role_picker("implementer");
        let clear = app
            .inline_interactive_state
            .as_ref()
            .unwrap()
            .filtered
            .iter()
            .position(|&idx| {
                matches!(
                    app.inline_interactive_state.as_ref().unwrap().entries[idx].action,
                    crate::tui::PickerAction::RoutingRoleFallbackClear { .. }
                )
            })
            .expect("role view should offer clearing fallbacks");
        app.inline_interactive_state.as_mut().unwrap().selected = clear;
        app.handle_inline_interactive_key(KeyCode::Enter, KeyModifiers::NONE)
            .unwrap();
        assert!(
            crate::config::Config::load_strict().unwrap().agents.roles["implementer"]
                .fallbacks
                .is_empty()
        );
        assert!(
            app.inline_interactive_state.is_none()
                || app
                    .inline_interactive_state
                    .as_ref()
                    .unwrap()
                    .is_routing_role_detail_picker()
        );
        assert_eq!(app.provider.model(), main_model);
    });
}

#[test]
fn routing_settings_completion_help_and_invalid_config_are_local() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        assert!(
            app.get_suggestions_for("/rout")
                .iter()
                .any(|(cmd, _)| cmd == "/routing")
        );
        assert!(
            app.get_suggestions_for("/routing e")
                .iter()
                .any(|(cmd, _)| cmd == "/routing edit")
        );
        assert!(App::command_accepts_args("/routing"));
        assert!(
            app.command_help("routing")
                .unwrap()
                .contains("/routing <role>")
        );
        let path = crate::config::Config::path().unwrap();
        let malformed = "[agents]\ndefault_role = [";
        std::fs::write(&path, malformed).unwrap();
        app.input = "/routing".into();
        app.submit_input();
        assert!(app.inline_interactive_state.is_none());
        assert!(!app.is_processing);
        assert!(
            app.display_messages
                .iter()
                .any(|message| message.content.contains("configuration error"))
        );
        app.input = "/routing invalid".into();
        app.submit_input();
        assert!(!app.is_processing);
        assert!(
            app.display_messages.iter().any(|message| {
                message.content.contains("Usage: /routing")
                    || message.content.contains("Unknown worker role")
            }),
            "expected routing usage or unknown-role error"
        );
        assert_eq!(std::fs::read_to_string(path).unwrap(), malformed);
    });
}
