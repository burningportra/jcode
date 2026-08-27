//! Skill tool - load, list, reload, and read skills

use super::{Tool, ToolContext, ToolOutput};
use crate::skill::SkillRegistry;
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::{Arc, OnceLock};
use tokio::sync::RwLock;

mod crystallization;
mod discovery;

use crystallization::EvidenceReference;

static CRYSTALLIZATION_OPERATION_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

pub struct SkillTool {
    registry: Arc<RwLock<SkillRegistry>>,
}

impl SkillTool {
    pub fn new(registry: Arc<RwLock<SkillRegistry>>) -> Self {
        Self { registry }
    }

    /// Effective skill set for this call: shared global registry plus the
    /// session's project-local overlay resolved from the tool context working
    /// dir (issue #457). The overlay is read fresh from disk so edits are
    /// visible without daemon restarts and never enter the shared registry.
    async fn effective_registry(&self, working_dir: Option<&std::path::Path>) -> SkillRegistry {
        let global = self.registry.read().await;
        SkillRegistry::effective_for_working_dir(&global, working_dir)
    }
}

#[derive(Deserialize)]
struct SkillInput {
    /// Action to perform: load (default), list, reload, reload_all, read.
    /// `list` shows both loaded skills and the jcode-endorsed catalog.
    #[serde(default = "default_action")]
    action: String,
    /// Skill name (required for load, reload, read)
    #[serde(alias = "skill")]
    #[serde(default)]
    name: Option<String>,
    /// Optional Claude-compatible Skill wrapper argument. The skill loader only
    /// needs to load the prompt, so args are currently accepted and ignored.
    #[serde(default)]
    args: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    evidence: Option<Vec<EvidenceReference>>,
    #[serde(default)]
    proposal_id: Option<String>,
    #[serde(default)]
    confirmed: Option<bool>,
    #[serde(default)]
    approval_evidence: Option<EvidenceReference>,
    #[serde(default)]
    suggestion_id: Option<String>,
}

fn default_action() -> String {
    "load".to_string()
}

fn validate_action_fields(params: &SkillInput) -> Result<()> {
    match params.action.as_str() {
        "crystallize" => {
            if params.proposal_id.is_some()
                || params.confirmed.is_some()
                || params.approval_evidence.is_some()
                || params.suggestion_id.is_some()
            {
                anyhow::bail!("crystallize accepts name, description, content, and evidence only");
            }
        }
        "approve_crystallization" => {
            if params.name.is_some()
                || params.description.is_some()
                || params.content.is_some()
                || params.evidence.is_some()
                || params.args.is_some()
                || params.suggestion_id.is_some()
            {
                anyhow::bail!(
                    "approve_crystallization accepts proposal_id, confirmed, and approval_evidence only"
                );
            }
        }
        "discover_crystallization" => {
            if params.name.is_some()
                || params.args.is_some()
                || params.description.is_some()
                || params.content.is_some()
                || params.evidence.is_some()
                || params.proposal_id.is_some()
                || params.confirmed.is_some()
                || params.approval_evidence.is_some()
                || params.suggestion_id.is_some()
            {
                anyhow::bail!("discover_crystallization accepts no action-specific fields");
            }
        }
        "review_crystallization" | "dismiss_crystallization" | "suppress_crystallization" => {
            if params.name.is_some()
                || params.args.is_some()
                || params.description.is_some()
                || params.content.is_some()
                || params.evidence.is_some()
                || params.proposal_id.is_some()
                || params.confirmed.is_some()
                || params.approval_evidence.is_some()
            {
                anyhow::bail!("Discovery suggestion actions accept suggestion_id only");
            }
        }
        _ => {
            if params.description.is_some()
                || params.content.is_some()
                || params.evidence.is_some()
                || params.proposal_id.is_some()
                || params.confirmed.is_some()
                || params.approval_evidence.is_some()
                || params.suggestion_id.is_some()
            {
                anyhow::bail!("Crystallization fields require a crystallization action");
            }
        }
    }
    Ok(())
}

#[async_trait]
impl Tool for SkillTool {
    fn name(&self) -> &str {
        "skill_manage"
    }

    fn description(&self) -> &str {
        "Manage skills."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "intent": super::intent_schema_property(),
                "action": {
                    "type": "string",
                    "enum": ["load", "list", "reload", "reload_all", "read", "crystallize", "approve_crystallization", "discover_crystallization", "review_crystallization", "dismiss_crystallization", "suppress_crystallization"],
                    "description": "Action."
                },
                "name": {
                    "type": "string",
                    "description": "Skill name."
                },
                "description": {
                    "type": "string",
                    "description": "Proposed skill description for crystallize."
                },
                "content": {
                    "type": "string",
                    "description": "Proposed SKILL.md body for crystallize."
                },
                "evidence": {
                    "type": "array",
                    "minItems": 2,
                    "maxItems": 12,
                    "items": {
                        "type": "object",
                        "required": ["session_id", "message_id"],
                        "properties": {
                            "session_id": {"type": "string"},
                            "message_id": {"type": "string"}
                        }
                    }
                },
                "proposal_id": {
                    "type": "string",
                    "description": "Content-addressed crystallization proposal ID."
                },
                "confirmed": {
                    "type": "boolean",
                    "description": "Must be true to install an approved crystallization proposal."
                },
                "approval_evidence": {
                    "type": "object",
                    "required": ["session_id", "message_id"],
                    "properties": {
                        "session_id": {"type": "string"},
                        "message_id": {"type": "string"}
                    },
                    "description": "Persisted user message containing the full proposal ID and explicit approval."
                },
                "suggestion_id": {
                    "type": "string",
                    "description": "Content-addressed proactive discovery suggestion ID."
                }
            }
        })
    }

    async fn execute(&self, input: Value, ctx: ToolContext) -> Result<ToolOutput> {
        let params: SkillInput = serde_json::from_value(input)?;
        validate_action_fields(&params)?;
        let action_label = params.action.clone();
        let name_label = params.name.clone().unwrap_or_else(|| "<none>".to_string());
        let _args = params.args.as_deref();

        match params.action.as_str() {
            "load" => {
                self.load_skill(params.name, ctx.working_dir.as_deref())
                    .await
            }
            "list" => self.list_skills(ctx.working_dir.as_deref()).await,
            "reload" => self.reload_skill(params.name).await,
            "reload_all" => self.reload_all_skills(ctx.working_dir.as_deref()).await,
            "read" => {
                self.read_skill(params.name, ctx.working_dir.as_deref())
                    .await
            }
            "crystallize" => {
                self.crystallize(
                    params.name,
                    params.description,
                    params.content,
                    params.evidence,
                )
                .await
            }
            "approve_crystallization" => {
                self.approve_crystallization(
                    params.proposal_id,
                    params.confirmed == Some(true),
                    params.approval_evidence,
                )
                .await
            }
            "discover_crystallization" => self.discover_crystallization().await,
            "review_crystallization" => self.review_crystallization(params.suggestion_id).await,
            "dismiss_crystallization" => self.dismiss_crystallization(params.suggestion_id).await,
            "suppress_crystallization" => self.suppress_crystallization(params.suggestion_id).await,
            _ => Ok(ToolOutput::new(format!(
                "Unknown action: {}. Use a documented skill_manage action.",
                params.action
            ))),
        }
        .map_err(|err| {
            crate::logging::warn(&format!(
                "[tool:skill_manage] action failed action={} skill={} session_id={} error={}",
                action_label, name_label, ctx.session_id, err
            ));
            err
        })
    }
}

impl SkillTool {
    async fn discover_crystallization(&self) -> Result<ToolOutput> {
        let _operation = CRYSTALLIZATION_OPERATION_LOCK
            .get_or_init(|| tokio::sync::Mutex::new(()))
            .lock()
            .await;
        let _file_lock = crystallization::acquire_operation_lock()?;
        match discovery::discover()? {
            Some(suggestion) => Ok(discovery::suggestion_output(&suggestion, "suggested")),
            None => Ok(discovery::no_suggestion_output()),
        }
    }

    async fn review_crystallization(&self, suggestion_id: Option<String>) -> Result<ToolOutput> {
        let suggestion = discovery::review(&required_suggestion_id(suggestion_id)?)?;
        Ok(discovery::suggestion_output(&suggestion, "reviewed"))
    }

    async fn dismiss_crystallization(&self, suggestion_id: Option<String>) -> Result<ToolOutput> {
        let _operation = CRYSTALLIZATION_OPERATION_LOCK
            .get_or_init(|| tokio::sync::Mutex::new(()))
            .lock()
            .await;
        let _file_lock = crystallization::acquire_operation_lock()?;
        let suggestion = discovery::dismiss(&required_suggestion_id(suggestion_id)?)?;
        Ok(discovery::state_output(&suggestion, "dismissed"))
    }

    async fn suppress_crystallization(&self, suggestion_id: Option<String>) -> Result<ToolOutput> {
        let _operation = CRYSTALLIZATION_OPERATION_LOCK
            .get_or_init(|| tokio::sync::Mutex::new(()))
            .lock()
            .await;
        let _file_lock = crystallization::acquire_operation_lock()?;
        let suggestion = discovery::suppress(&required_suggestion_id(suggestion_id)?)?;
        Ok(discovery::state_output(&suggestion, "suppressed"))
    }

    async fn crystallize(
        &self,
        name: Option<String>,
        description: Option<String>,
        content: Option<String>,
        evidence: Option<Vec<EvidenceReference>>,
    ) -> Result<ToolOutput> {
        let _operation = CRYSTALLIZATION_OPERATION_LOCK
            .get_or_init(|| tokio::sync::Mutex::new(()))
            .lock()
            .await;
        let _file_lock = crystallization::acquire_operation_lock()?;
        let registry = self.registry.read().await;
        let proposal = crystallization::propose(&registry, name, description, content, evidence)?;
        let preview = crystallization::skill_preview(&proposal);
        Ok(crystallization_output(&proposal, &preview, false, None))
    }

    async fn approve_crystallization(
        &self,
        proposal_id: Option<String>,
        confirmed: bool,
        approval_evidence: Option<EvidenceReference>,
    ) -> Result<ToolOutput> {
        let _operation = CRYSTALLIZATION_OPERATION_LOCK
            .get_or_init(|| tokio::sync::Mutex::new(()))
            .lock()
            .await;
        let _file_lock = crystallization::acquire_operation_lock()?;
        let proposal_id = proposal_id
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!("'proposal_id' is required for approve_crystallization action")
            })?;
        let proposal = crystallization::load_for_approval(&proposal_id)?;
        let preview = crystallization::skill_preview(&proposal);
        if !confirmed {
            return Ok(crystallization_output(&proposal, &preview, false, None));
        }
        let approval_evidence = approval_evidence.context(
            "'approval_evidence' is required for confirmed approve_crystallization action",
        )?;
        crystallization::verify_approval(&proposal, &approval_evidence)?;
        {
            let registry = self.registry.read().await;
            crystallization::revalidate_for_approval(&registry, &proposal)?;
        }
        let (installed_path, installed_preview) = crystallization::install_skill(&proposal)?;
        let candidate = match SkillRegistry::load_global() {
            Ok(candidate) => candidate,
            Err(error) => {
                return Ok(crystallization_incomplete_output(
                    &proposal,
                    &installed_preview,
                    &installed_path,
                    "installed_registry_incomplete",
                    &error.to_string(),
                ));
            }
        };
        if let Err(error) = crystallization::verify_reloaded(&candidate, &proposal, &installed_path)
        {
            return Ok(crystallization_incomplete_output(
                &proposal,
                &installed_preview,
                &installed_path,
                "installed_registry_incomplete",
                &error.to_string(),
            ));
        }
        *self.registry.write().await = candidate;
        let archive_path = match crystallization::archive(&proposal) {
            Ok(path) => path,
            Err(error) => {
                return Ok(crystallization_incomplete_output(
                    &proposal,
                    &installed_preview,
                    &installed_path,
                    "installed_archive_incomplete",
                    &error.to_string(),
                ));
            }
        };
        Ok(crystallization_output(
            &proposal,
            &installed_preview,
            true,
            Some((installed_path, archive_path)),
        ))
    }

    async fn load_skill(
        &self,
        name: Option<String>,
        working_dir: Option<&std::path::Path>,
    ) -> Result<ToolOutput> {
        let name = normalize_skill_name(name, "load")?;

        let registry = self.effective_registry(working_dir).await;
        let skill = registry.get(&name).ok_or_else(|| {
            // Endorsed skills are advertised in `list` but are not bundled;
            // a bare "not found" here reads like a bug (issue #445). Point at
            // the actual install command instead.
            if let Some(endorsed) = crate::skill::endorsed_skills()
                .iter()
                .find(|endorsed| endorsed.name == name)
            {
                match endorsed.install {
                    Some(install) => anyhow::anyhow!(
                        "Skill '{}' is endorsed but not installed. Install it with `{}`, then run skill_manage reload_all.",
                        name,
                        install
                    ),
                    None => anyhow::anyhow!(
                        "Skill '{}' is endorsed but not installed (source: {}). Install it into ~/.jcode/skills/{}/SKILL.md, then run skill_manage reload_all.",
                        name,
                        endorsed.source,
                        name
                    ),
                }
            } else {
                anyhow::anyhow!("Skill '{}' not found", name)
            }
        })?;

        let base_dir = skill
            .path
            .parent()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| ".".to_string());

        Ok(ToolOutput::new(format!(
            "## Skill: {}\n\n**Base directory**: {}\n\n{}",
            skill.name,
            base_dir,
            skill.get_prompt()
        ))
        .with_title(format!("skill: {}", skill.name)))
    }

    async fn list_skills(&self, working_dir: Option<&std::path::Path>) -> Result<ToolOutput> {
        let registry = self.effective_registry(working_dir).await;
        let mut skills = registry.list();
        skills.sort_by(|a, b| a.name.cmp(&b.name));

        let installed: std::collections::HashSet<&str> =
            skills.iter().map(|s| s.name.as_str()).collect();

        let mut output = if skills.is_empty() {
            "No skills loaded.\n\n\
            Skills are loaded from:\n\
            - ~/.jcode/skills/<skill-name>/SKILL.md (global)\n\
            - ./.jcode/skills/<skill-name>/SKILL.md (project-local)\n\
            - ./.claude/skills/<skill-name>/SKILL.md (compatibility)\n\n\
            Create a SKILL.md file with YAML frontmatter:\n\
            ---\n\
            name: my-skill\n\
            description: What this skill does\n\
            allowed-tools: bash, read, write\n\
            ---\n\n\
            # Skill content here\n"
                .to_string()
        } else {
            let mut output = format!("Loaded skills: {}\n\n", skills.len());
            for skill in &skills {
                output.push_str(&format!("## /{}\n", skill.name));
                output.push_str(&format!("  {}\n", skill.description));
                output.push_str(&format!("  Path: {}\n", skill.path.display()));
                if let Some(ref tools) = skill.allowed_tools {
                    output.push_str(&format!("  Tools: {}\n", tools.join(", ")));
                }
                output.push('\n');
            }
            output
        };

        append_endorsed_skills(&mut output, &installed);

        Ok(ToolOutput::new(output).with_title("Skills: List"))
    }

    async fn reload_skill(&self, name: Option<String>) -> Result<ToolOutput> {
        let name = normalize_skill_name(name, "reload")?;

        let mut registry = self.registry.write().await;

        match registry.reload(&name) {
            Ok(true) => {
                // Re-read to get updated info
                if let Some(skill) = registry.get(&name) {
                    Ok(ToolOutput::new(format!(
                        "Reloaded skill '{}'\n\nDescription: {}\nPath: {}",
                        name,
                        skill.description,
                        skill.path.display()
                    ))
                    .with_title(format!("Skills: Reloaded {}", name)))
                } else {
                    Ok(ToolOutput::new(format!("Reloaded skill '{}'", name))
                        .with_title(format!("Skills: Reloaded {}", name)))
                }
            }
            Ok(false) => Ok(ToolOutput::new(format!(
                "Skill '{}' not found or was deleted.\n\nUse 'list' to see available skills.",
                name
            ))
            .with_title("Skills: Not found")),
            Err(e) => {
                crate::logging::warn(&format!(
                    "[tool:skill_manage] reload failed skill={} error={}",
                    name, e
                ));
                Ok(
                    ToolOutput::new(format!("Failed to reload skill '{}': {}", name, e))
                        .with_title("Skills: Reload failed"),
                )
            }
        }
    }

    async fn reload_all_skills(&self, working_dir: Option<&std::path::Path>) -> Result<ToolOutput> {
        // Reload the shared GLOBAL registry only; the project-local overlay is
        // session-scoped and re-read from disk on every access, so reloading
        // it here would leak this session's project skills to other sessions
        // (issue #457).
        let reloaded = {
            let mut registry = self.registry.write().await;
            registry.reload_global()
        };

        match reloaded {
            Ok(global_count) => {
                let effective = self.effective_registry(working_dir).await;
                let skills = effective.list();
                let mut output = format!(
                    "Reloaded {} global skills ({} effective for this session)\n\n",
                    global_count,
                    skills.len()
                );

                for skill in skills {
                    output.push_str(&format!("- /{}: {}\n", skill.name, skill.description));
                }

                Ok(
                    ToolOutput::new(output)
                        .with_title(format!("Skills: Reloaded {}", global_count)),
                )
            }
            Err(e) => {
                crate::logging::warn(&format!(
                    "[tool:skill_manage] reload_all failed error={}",
                    e
                ));
                Ok(ToolOutput::new(format!("Failed to reload skills: {}", e))
                    .with_title("Skills: Reload failed"))
            }
        }
    }

    async fn read_skill(
        &self,
        name: Option<String>,
        working_dir: Option<&std::path::Path>,
    ) -> Result<ToolOutput> {
        let name = normalize_skill_name(name, "read")?;

        let registry = self.effective_registry(working_dir).await;

        if let Some(skill) = registry.get(&name) {
            let mut output = format!("# Skill: {}\n\n", skill.name);
            output.push_str(&format!("**Description:** {}\n", skill.description));
            output.push_str(&format!("**Path:** {}\n", skill.path.display()));
            if let Some(ref tools) = skill.allowed_tools {
                output.push_str(&format!("**Allowed tools:** {}\n", tools.join(", ")));
            }
            output.push_str("\n---\n\n");
            output.push_str(&skill.content);

            Ok(ToolOutput::new(output).with_title(format!("Skills: {}", name)))
        } else {
            Ok(ToolOutput::new(format!(
                "Skill '{}' not found.\n\nUse 'list' to see available skills.",
                name
            ))
            .with_title("Skills: Not found"))
        }
    }
}

/// Append the curated jcode-endorsed skill catalog to `output`, grouped by
/// category and marked with installed/not-installed status. `installed` is the
/// set of skill names currently loaded in the registry.
fn append_endorsed_skills(output: &mut String, installed: &std::collections::HashSet<&str>) {
    let endorsed = crate::skill::endorsed_skills();
    if endorsed.is_empty() {
        return;
    }

    output.push_str("\nEndorsed skills (recommended by jcode)\n");

    // Group by category, preserving first-seen order.
    let mut category_order: Vec<&str> = Vec::new();
    for skill in endorsed {
        if !category_order.contains(&skill.category) {
            category_order.push(skill.category);
        }
    }

    for category in category_order {
        let in_category: Vec<_> = endorsed.iter().filter(|e| e.category == category).collect();
        let installed_count = in_category
            .iter()
            .filter(|e| installed.contains(e.name))
            .count();
        output.push_str(&format!(
            "\n  {} ({}/{} installed)\n",
            category,
            installed_count,
            in_category.len()
        ));
        for skill in in_category {
            let is_installed = installed.contains(skill.name);
            let status = if is_installed {
                "installed"
            } else {
                "not installed"
            };
            output.push_str(&format!("  - /{} [{}]\n", skill.name, status));
            output.push_str(&format!("      {}\n", skill.description));
            output.push_str(&format!("      source: {}\n", skill.source));
            if !is_installed && let Some(install) = skill.install {
                output.push_str(&format!("      install: {}\n", install));
            }
        }
    }

    output.push_str(
        "\nActivate a loaded skill by loading it with skill_manage (action=load) or typing its slash command.\n",
    );
    output.push_str(
        "NVIDIA CUDA-X skills come from the official catalog at https://github.com/NVIDIA/skills.\n",
    );
}

fn normalize_skill_name(name: Option<String>, action: &str) -> Result<String> {
    let name = name.ok_or_else(|| anyhow::anyhow!("'name' is required for {} action", action))?;
    let trimmed = name.trim().trim_start_matches('/').to_string();
    if trimmed.is_empty() {
        anyhow::bail!("'name' is required for {} action", action);
    }
    Ok(trimmed)
}

fn required_suggestion_id(suggestion_id: Option<String>) -> Result<String> {
    suggestion_id
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("'suggestion_id' is required for this discovery action"))
}

fn crystallization_output(
    proposal: &crystallization::Proposal,
    preview: &str,
    installed: bool,
    paths: Option<(std::path::PathBuf, std::path::PathBuf)>,
) -> ToolOutput {
    let evidence = proposal
        .evidence
        .iter()
        .map(|item| {
            format!(
                "- session={} message={} role={}: {}",
                item.session_id, item.message_id, item.role, item.excerpt
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let confirmation = json!({
        "action": "approve_crystallization",
        "proposal_id": proposal.proposal_id,
        "confirmed": true,
        "approval_evidence": {
            "session_id": "<session containing the explicit user approval>",
            "message_id": "<persisted user approval message ID>"
        }
    });
    let (status, note) = if installed {
        (
            "installed",
            "The skill was created, reloaded, verified, and the proposal was archived.",
        )
    } else {
        (
            "pending_confirmation",
            "No skill was created. Explicit confirmation is required.",
        )
    };
    let mut metadata = json!({
        "kind": "skill_crystallization",
        "schema_version": 1,
        "status": status,
        "proposal_id": proposal.proposal_id,
        "name": proposal.name,
        "content_fingerprint": proposal.content_fingerprint,
        "preview": preview,
        "evidence": proposal.evidence,
        "confirmation": confirmation,
        "installed": installed
    });
    if let Some((installed_path, archive_path)) = paths {
        metadata["installed_path"] = json!(installed_path);
        metadata["archive_path"] = json!(archive_path);
    }
    ToolOutput::new(format!(
        "Skill crystallization proposal: {}\n\n## Exact SKILL.md preview\n\n```markdown\n{}\n```\n\n## Verified evidence\n{}\n\n{}\n\nApproval call:\n```json\n{}\n```",
        proposal.proposal_id,
        preview,
        evidence,
        note,
        serde_json::to_string_pretty(&confirmation).expect("JSON serialization cannot fail")
    ))
    .with_title(if installed {
        format!("Skill crystallized: {}", proposal.name)
    } else {
        format!("Skill proposal: {}", proposal.name)
    })
    .with_metadata(metadata)
}

fn crystallization_incomplete_output(
    proposal: &crystallization::Proposal,
    preview: &str,
    installed_path: &std::path::Path,
    status: &str,
    error: &str,
) -> ToolOutput {
    let retry = json!({
        "action": "approve_crystallization",
        "proposal_id": proposal.proposal_id,
        "confirmed": true,
        "approval_evidence": {
            "session_id": "<session containing the explicit user approval>",
            "message_id": "<persisted user approval message ID>"
        }
    });
    ToolOutput::new(format!(
        "Skill installation requires recovery. The exact skill exists at {}, but the workflow stopped in state `{status}`: {error}\n\nRetry with the same verified approval call.",
        installed_path.display()
    ))
    .with_title(format!("Skill recovery required: {}", proposal.name))
    .with_metadata(json!({
        "kind": "skill_crystallization",
        "schema_version": 1,
        "status": status,
        "proposal_id": proposal.proposal_id,
        "name": proposal.name,
        "preview": preview,
        "installed": true,
        "installed_path": installed_path,
        "retry": retry,
        "error": error
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_tool() -> SkillTool {
        let registry = Arc::new(RwLock::new(SkillRegistry::default()));
        SkillTool::new(registry)
    }

    fn create_test_tool_with_skill(name: &str) -> (SkillTool, tempfile::TempDir) {
        let temp_dir = tempfile::tempdir().unwrap();
        let skill_dir = temp_dir.path().join(".jcode").join("skills").join(name);
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            format!(
                "---\nname: {name}\ndescription: Test skill\n---\n\n# Test Skill\n\nUse this test skill."
            ),
        )
        .unwrap();

        let registry = SkillRegistry::load_for_working_dir(Some(temp_dir.path())).unwrap();
        let tool = SkillTool::new(Arc::new(RwLock::new(registry)));
        (tool, temp_dir)
    }

    fn create_test_context() -> ToolContext {
        ToolContext {
            session_id: "test-session".to_string(),
            message_id: "test-message".to_string(),
            tool_call_id: "test-tool-call".to_string(),
            working_dir: None,
            stdin_request_tx: None,
            graceful_shutdown_signal: None,
            execution_mode: crate::tool::ToolExecutionMode::Direct,
        }
    }

    #[test]
    fn test_tool_name() {
        let tool = create_test_tool();
        assert_eq!(tool.name(), "skill_manage");
    }

    #[test]
    fn test_tool_description() {
        let tool = create_test_tool();
        assert!(tool.description().contains("skill"));
    }

    #[test]
    fn test_parameters_schema() {
        let tool = create_test_tool();
        let schema = tool.parameters_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["action"].is_object());
        assert!(schema["properties"]["name"].is_object());
    }

    #[tokio::test]
    async fn test_list_empty() {
        let tool = create_test_tool();
        let ctx = create_test_context();
        let input = json!({"action": "list"});

        let result = tool.execute(input, ctx).await.unwrap();
        assert!(result.output.contains("No skills loaded"));
        // Even with no skills loaded, the endorsed catalog should be listed.
        assert!(result.output.contains("Endorsed skills"));
    }

    #[tokio::test]
    async fn test_list_includes_endorsed_skills() {
        let tool = create_test_tool();
        let ctx = create_test_context();
        let input = json!({"action": "list"});

        let result = tool.execute(input, ctx).await.unwrap();
        // Every endorsed skill should appear with an install-status marker.
        for endorsed in crate::skill::endorsed_skills() {
            assert!(
                result.output.contains(&format!("/{}", endorsed.name)),
                "expected endorsed skill /{} in:\n{}",
                endorsed.name,
                result.output
            );
        }
        // No skills are loaded in this tool, so they should be "not installed".
        assert!(result.output.contains("[not installed]"));
    }

    #[tokio::test]
    async fn test_load_missing_name() {
        let tool = create_test_tool();
        let ctx = create_test_context();
        let input = json!({"action": "load"});

        let result = tool.execute(input, ctx).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("name"));
    }

    #[tokio::test]
    async fn test_load_accepts_skill_alias_and_args() {
        let (tool, _temp_dir) = create_test_tool_with_skill("optimization");
        let ctx = create_test_context();
        let input = json!({"skill": "optimization", "args": "optimize this"});

        let result = tool.execute(input, ctx).await.unwrap();
        assert!(result.output.contains("## Skill: optimization"));
        assert_eq!(result.title.as_deref(), Some("skill: optimization"));
    }

    #[tokio::test]
    async fn test_load_strips_leading_slash_from_name() {
        let (tool, _temp_dir) = create_test_tool_with_skill("optimization");
        let ctx = create_test_context();
        let input = json!({"action": "load", "name": "/optimization"});

        let result = tool.execute(input, ctx).await.unwrap();
        assert!(result.output.contains("## Skill: optimization"));
    }

    #[tokio::test]
    async fn test_reload_missing_name() {
        let tool = create_test_tool();
        let ctx = create_test_context();
        let input = json!({"action": "reload"});

        let result = tool.execute(input, ctx).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("name"));
    }

    #[tokio::test]
    async fn test_read_missing_name() {
        let tool = create_test_tool();
        let ctx = create_test_context();
        let input = json!({"action": "read"});

        let result = tool.execute(input, ctx).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("name"));
    }

    #[tokio::test]
    async fn test_reload_nonexistent() {
        let tool = create_test_tool();
        let ctx = create_test_context();
        let input = json!({"action": "reload", "name": "nonexistent"});

        let result = tool.execute(input, ctx).await.unwrap();
        assert!(result.output.contains("not found"));
    }

    #[tokio::test]
    async fn test_unknown_action() {
        let tool = create_test_tool();
        let ctx = create_test_context();
        let input = json!({"action": "invalid"});

        let result = tool.execute(input, ctx).await.unwrap();
        assert!(result.output.contains("Unknown action"));
    }

    #[tokio::test]
    async fn test_reload_all() {
        let tool = create_test_tool();
        let ctx = create_test_context();
        let input = json!({"action": "reload_all"});

        let result = tool.execute(input, ctx).await.unwrap();
        // The output format is "Reloaded N skills" where N is any number
        // (depends on what skills exist on the system)
        assert!(
            result.output.contains("Reloaded"),
            "Expected 'Reloaded' in output, got: {}",
            result.output
        );
        assert!(
            result.output.contains("skills"),
            "Expected 'skills' in output, got: {}",
            result.output
        );
    }

    fn context_with_working_dir(dir: &std::path::Path) -> ToolContext {
        ToolContext {
            working_dir: Some(dir.to_path_buf()),
            ..create_test_context()
        }
    }

    fn write_project_skill(root: &std::path::Path, name: &str) {
        let skill_dir = root.join(".agents").join("skills").join(name);
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: Project skill {name}\n---\n\nBody."),
        )
        .unwrap();
    }

    /// Issue #457: project-local skills must be session-scoped. Two contexts
    /// with different working dirs share one registry but must each see only
    /// their own project skills, immediately and without reload_all.
    #[tokio::test]
    async fn test_project_skills_are_scoped_to_tool_context_working_dir() {
        let tool = create_test_tool();
        let repo_a = tempfile::tempdir().unwrap();
        let repo_b = tempfile::tempdir().unwrap();
        write_project_skill(repo_a.path(), "repo-a-skill");
        write_project_skill(repo_b.path(), "repo-b-skill");

        // Immediately visible in each session without any reload.
        let list_a = tool
            .execute(
                json!({"action": "list"}),
                context_with_working_dir(repo_a.path()),
            )
            .await
            .unwrap();
        assert!(list_a.output.contains("repo-a-skill"));
        assert!(
            !list_a.output.contains("repo-b-skill"),
            "session A must not see session B's project skills"
        );

        let list_b = tool
            .execute(
                json!({"action": "list"}),
                context_with_working_dir(repo_b.path()),
            )
            .await
            .unwrap();
        assert!(list_b.output.contains("repo-b-skill"));
        assert!(!list_b.output.contains("repo-a-skill"));

        // reload_all in session A must not leak A's project skills into the
        // shared registry that session B reads.
        tool.execute(
            json!({"action": "reload_all"}),
            context_with_working_dir(repo_a.path()),
        )
        .await
        .unwrap();
        let shared = tool.registry.read().await;
        assert!(
            shared.get("repo-a-skill").is_none(),
            "shared registry must stay free of project-local skills"
        );
        drop(shared);

        // Skill file edits are visible without any reload/restart.
        let skill_md = repo_a.path().join(".agents/skills/repo-a-skill/SKILL.md");
        std::fs::write(
            &skill_md,
            "---\nname: repo-a-skill\ndescription: Updated description\n---\n\nNew body.",
        )
        .unwrap();
        let read = tool
            .execute(
                json!({"action": "read", "name": "repo-a-skill"}),
                context_with_working_dir(repo_a.path()),
            )
            .await
            .unwrap();
        assert!(
            read.output.contains("Updated description"),
            "skill edits must be visible without daemon restart, got: {}",
            read.output
        );
    }

    struct TestHome {
        previous: Option<std::ffi::OsString>,
        _dir: tempfile::TempDir,
    }

    impl TestHome {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let previous = std::env::var_os("JCODE_HOME");
            unsafe { std::env::set_var("JCODE_HOME", dir.path()) };
            Self {
                previous,
                _dir: dir,
            }
        }
    }

    impl Drop for TestHome {
        fn drop(&mut self) {
            if let Some(previous) = self.previous.take() {
                unsafe { std::env::set_var("JCODE_HOME", previous) };
            } else {
                unsafe { std::env::remove_var("JCODE_HOME") };
            }
        }
    }

    fn save_evidence_session(id: &str, text: &str) -> String {
        use crate::message::{ContentBlock, Role};
        let mut session = crate::session::Session::create_with_id(id.to_string(), None, None);
        let message_id = session.add_message(
            Role::User,
            vec![ContentBlock::Text {
                text: text.to_string(),
                cache_control: None,
            }],
        );
        session.save().unwrap();
        message_id
    }

    #[tokio::test]
    async fn proactive_discovery_requires_three_sessions_and_supports_all_controls() {
        let _env_lock = crate::storage::lock_test_env();
        let _home = TestHome::new();
        let workflow = "Run the release checklist, verify every artifact, and summarize failures.";
        save_evidence_session("discovery-session-a", workflow);
        save_evidence_session("discovery-session-b", workflow);
        let tool = create_test_tool();

        let none = tool
            .execute(
                json!({"action": "discover_crystallization"}),
                create_test_context(),
            )
            .await
            .unwrap();
        assert_eq!(none.metadata.unwrap()["status"], "no_suggestion");

        save_evidence_session("discovery-session-c", workflow);
        let suggested = tool
            .execute(
                json!({"action": "discover_crystallization"}),
                create_test_context(),
            )
            .await
            .unwrap();
        let metadata = suggested.metadata.as_ref().unwrap();
        assert_eq!(metadata["status"], "suggested");
        assert_eq!(metadata["evidence"].as_array().unwrap().len(), 3);
        assert!(suggested.output.contains("**Review**"));
        assert!(suggested.output.contains("**Dismiss**"));
        assert!(suggested.output.contains("**Never suggest this**"));
        assert!(metadata["actions"]["propose"].is_null());
        let first_id = metadata["suggestion_id"].as_str().unwrap().to_string();

        let reviewed = tool
            .execute(
                json!({
                    "action": "review_crystallization",
                    "suggestion_id": first_id
                }),
                create_test_context(),
            )
            .await
            .unwrap();
        let reviewed_metadata = reviewed.metadata.unwrap();
        assert_eq!(reviewed_metadata["status"], "reviewed");
        assert_eq!(
            reviewed_metadata["actions"]["propose"]["action"],
            "crystallize"
        );

        let dismissed = tool
            .execute(
                json!({
                    "action": "dismiss_crystallization",
                    "suggestion_id": first_id
                }),
                create_test_context(),
            )
            .await
            .unwrap();
        assert_eq!(dismissed.metadata.unwrap()["status"], "dismissed");
        let dismissed_scan = tool
            .execute(
                json!({"action": "discover_crystallization"}),
                create_test_context(),
            )
            .await
            .unwrap();
        assert_eq!(dismissed_scan.metadata.unwrap()["status"], "no_suggestion");

        save_evidence_session("discovery-session-d", workflow);
        let newer = tool
            .execute(
                json!({"action": "discover_crystallization"}),
                create_test_context(),
            )
            .await
            .unwrap();
        let newer_metadata = newer.metadata.as_ref().unwrap();
        assert_eq!(newer_metadata["status"], "suggested");
        assert_ne!(newer_metadata["suggestion_id"], first_id);
        let newer_id = newer_metadata["suggestion_id"]
            .as_str()
            .unwrap()
            .to_string();

        let suppressed = tool
            .execute(
                json!({
                    "action": "suppress_crystallization",
                    "suggestion_id": newer_id
                }),
                create_test_context(),
            )
            .await
            .unwrap();
        assert_eq!(suppressed.metadata.unwrap()["status"], "suppressed");
        save_evidence_session("discovery-session-e", workflow);
        let suppressed_scan = tool
            .execute(
                json!({"action": "discover_crystallization"}),
                create_test_context(),
            )
            .await
            .unwrap();
        assert_eq!(suppressed_scan.metadata.unwrap()["status"], "no_suggestion");
    }

    #[tokio::test]
    async fn proactive_discovery_rejects_unknown_or_tampered_suggestions() {
        let _env_lock = crate::storage::lock_test_env();
        let home = TestHome::new();
        let workflow = "Always run the focused migration check before changing the schema.";
        save_evidence_session("tamper-discovery-a", workflow);
        save_evidence_session("tamper-discovery-b", workflow);
        save_evidence_session("tamper-discovery-c", workflow);
        let tool = create_test_tool();
        let suggested = tool
            .execute(
                json!({"action": "discover_crystallization"}),
                create_test_context(),
            )
            .await
            .unwrap();
        let suggestion_id = suggested.metadata.unwrap()["suggestion_id"]
            .as_str()
            .unwrap()
            .to_string();
        let path = home
            ._dir
            .path()
            .join("skill-crystallization/discovery/suggestions")
            .join(format!("{suggestion_id}.json"));
        let persisted = std::fs::read_to_string(&path).unwrap();
        std::fs::write(
            &path,
            persisted.replace("focused migration check", "different migration check"),
        )
        .unwrap();
        let error = tool
            .execute(
                json!({
                    "action": "review_crystallization",
                    "suggestion_id": suggestion_id
                }),
                create_test_context(),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("fingerprint"));

        let unsafe_id = tool
            .execute(
                json!({
                    "action": "review_crystallization",
                    "suggestion_id": "../unsafe"
                }),
                create_test_context(),
            )
            .await
            .unwrap_err();
        assert!(unsafe_id.to_string().contains("64 lowercase hexadecimal"));
    }

    #[tokio::test]
    async fn proactive_discovery_ignores_jcode_control_plane_messages() {
        let _env_lock = crate::storage::lock_test_env();
        let _home = TestHome::new();
        for suffix in ["a", "b", "c"] {
            save_evidence_session(
                &format!("control-plane-{suffix}"),
                "[auto] Quality checks passed. Give the user a concise final response now.",
            );
        }
        let output = create_test_tool()
            .execute(
                json!({"action": "discover_crystallization"}),
                create_test_context(),
            )
            .await
            .unwrap();
        assert_eq!(output.metadata.unwrap()["status"], "no_suggestion");
    }

    #[tokio::test]
    async fn crystallization_public_workflow_requires_confirmation_and_reloads() {
        let _env_lock = crate::storage::lock_test_env();
        let home = TestHome::new();
        let first = save_evidence_session(
            "crystal-session-a",
            "Repeated workflow evidence A and private unrelated tail.",
        );
        let second = save_evidence_session("crystal-session-b", "Repeated workflow evidence B.");
        let registry = Arc::new(RwLock::new(SkillRegistry::load_global().unwrap()));
        let tool = SkillTool::new(registry);
        let input = json!({
            "action": "crystallize",
            "name": "repeatable-check",
            "description": "Run a repeatable verified check.",
            "content": "# Repeatable check\n\nRun the focused verification.",
            "evidence": [
                {"session_id": "crystal-session-b", "message_id": second},
                {"session_id": "crystal-session-a", "message_id": first}
            ]
        });
        let proposed = tool
            .execute(input.clone(), create_test_context())
            .await
            .unwrap();
        let metadata = proposed.metadata.as_ref().unwrap();
        let proposal_id = metadata["proposal_id"].as_str().unwrap();
        assert_eq!(metadata["status"], "pending_confirmation");
        assert!(
            !home
                ._dir
                .path()
                .join("skills/repeatable-check/SKILL.md")
                .exists()
        );
        assert!(proposed.output.contains("No skill was created"));

        let not_found = tool
            .execute(
                json!({"action": "read", "name": "repeatable-check"}),
                create_test_context(),
            )
            .await
            .unwrap();
        assert!(not_found.output.contains("not found"));

        let unconfirmed = tool
            .execute(
                json!({
                    "action": "approve_crystallization",
                    "proposal_id": proposal_id
                }),
                create_test_context(),
            )
            .await
            .unwrap();
        assert_eq!(unconfirmed.metadata.unwrap()["installed"], false);
        assert!(
            !home
                ._dir
                .path()
                .join("skills/repeatable-check/SKILL.md")
                .exists()
        );

        let missing_approval = tool
            .execute(
                json!({
                    "action": "approve_crystallization",
                    "proposal_id": proposal_id,
                    "confirmed": true
                }),
                create_test_context(),
            )
            .await
            .unwrap_err();
        assert!(missing_approval.to_string().contains("approval_evidence"));

        let rejected_approval = save_evidence_session(
            "crystal-rejected-approval",
            &format!("Do not approve skill crystallization proposal {proposal_id}."),
        );
        let rejected = tool
            .execute(
                json!({
                    "action": "approve_crystallization",
                    "proposal_id": proposal_id,
                    "confirmed": true,
                    "approval_evidence": {
                        "session_id": "crystal-rejected-approval",
                        "message_id": rejected_approval
                    }
                }),
                create_test_context(),
            )
            .await
            .unwrap_err();
        assert!(rejected.to_string().contains("must exactly match"));

        let staging = home
            ._dir
            .path()
            .join("skills")
            .join(format!(".crystallize-{proposal_id}"));
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(
            staging.join("SKILL.md"),
            metadata["preview"].as_str().unwrap(),
        )
        .unwrap();

        let approval_message = save_evidence_session(
            "crystal-approval-session",
            &format!("I approve skill crystallization proposal {proposal_id}."),
        );
        let approved = tool
            .execute(
                json!({
                    "action": "approve_crystallization",
                    "proposal_id": proposal_id,
                    "confirmed": true,
                    "approval_evidence": {
                        "session_id": "crystal-approval-session",
                        "message_id": approval_message
                    }
                }),
                create_test_context(),
            )
            .await
            .unwrap();
        assert_eq!(approved.metadata.as_ref().unwrap()["status"], "installed");
        let installed =
            std::fs::read_to_string(home._dir.path().join("skills/repeatable-check/SKILL.md"))
                .unwrap();
        assert_eq!(installed, metadata["preview"].as_str().unwrap());

        let read = tool
            .execute(
                json!({"action": "read", "name": "repeatable-check"}),
                create_test_context(),
            )
            .await
            .unwrap();
        assert!(read.output.contains("Run the focused verification"));
        assert!(
            tool.execute(input, create_test_context())
                .await
                .unwrap_err()
                .to_string()
                .contains("already exists")
        );
        assert!(
            tool.execute(
                json!({
                    "action": "approve_crystallization",
                    "proposal_id": proposal_id,
                    "confirmed": true,
                    "approval_evidence": {
                        "session_id": "crystal-approval-session",
                        "message_id": approval_message
                    }
                }),
                create_test_context()
            )
            .await
            .is_err()
        );
    }

    #[tokio::test]
    async fn crystallization_rejects_unsafe_and_unverified_evidence() {
        let _env_lock = crate::storage::lock_test_env();
        let _home = TestHome::new();
        let tool = create_test_tool();
        let error = tool
            .execute(
                json!({
                    "action": "crystallize",
                    "name": "../unsafe",
                    "description": "Description",
                    "content": "Body",
                    "evidence": [
                        {"session_id": "../session", "message_id": "a"},
                        {"session_id": "other", "message_id": "b"}
                    ]
                }),
                create_test_context(),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("lowercase kebab-case"));
    }

    #[tokio::test]
    async fn crystallization_detects_full_message_changes_outside_the_excerpt() {
        let _env_lock = crate::storage::lock_test_env();
        let _home = TestHome::new();
        let long_prefix = "stable ".repeat(100);
        let first = save_evidence_session("digest-session-a", &format!("{long_prefix}tail-a"));
        let second = save_evidence_session("digest-session-b", "Second repeated workflow example.");
        let tool = SkillTool::new(Arc::new(RwLock::new(SkillRegistry::load_global().unwrap())));
        let proposed = tool
            .execute(
                json!({
                    "action": "crystallize",
                    "name": "digest-bound-workflow",
                    "description": "Capture a digest-bound workflow.",
                    "content": "# Digest-bound workflow\n\nRun the verified steps.",
                    "evidence": [
                        {"session_id": "digest-session-a", "message_id": first},
                        {"session_id": "digest-session-b", "message_id": second}
                    ]
                }),
                create_test_context(),
            )
            .await
            .unwrap();
        let proposal_id = proposed.metadata.unwrap()["proposal_id"]
            .as_str()
            .unwrap()
            .to_string();

        let session_path = crate::session::session_path("digest-session-a").unwrap();
        let persisted = std::fs::read_to_string(&session_path).unwrap();
        assert!(persisted.contains("tail-a"));
        std::fs::write(&session_path, persisted.replace("tail-a", "tail-b")).unwrap();
        let approval = save_evidence_session(
            "digest-approval-session",
            &format!("I approve skill crystallization proposal {proposal_id}."),
        );
        let error = tool
            .execute(
                json!({
                    "action": "approve_crystallization",
                    "proposal_id": proposal_id,
                    "confirmed": true,
                    "approval_evidence": {
                        "session_id": "digest-approval-session",
                        "message_id": approval
                    }
                }),
                create_test_context(),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("evidence changed"));
    }

    #[tokio::test]
    async fn concurrent_crystallization_proposals_are_serialized_and_deduplicated() {
        let _env_lock = crate::storage::lock_test_env();
        let _home = TestHome::new();
        let first = save_evidence_session("concurrent-session-a", "Repeated workflow A.");
        let second = save_evidence_session("concurrent-session-b", "Repeated workflow B.");
        let tool = Arc::new(SkillTool::new(Arc::new(RwLock::new(
            SkillRegistry::load_global().unwrap(),
        ))));
        let input = json!({
            "action": "crystallize",
            "name": "serialized-workflow",
            "description": "Serialize proposal creation.",
            "content": "# Serialized workflow\n\nRun once.",
            "evidence": [
                {"session_id": "concurrent-session-a", "message_id": first},
                {"session_id": "concurrent-session-b", "message_id": second}
            ]
        });
        let (left, right) = tokio::join!(
            tool.execute(input.clone(), create_test_context()),
            tool.execute(input, create_test_context())
        );
        assert_eq!(usize::from(left.is_ok()) + usize::from(right.is_ok()), 1);
        let error = left.err().or_else(|| right.err()).unwrap();
        assert!(error.to_string().contains("pending proposal"));
    }
}
