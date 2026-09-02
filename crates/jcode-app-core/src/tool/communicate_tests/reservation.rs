// Reservation action tests for the swarm tool.
// The reserve/release/reservations actions are self-contained (no server round
// trip), so they are exercised with a minimal ToolContext pointed at a temp
// project root. Parent imports (`Tool`, `ToolContext`, json!, etc.) are in scope.

fn reservation_ctx(project: &std::path::Path) -> ToolContext {
    ToolContext {
        session_id: "test-session".to_string(),
        message_id: "test-msg".to_string(),
        tool_call_id: "test-call".to_string(),
        working_dir: Some(project.to_path_buf()),
        stdin_request_tx: None,
        graceful_shutdown_signal: None,
        execution_mode: crate::tool::ToolExecutionMode::Direct,
    }
}

fn out_text(output: &crate::tool::ToolOutput) -> String {
    output.output.clone()
}

#[tokio::test]
async fn reserve_list_release_roundtrip() {
    let _guard = crate::storage::lock_test_env();
    let project = tempfile::tempdir().expect("tempdir");
    let tool = CommunicateTool::new();

    let out = tool
        .execute(
            json!({"action":"reserve","paths":["src/*.rs"],"ttl_secs":300,"holder_label":"A-agent","reason":"refactor auth"}),
            reservation_ctx(&project.path()),
        )
        .await
        .expect("reserve executes");
    assert!(out_text(&out).contains("Reserved 1 paths"), "output shown in failure above");

    let list = tool
        .execute(json!({"action":"reservations"}), reservation_ctx(&project.path()))
        .await
        .expect("list executes");
    assert!(out_text(&list).contains("src/*.rs"), "output shown in failure above");

    let release = tool
        .execute(json!({"action":"release"}), reservation_ctx(&project.path()))
        .await
        .expect("release executes");
    assert!(
        out_text(&release).contains("Released 1 reservation"),
        "output shown in failure above"
    );

    let list2 = tool
        .execute(json!({"action":"reservations"}), reservation_ctx(&project.path()))
        .await
        .expect("second list executes");
    assert!(
        out_text(&list2).contains("No active reservations"),
        "output shown in failure above"
    );
}

#[tokio::test]
async fn reserve_requires_paths() {
    let project = tempfile::tempdir().expect("tempdir");
    let tool = CommunicateTool::new();
    let result = tool
        .execute(json!({"action":"reserve"}), reservation_ctx(&project.path()))
        .await;
    assert!(result.is_err(), "reserve without paths must fail");
}