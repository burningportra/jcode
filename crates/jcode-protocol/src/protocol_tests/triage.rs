// Tests for graph-theory triage surfaced on PlanGraphStatus.

use crate::{PlanGraphStatus, VersionedPlan};

fn plan_item(id: &str, status: &str, blocked_by: &[&str]) -> jcode_plan::PlanItem {
    jcode_plan::PlanItem {
        id: id.to_string(),
        content: id.to_string(),
        status: status.to_string(),
        priority: "high".to_string(),
        subsystem: None,
        file_scope: Vec::new(),
        blocked_by: blocked_by.iter().map(|s| s.to_string()).collect(),
        assigned_to: None,
    }
}

fn plan_with(items: Vec<jcode_plan::PlanItem>) -> VersionedPlan {
    let mut plan = VersionedPlan::new();
    plan.replace_items(items);
    plan
}

#[test]
fn from_versioned_plan_populates_triage_ranking_best_first() -> Result<()> {
    // Two independent roots: `b`'s completion unblocks a long chain, so it must
    // rank #1 in triage even though `a` is also runnable.
    let plan = plan_with(vec![
        plan_item("a", "ready", &[]),
        plan_item("b", "ready", &[]),
        plan_item("c", "ready", &["b"]),
        plan_item("d", "ready", &["c"]),
    ]);
    let status = PlanGraphStatus::from_versioned_plan("s1", &plan, Some(8), Vec::new());
    assert_eq!(status.ready_ids, vec!["a".to_string(), "b".to_string()]);
    assert!(!status.triage_ranking.is_empty(), "triage must be populated");
    // The entry with the largest unblock reach (b) is ranked first.
    let first = &status.triage_ranking[0];
    assert_eq!(first.id, "b", "highest unblock reach ranks first");
    assert!(first.rank == 1);
    assert!(first.unblock_reach > status.triage_ranking[1].unblock_reach);
    let ids: Vec<&str> = status
        .triage_ranking
        .iter()
        .map(|e| e.id.as_str())
        .collect();
    assert_eq!(ids, vec!["b", "a"], "only runnable roots appear, ordered by unlock");

    // The plan-status formatter renders the triage block so agents can read it.
    let rendered = crate::format_comm_plan_status(&status);
    assert!(rendered.contains("Triage (best-first, by unblock reach)"));
    assert!(rendered.contains("1. b (unblocks"));
    Ok(())
}

#[test]
fn empty_plan_has_no_triage() -> Result<()> {
    let plan = plan_with(vec![]);
    let status = PlanGraphStatus::from_versioned_plan("s1", &plan, Some(8), Vec::new());
    assert!(status.triage_ranking.is_empty());
    assert!(status.ready_ids.is_empty());
    Ok(())
}
