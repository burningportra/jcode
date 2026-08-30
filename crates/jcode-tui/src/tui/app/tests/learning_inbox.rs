#[test]
fn learning_inbox_review_completion_queues_proposal_drafting_turn() {
    let mut app = create_test_app();
    let session_id = app.session.id.clone();
    let handled =
        app.handle_learning_inbox_command_completed(crate::bus::LearningInboxCommandCompleted {
            session_id,
            action: "review".to_string(),
            suggestion_id: Some("a".repeat(64)),
            suggestion_kind: Some("workflow".to_string()),
            review_prompt: Some(
                "Call skill_manage review_crystallization, draft one focused skill, and call skill_manage crystallize. Do not approve or install it."
                    .to_string(),
            ),
            output: Some("Reviewed repeated workflow evidence.".to_string()),
            error: None,
        });

    assert!(handled);
    assert!(app.pending_queued_dispatch);
    assert_eq!(app.queued_messages.len(), 1);
    assert!(app.queued_messages[0].contains("skill_manage crystallize"));
    assert!(app.queued_messages[0].contains("Do not approve or install"));
}

#[test]
fn skill_evolution_review_queues_evolution_proposal_without_approval() {
    let mut app = create_test_app();
    let session_id = app.session.id.clone();
    let prompt = "Review evolution suggestion abc, then call skill_manage propose_skill_evolution. Do not approve or mutate any skill.";
    let handled =
        app.handle_learning_inbox_command_completed(crate::bus::LearningInboxCommandCompleted {
            session_id,
            action: "review".to_string(),
            suggestion_id: Some("b".repeat(64)),
            suggestion_kind: Some("refine".to_string()),
            review_prompt: Some(prompt.to_string()),
            output: Some("Reviewed skill refinement evidence.".to_string()),
            error: None,
        });

    assert!(handled);
    assert!(app.pending_queued_dispatch);
    assert_eq!(app.queued_messages, vec![prompt]);
    assert!(!app.queued_messages[0].contains("approve_skill_evolution"));
}

#[test]
fn learning_inbox_command_fails_closed_in_remote_tui() {
    let mut app = create_test_app();
    app.is_remote = true;
    app.set_learning_inbox_store_is_local(false);

    assert!(super::learning_inbox::handle_learning_command(
        &mut app,
        "/learning dismiss"
    ));
    assert!(app.queued_messages.is_empty());
    assert!(!app.pending_queued_dispatch);
}
