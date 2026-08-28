#[test]
fn learning_inbox_review_completion_queues_proposal_drafting_turn() {
    let mut app = create_test_app();
    let session_id = app.session.id.clone();
    let handled = app.handle_learning_inbox_command_completed(
        crate::bus::LearningInboxCommandCompleted {
            session_id,
            action: "review".to_string(),
            suggestion_id: Some("a".repeat(64)),
            output: Some("Reviewed repeated workflow evidence.".to_string()),
            error: None,
        },
    );

    assert!(handled);
    assert!(app.pending_queued_dispatch);
    assert_eq!(app.queued_messages.len(), 1);
    assert!(app.queued_messages[0].contains("skill_manage crystallize"));
    assert!(app.queued_messages[0].contains("Do not approve or install"));
}

#[test]
fn learning_inbox_command_fails_closed_in_remote_tui() {
    let mut app = create_test_app();
    app.is_remote = true;

    assert!(super::learning_inbox::handle_learning_command(
        &mut app,
        "/learning dismiss"
    ));
    assert!(app.queued_messages.is_empty());
    assert!(!app.pending_queued_dispatch);
}
