#[derive(Clone)]
struct AutoRouterTestProvider {
    tier_calls: StdArc<StdMutex<Vec<Option<String>>>>,
    route_calls: StdArc<StdMutex<Vec<crate::provider::RouteSelection>>>,
    model: StdArc<StdMutex<String>>,
    resolved_model: Option<&'static str>,
}

impl AutoRouterTestProvider {
    fn new(
        model: &'static str,
        resolved_model: Option<&'static str>,
    ) -> (
        Self,
        StdArc<StdMutex<Vec<Option<String>>>>,
        StdArc<StdMutex<Vec<crate::provider::RouteSelection>>>,
    ) {
        let tier_calls = StdArc::new(StdMutex::new(Vec::new()));
        let route_calls = StdArc::new(StdMutex::new(Vec::new()));
        (
            Self {
                tier_calls: tier_calls.clone(),
                route_calls: route_calls.clone(),
                model: StdArc::new(StdMutex::new(model.to_string())),
                resolved_model,
            },
            tier_calls,
            route_calls,
        )
    }
}

#[async_trait::async_trait]
impl Provider for AutoRouterTestProvider {
    async fn complete(
        &self,
        _messages: &[Message],
        _tools: &[crate::message::ToolDefinition],
        _system: &str,
        _resume_session_id: Option<&str>,
    ) -> Result<crate::provider::EventStream> {
        unimplemented!("AutoRouterTestProvider")
    }

    fn name(&self) -> &str {
        "auto-test"
    }

    fn display_name(&self) -> String {
        "jcode".to_string()
    }

    fn model(&self) -> String {
        self.model.lock().unwrap().clone()
    }

    fn auto_last_resolved_model(&self) -> Option<String> {
        self.resolved_model.map(str::to_string)
    }

    fn auto_state_snapshot(&self) -> Option<jcode_provider_core::AutoRouterStateSnapshot> {
        Some(jcode_provider_core::AutoRouterStateSnapshot {
            active: self.model.lock().unwrap().as_str() == "jcode-auto",
            last_resolved: self.resolved_model.map(|model| {
                jcode_provider_core::AutoRouterDecisionSnapshot {
                    tier: "implement".to_string(),
                    model_spec: model.to_string(),
                    provider_family: "anthropic".to_string(),
                    reason: "implementation turn".to_string(),
                }
            }),
            decisions_tail: self.resolved_model.map_or_else(Vec::new, |model| {
                vec![jcode_provider_core::AutoRouterDecisionSnapshot {
                    tier: "implement".to_string(),
                    model_spec: model.to_string(),
                    provider_family: "anthropic".to_string(),
                    reason: "implementation turn".to_string(),
                }]
            }),
        })
    }

    fn set_auto_tier(&self, tier: Option<&str>) -> Result<()> {
        self.tier_calls
            .lock()
            .unwrap()
            .push(tier.map(str::to_string));
        Ok(())
    }

    fn set_route_selection(&self, selection: &crate::provider::RouteSelection) -> Result<()> {
        self.route_calls.lock().unwrap().push(selection.clone());
        Ok(())
    }

    fn set_model(&self, model: &str) -> Result<()> {
        if model == "jcode-auto" {
            *self.model.lock().unwrap() = model.to_string();
            Ok(())
        } else {
            Err(anyhow::anyhow!("unexpected model {model}"))
        }
    }

    fn available_models_display(&self) -> Vec<String> {
        vec!["jcode-auto".to_string()]
    }

    fn model_routes(&self) -> Vec<crate::provider::ModelRoute> {
        vec![crate::provider::ModelRoute {
            model: "jcode-auto".to_string(),
            provider: "jcode".to_string(),
            api_method: "auto".to_string(),
            available: true,
            detail: "frontier: claude-opus · implement: claude-sonnet · fast: glm-flash".to_string(),
            cheapness: None,
        }]
    }

    fn fork(&self) -> Arc<dyn Provider> {
        Arc::new(self.clone())
    }
}

fn create_auto_router_test_app(
    model: &'static str,
    resolved_model: Option<&'static str>,
) -> (
    App,
    StdArc<StdMutex<Vec<Option<String>>>>,
    StdArc<StdMutex<Vec<crate::provider::RouteSelection>>>,
) {
    ensure_test_jcode_home_if_unset();
    clear_persisted_test_ui_state();
    crate::tui::ui::clear_test_render_state_for_tests_unlocked();
    let (provider, tier_calls, route_calls) = AutoRouterTestProvider::new(model, resolved_model);
    let provider: Arc<dyn Provider> = Arc::new(provider);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let registry = rt.block_on(crate::tool::Registry::new(provider.clone()));
    let mut app = App::new_for_test_harness(provider, registry);
    app.queue_mode = false;
    app.diff_mode = crate::config::DiffDisplayMode::Inline;
    (app, tier_calls, route_calls)
}

#[test]
fn auto_command_registered_and_suggests_tiers() {
    let app = create_test_app();
    let registered = app.get_suggestions_for("/au");
    assert!(registered.iter().any(|(cmd, _)| cmd == "/auto"));

    let exact = app.get_suggestions_for("/auto");
    assert!(exact.iter().any(|(cmd, _)| cmd == "/auto"));
    assert!(exact.iter().any(|(cmd, _)| cmd == "/auto frontier"));
    assert!(exact.iter().any(|(cmd, _)| cmd == "/auto implement"));
    assert!(exact.iter().any(|(cmd, _)| cmd == "/auto fast"));

    let forced = app.get_suggestions_for("/auto f");
    assert_eq!(forced.first().map(|(cmd, _)| cmd.as_str()), Some("/auto fast"));
}

#[test]
fn auto_command_audit_renders_last_decision() {
    let (mut app, _, _) = create_auto_router_test_app("jcode-auto", Some("claude-sonnet-4"));

    assert!(crate::tui::app::model_context::handle_auto_command(&mut app, "/auto"));

    let message = app.display_messages.last().expect("audit message");
    assert_eq!(message.role, "system");
    assert!(message.content.contains("Auto router: active"), "{}", message.content);
    assert!(message.content.contains("implement -> claude-sonnet-4"), "{}", message.content);
    assert!(message.content.contains("implementation turn"), "{}", message.content);
}

#[test]
fn auto_command_forces_local_tier_once() {
    let (mut app, tier_calls, _) = create_auto_router_test_app("jcode-auto", Some("claude-sonnet-4"));

    assert!(crate::tui::app::model_context::handle_auto_command(&mut app, "/auto fast"));

    assert_eq!(
        tier_calls.lock().unwrap().as_slice(),
        &[Some("fast".to_string())]
    );
    assert_eq!(app.session.model.as_deref(), Some("jcode-auto"));
    assert!(app.display_messages.last().unwrap().content.contains("exactly one next turn"));
}

#[test]
fn model_command_selects_jcode_auto() {
    let (mut app, _, _) = create_auto_router_test_app("gpt-5.5", None);

    assert!(crate::tui::app::model_context::handle_model_command(&mut app, "/model jcode-auto"));

    assert_eq!(app.session.model.as_deref(), Some("jcode-auto"));
    assert!(app
        .display_messages
        .iter()
        .any(|message| message.role == "system" && message.content.contains("jcode-auto")));
}

#[test]
fn remote_auto_command_sends_set_auto_tier() {
    use crossterm::event::{KeyEvent, KeyEventKind};
    use tokio::io::AsyncBufReadExt;

    let mut app = create_test_app();
    app.is_remote = true;
    app.set_input_for_test("/auto frontier");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();

    let line = rt.block_on(async {
        let mut remote = crate::tui::backend::RemoteConnection::dummy();
        remote.mark_history_loaded();
        let peer = remote.take_dummy_peer().expect("dummy peer");
        let (reader, _writer) = peer.into_split();
        let mut reader = tokio::io::BufReader::new(reader);
        remote::handle_remote_key_event(
            &mut app,
            KeyEvent::new_with_kind(KeyCode::Enter, KeyModifiers::NONE, KeyEventKind::Press),
            &mut remote,
        )
        .await
        .expect("remote enter");

        let mut line = String::new();
        reader.read_line(&mut line).await.expect("read request");
        line
    });

    match serde_json::from_str::<crate::protocol::Request>(&line).expect("request json") {
        crate::protocol::Request::SetAutoTier { tier, .. } => {
            assert_eq!(tier.as_deref(), Some("frontier"));
        }
        other => panic!("expected SetAutoTier request, got {other:?}"),
    }
}

#[test]
fn remote_state_event_updates_auto_status_model() {
    let mut app = create_test_app();
    app.is_remote = true;
    app.remote_provider_model = Some("jcode-auto".to_string());
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();
    let mut remote = crate::tui::backend::RemoteConnection::dummy();

    app.handle_server_event(
        crate::protocol::ServerEvent::State {
            id: 1,
            session_id: app.session.id.clone(),
            message_count: 0,
            is_processing: false,
            resolved_model: Some("claude-sonnet-4".to_string()),
            auto_state: Some(jcode_provider_core::AutoRouterStateSnapshot {
                active: true,
                last_resolved: Some(jcode_provider_core::AutoRouterDecisionSnapshot {
                    tier: "implement".to_string(),
                    model_spec: "claude-sonnet-4".to_string(),
                    provider_family: "anthropic".to_string(),
                    reason: "implementation turn".to_string(),
                }),
                decisions_tail: Vec::new(),
            }),
        },
        &mut remote,
    );

    let data = crate::tui::TuiState::info_widget_data(&app);
    assert_eq!(data.model.as_deref(), Some("jcode-auto"));
    assert_eq!(data.resolved_model.as_deref(), Some("claude-sonnet-4"));
}

#[test]
fn auto_model_picker_selection_round_trips_route_identity() {
    let (mut app, _, route_calls) = create_auto_router_test_app("gpt-5.5", None);

    app.open_model_picker();
    wait_for_model_picker_load(&mut app);
    {
        let picker = app.inline_interactive_state.as_mut().expect("model picker");
        let idx = picker
            .entries
            .iter()
            .position(|entry| entry.name == "jcode-auto")
            .expect("jcode-auto picker entry");
        picker.selected = picker.filtered.iter().position(|entry_idx| *entry_idx == idx).unwrap_or(0);
    }
    app.handle_inline_interactive_key(KeyCode::Enter, KeyModifiers::NONE)
        .expect("select auto route");

    let calls = route_calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].model, "jcode-auto");
    assert_eq!(calls[0].api_method, "auto");
    assert_eq!(calls[0].runtime_key, crate::provider::RuntimeKey::Auto);
    assert_eq!(calls[0].routed_model_spec(), "jcode-auto");
}

#[test]
fn auto_routing_status_detail_renders_notice_remotely() {
    let mut app = create_test_app();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();
    let mut remote = crate::tui::backend::RemoteConnection::dummy();

    let redraw = app.handle_server_event(
        crate::protocol::ServerEvent::StatusDetail {
            detail: "auto: sonnet -> glm-5.3-flash (mechanical)".to_string(),
        },
        &mut remote,
    );

    assert!(redraw);
    assert!(app.display_messages.iter().any(|message| {
        message.role == "system" && message.content.contains("auto: sonnet -> glm-5.3-flash")
    }));
}
