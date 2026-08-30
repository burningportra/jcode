#![cfg_attr(test, allow(clippy::await_holding_lock))]

use super::{Tool, ToolContext, ToolOutput};
use crate::bus::{Bus, BusEvent, SidePanelUpdated};
use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

pub struct InitiativeTool;

impl InitiativeTool {
    pub fn new() -> Self {
        Self
    }
}

fn default_display_for_action(action: &str) -> crate::goal::GoalDisplayMode {
    match action {
        // The tool must never open (spawn) the side panel on its own; users
        // open it explicitly via /goals. UpdateOnly refreshes pages that are
        // already open without stealing focus.
        "update" | "checkpoint" | "review" => crate::goal::GoalDisplayMode::UpdateOnly,
        _ => crate::goal::GoalDisplayMode::None,
    }
}

fn publish_side_panel_snapshot(session_id: &str, snapshot: &crate::side_panel::SidePanelSnapshot) {
    Bus::global().publish(BusEvent::SidePanelUpdated(SidePanelUpdated {
        session_id: session_id.to_string(),
        snapshot: snapshot.clone(),
    }));
}

fn maybe_publish_goals_overview_refresh(
    session_id: &str,
    working_dir: Option<&std::path::Path>,
) -> Result<()> {
    if let Some(snapshot) =
        crate::goal::refresh_goals_overview_for_session(session_id, working_dir)?
    {
        publish_side_panel_snapshot(session_id, &snapshot);
    }
    Ok(())
}

fn goal_page_is_open(session_id: &str, goal_id: &str) -> Result<bool> {
    let page_id = crate::goal::goal_page_id(goal_id);
    let snapshot = crate::side_panel::snapshot_for_session(session_id)?;
    Ok(snapshot.pages.iter().any(|page| page.id == page_id))
}

#[derive(Debug, Deserialize)]
struct GoalInput {
    action: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    why: Option<String>,
    #[serde(default)]
    success_criteria: Option<Vec<String>>,
    #[serde(default)]
    milestones: Option<Vec<crate::goal::GoalMilestone>>,
    #[serde(default)]
    next_steps: Option<Vec<String>>,
    #[serde(default)]
    blockers: Option<Vec<String>>,
    #[serde(default)]
    current_milestone_id: Option<String>,
    #[serde(default)]
    progress_percent: Option<u8>,
    #[serde(default)]
    checkpoint_summary: Option<String>,
    #[serde(default)]
    display: Option<String>,
    #[serde(default)]
    lens: Option<String>,
    #[serde(default)]
    score: Option<u8>,
    #[serde(default)]
    pass: Option<u32>,
    #[serde(default)]
    gaps: Option<Vec<String>>,
    #[serde(default)]
    resolved: Option<Vec<String>>,
    #[serde(default)]
    reviewer_model: Option<String>,
    #[serde(default)]
    format: Option<String>,
}

fn goal_step_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": true
    })
}

fn goal_milestone_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "steps": {
                "type": "array",
                "items": goal_step_schema()
            }
        },
        "additionalProperties": true
    })
}

#[async_trait]
impl Tool for InitiativeTool {
    fn name(&self) -> &str {
        "initiative"
    }

    fn description(&self) -> &str {
        "Manage durable initiatives."
    }

    fn parameters_schema(&self) -> Value {
        json!({
        "type": "object",
        "required": ["action"],
            "properties": {
                "intent": super::intent_schema_property(),
                "action": {
                    "type": "string",
                    "enum": ["create", "list", "show", "resume", "update", "checkpoint", "review", "delete", "focus"],
                    "description": "Action."
                },
                "id": {"type": "string", "description": "Initiative ID. Required for show, focus, update, checkpoint, review, and delete. The id is returned in the create response (in backticks)."},
                "title": {"type": "string"},
                "scope": {"type": "string"},
                "status": {"type": "string"},
                "description": {"type": "string"},
                "why": {"type": "string"},
                "success_criteria": {"type": "array", "items": {"type": "string"}},
                "milestones": {"type": "array", "items": goal_milestone_schema()},
                "next_steps": {"type": "array", "items": {"type": "string"}},
                "blockers": {"type": "array", "items": {"type": "string"}},
                "current_milestone_id": {"type": "string"},
                "progress_percent": {"type": "integer"},
                "checkpoint_summary": {"type": "string"},
                "lens": {"type": "string", "description": "Review lens for a `review` pass, e.g. architecture, edge-cases, security."},
                "score": {"type": "integer", "description": "Quality score 0-100 after a `review` pass."},
                "pass": {"type": "integer", "description": "1-based pass number for `review`; auto-increments when omitted."},
                "gaps": {"type": "array", "items": {"type": "string"}, "description": "Gaps found in this review pass."},
                "resolved": {"type": "array", "items": {"type": "string"}, "description": "Gaps resolved in this review pass."},
                "reviewer_model": {"type": "string", "description": "Model that produced a cross-model review pass, if any."},
                "format": {"type": "string", "enum": ["markdown", "json"], "description": "Output format for the `list` action. Default markdown; `json` returns the goals as an in-band JSON array (same shape as list metadata) so models/scripts get structured data."}
            }
        })
    }

    async fn execute(&self, input: Value, ctx: ToolContext) -> Result<ToolOutput> {
        let params: GoalInput = serde_json::from_value(input)?;
        let action_label = params.action.clone();
        let goal_id_label = params.id.clone().unwrap_or_else(|| "<none>".to_string());
        let working_dir = ctx.working_dir.as_deref();
        let display = params
            .display
            .as_deref()
            .and_then(crate::goal::GoalDisplayMode::parse)
            .unwrap_or_else(|| default_display_for_action(&params.action));

        match params.action.as_str() {
            "list" => {
                // Resolve the output format up front so an invalid value errors
                // before any side effects. markdown (default) keeps the output
                // byte-identical to before; json puts structured data in-band so
                // the model/scripts can consume it (metadata is not forwarded to
                // the model transcript).
                let as_json = match params.format.as_deref().map(str::trim) {
                    None | Some("") | Some("markdown") => false,
                    Some("json") => true,
                    Some(other) => {
                        anyhow::bail!("unknown format: {} (expected markdown or json)", other)
                    }
                };
                let goals = crate::goal::list_relevant_goals(working_dir)?;
                if display != crate::goal::GoalDisplayMode::None {
                    let focus = display != crate::goal::GoalDisplayMode::UpdateOnly;
                    let snapshot = crate::goal::open_goals_overview_for_session(
                        &ctx.session_id,
                        working_dir,
                        focus,
                    )?;
                    publish_side_panel_snapshot(&ctx.session_id, &snapshot);
                }
                let goals_json = serde_json::to_value(&goals)?;
                let body = if as_json {
                    serde_json::to_string_pretty(&goals_json)?
                } else {
                    crate::goal::render_goals_overview(&goals)
                };
                Ok(ToolOutput::new(body)
                    .with_title(format!("{} goals", goals.len()))
                    .with_metadata(goals_json))
            }
            "create" => {
                let title = params
                    .title
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("title is required for create"))?;
                let scope = params
                    .scope
                    .as_deref()
                    .and_then(crate::goal::GoalScope::parse)
                    .unwrap_or(crate::goal::GoalScope::Project);
                let goal = crate::goal::create_goal(
                    crate::goal::GoalCreateInput {
                        id: params.id.clone(),
                        title: title.to_string(),
                        scope,
                        description: params.description.clone(),
                        why: params.why.clone(),
                        success_criteria: params.success_criteria.unwrap_or_default(),
                        milestones: params.milestones.unwrap_or_default(),
                        next_steps: params.next_steps.unwrap_or_default(),
                        blockers: params.blockers.unwrap_or_default(),
                        current_milestone_id: params.current_milestone_id.clone(),
                        progress_percent: params.progress_percent,
                    },
                    working_dir,
                )?;
                let metadata = serde_json::to_value(&goal)?;
                let output = if display == crate::goal::GoalDisplayMode::None {
                    ToolOutput::new(format!(
                        "Created initiative `{}` ({}).\nid=`{}` — pass this id to update, checkpoint, review, show, and delete.",
                        goal.id, goal.title, goal.id
                    ))
                } else {
                    let snapshot =
                        crate::goal::write_goal_page(&ctx.session_id, working_dir, &goal, display)?;
                    publish_side_panel_snapshot(&ctx.session_id, &snapshot);
                    maybe_publish_goals_overview_refresh(&ctx.session_id, working_dir)?;
                    ToolOutput::new(format!(
                        "Created initiative `{}` ({}) and opened it in the side panel.\nid=`{}` — pass this id to update, checkpoint, review, show, and delete.",
                        goal.id, goal.title, goal.id
                    ))
                };
                Ok(output
                    .with_title(goal.title.clone())
                    .with_metadata(metadata))
            }
            "show" | "focus" => {
                let id = params
                    .id
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("id is required for show/focus"))?;
                if display == crate::goal::GoalDisplayMode::None {
                    let Some(goal) = crate::goal::load_goal(id, None, working_dir)? else {
                        anyhow::bail!("initiative not found: {}", id);
                    };
                    crate::goal::attach_goal_to_session(&ctx.session_id, &goal, working_dir)?;
                    Ok(ToolOutput::new(crate::goal::render_goal_detail(&goal))
                        .with_title(goal.title.clone())
                        .with_metadata(serde_json::to_value(&goal)?))
                } else {
                    let Some(result) = crate::goal::open_goal_for_session(
                        &ctx.session_id,
                        working_dir,
                        id,
                        params.action == "focus" || display == crate::goal::GoalDisplayMode::Focus,
                    )?
                    else {
                        anyhow::bail!("initiative not found: {}", id);
                    };
                    publish_side_panel_snapshot(&ctx.session_id, &result.snapshot);
                    Ok(
                        ToolOutput::new(crate::goal::render_goal_detail(&result.goal))
                            .with_title(result.goal.title.clone())
                            .with_metadata(serde_json::to_value(&result.goal)?),
                    )
                }
            }
            "resume" => {
                let goal = if display == crate::goal::GoalDisplayMode::None {
                    let Some(goal) = crate::goal::resume_goal(&ctx.session_id, working_dir)? else {
                        return Ok(ToolOutput::new("No resumable goals found."));
                    };
                    crate::goal::attach_goal_to_session(&ctx.session_id, &goal, working_dir)?;
                    goal
                } else {
                    let Some(result) = crate::goal::resume_goal_for_session(
                        &ctx.session_id,
                        working_dir,
                        display == crate::goal::GoalDisplayMode::Focus,
                    )?
                    else {
                        return Ok(ToolOutput::new("No resumable goals found."));
                    };
                    publish_side_panel_snapshot(&ctx.session_id, &result.snapshot);
                    result.goal
                };
                let mut output = format!("Resumed initiative `{}` ({})", goal.id, goal.title);
                if let Some(progress) = goal.progress_percent {
                    output.push_str(&format!(" — {}%", progress));
                }
                if let Some(next_step) = goal.next_steps.first() {
                    output.push_str(&format!("\nNext step: {}", next_step));
                }
                Ok(ToolOutput::new(output)
                    .with_title(goal.title.clone())
                    .with_metadata(serde_json::to_value(&goal)?))
            }
            "update" | "checkpoint" => {
                let id = params
                    .id
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("id is required for update/checkpoint"))?;
                let status = params
                    .status
                    .as_deref()
                    .map(|value| {
                        crate::goal::GoalStatus::parse(value)
                            .ok_or_else(|| anyhow::anyhow!("invalid goal status: {}", value))
                    })
                    .transpose()?;
                let goal = crate::goal::update_goal(
                    id,
                    params
                        .scope
                        .as_deref()
                        .and_then(crate::goal::GoalScope::parse),
                    working_dir,
                    crate::goal::GoalUpdateInput {
                        title: params.title.clone(),
                        description: params.description.clone(),
                        why: params.why.clone(),
                        status,
                        success_criteria: params.success_criteria.clone(),
                        milestones: params.milestones.clone(),
                        next_steps: params.next_steps.clone(),
                        blockers: params.blockers.clone(),
                        current_milestone_id: if params.current_milestone_id.is_some() {
                            Some(params.current_milestone_id.clone())
                        } else {
                            None
                        },
                        progress_percent: if params.progress_percent.is_some() {
                            Some(params.progress_percent)
                        } else {
                            None
                        },
                        checkpoint_summary: if params.action == "checkpoint" {
                            params
                                .checkpoint_summary
                                .clone()
                                .or(params.description.clone())
                        } else {
                            params.checkpoint_summary.clone()
                        },
                    },
                )?
                .ok_or_else(|| anyhow::anyhow!("initiative not found: {}", id))?;
                if display != crate::goal::GoalDisplayMode::None {
                    let should_write_goal_page = match display {
                        crate::goal::GoalDisplayMode::None => false,
                        crate::goal::GoalDisplayMode::UpdateOnly => {
                            goal_page_is_open(&ctx.session_id, &goal.id)?
                        }
                        crate::goal::GoalDisplayMode::Auto
                        | crate::goal::GoalDisplayMode::Focus => true,
                    };
                    if should_write_goal_page {
                        let snapshot = crate::goal::write_goal_page(
                            &ctx.session_id,
                            working_dir,
                            &goal,
                            display,
                        )?;
                        publish_side_panel_snapshot(&ctx.session_id, &snapshot);
                    }
                    maybe_publish_goals_overview_refresh(&ctx.session_id, working_dir)?;
                }
                Ok(
                    ToolOutput::new(format!("Updated initiative `{}` ({})", goal.id, goal.title))
                        .with_title(goal.title.clone())
                        .with_metadata(serde_json::to_value(&goal)?),
                )
            }
            "review" => {
                let id = params
                    .id
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("id is required for review"))?;
                let lens = params
                    .lens
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| anyhow::anyhow!("lens is required for review"))?;
                let goal = crate::goal::record_review(
                    id,
                    params
                        .scope
                        .as_deref()
                        .and_then(crate::goal::GoalScope::parse),
                    working_dir,
                    crate::goal::GoalReviewInput {
                        pass: params.pass,
                        lens: lens.to_string(),
                        score: params.score.unwrap_or(0),
                        gaps: params.gaps.clone().unwrap_or_default(),
                        resolved: params.resolved.clone().unwrap_or_default(),
                        reviewer_model: params.reviewer_model.clone(),
                        summary: params.checkpoint_summary.clone(),
                    },
                )?
                .ok_or_else(|| anyhow::anyhow!("initiative not found: {}", id))?;
                if display != crate::goal::GoalDisplayMode::None
                    && goal_page_is_open(&ctx.session_id, &goal.id)?
                {
                    let snapshot =
                        crate::goal::write_goal_page(&ctx.session_id, working_dir, &goal, display)?;
                    publish_side_panel_snapshot(&ctx.session_id, &snapshot);
                }
                let latest = goal.reviews.last();
                let summary = latest
                    .map(|r| format!(" pass {} ({}) {}/100", r.pass, r.lens, r.score))
                    .unwrap_or_default();
                Ok(
                    ToolOutput::new(format!("Recorded review on `{}`:{}", goal.id, summary))
                        .with_title(goal.title.clone())
                        .with_metadata(serde_json::to_value(&goal)?),
                )
            }
            "delete" => {
                let id = params
                    .id
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("id is required for delete"))?;
                let scope_hint = params
                    .scope
                    .as_deref()
                    .and_then(crate::goal::GoalScope::parse);
                let Some(goal) = crate::goal::delete_goal(
                    id,
                    scope_hint,
                    working_dir,
                    Some(ctx.session_id.as_str()),
                )?
                else {
                    anyhow::bail!("initiative not found: {}", id);
                };
                Ok(ToolOutput::new(format!(
                    "Deleted initiative `{}` ({}) [was {}]",
                    goal.id,
                    goal.title,
                    goal.status.as_str()
                ))
                .with_title(goal.title.clone())
                .with_metadata(serde_json::to_value(&goal)?))
            }
            other => anyhow::bail!("unknown goal action: {}", other),
        }
        .map_err(|err| {
            crate::logging::warn(&format!(
                "[tool:goal] action failed action={} goal_id={} session_id={} error={}",
                action_label, goal_id_label, ctx.session_id, err
            ));
            err
        })
    }
}

#[cfg(test)]
#[path = "goal_tests.rs"]
mod goal_tests;
