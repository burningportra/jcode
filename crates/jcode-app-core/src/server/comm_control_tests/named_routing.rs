#[tokio::test]
async fn named_role_assignment_and_removed_worker_recovery_reject_mismatches_without_dispatch() {
    let (_env, runtime) = RuntimeEnvGuard::new();
    struct HomeGuard(Option<std::ffi::OsString>);
    impl Drop for HomeGuard {
        fn drop(&mut self) {
            if let Some(value) = self.0.take() { crate::env::set_var("JCODE_HOME", value); }
            else { crate::env::remove_var("JCODE_HOME"); }
        }
    }
    let _home = HomeGuard(std::env::var_os("JCODE_HOME"));
    crate::env::set_var("JCODE_HOME", runtime.path().join("home"));
    let swarm_id = "named-routing";
    let requester = "coord";
    let worker = "wrong-worker";
    let routing = jcode_swarm_core::AgentRoutingSelection {
        agent_role: "reviewer".into(), model: "gpt-test".into(), provider: "OpenAI".into(),
        api_method: "openai-api-key".into(), provider_key: Some("openai-api-key".into()),
    };
    let (client_tx, mut client_rx) = mpsc::unbounded_channel();
    let sessions = Arc::new(RwLock::new(HashMap::new()));
    let queues = Arc::new(RwLock::new(HashMap::new()));
    let connections = Arc::new(RwLock::new(HashMap::new()));
    let members = Arc::new(RwLock::new(HashMap::from([
        (requester.to_string(), { let mut m = member(requester, swarm_id, "ready"); m.role = "coordinator".into(); m }),
        (worker.to_string(), owned_member(worker, swarm_id, "ready", requester)),
    ])));
    let swarms = Arc::new(RwLock::new(HashMap::from([(swarm_id.into(), HashSet::from([requester.into(), worker.into()]))])));
    let mut plan = VersionedPlan::new();
    plan.items.push(plan_item("task", "pending", "high", &[]));
    let plans = Arc::new(RwLock::new(HashMap::from([(swarm_id.into(), plan)])));
    let coordinators = Arc::new(RwLock::new(HashMap::from([(swarm_id.into(), requester.into())])));
    let history = Arc::new(RwLock::new(VecDeque::new()));
    let counter = Arc::new(AtomicU64::new(1));
    let (events, _) = broadcast::channel(32);
    let mutations = SwarmMutationRuntime::default();

    for target in [Some(worker.to_string()), None] {
        handle_comm_assign_task(
            Ok(Some(routing.clone())), 7, requester.into(), target, Some("task".into()), None,
            &client_tx, &sessions, &queues, &connections, &members, &swarms, &plans,
            &coordinators, &history, &counter, &events, &mutations,
        ).await;
        assert!(matches!(client_rx.recv().await, Some(ServerEvent::Error { .. })));
        assert!(plans.read().await[swarm_id].items[0].assigned_to.is_none());
        assert!(queues.read().await.is_empty());
    }

    // A failed worker may have been removed. Its task policy must survive independently.
    {
        let mut plans = plans.write().await;
        let plan = plans.get_mut(swarm_id).unwrap();
        plan.items[0].status = "failed".into();
        plan.items[0].assigned_to = Some("removed-worker".into());
        plan.task_progress.entry("task".into()).or_default().routing = Some(routing);
    }
    handle_comm_task_control(
        Err(anyhow::anyhow!("unrelated default is unavailable")),
        8, requester.into(), "replace".into(), "task".into(), Some(worker.into()), None,
        &client_tx, &sessions, &queues, &connections, &members, &swarms, &plans,
        &coordinators, &history, &counter, &events, &mutations,
    ).await;
    let Some(ServerEvent::Error { message, .. }) = client_rx.recv().await else { panic!("expected route mismatch") };
    assert!(message.contains("preserved agent_role 'reviewer'"), "{message}");
    assert!(!message.contains("unrelated default"));
    assert_eq!(plans.read().await[swarm_id].items[0].assigned_to.as_deref(), Some("removed-worker"));
    assert!(queues.read().await.is_empty());
}
