use super::ToolOutput;
use super::crystallization::{EvidenceReference, acquire_operation_lock};
use crate::session::{Session, durable_conversation_evidence_text};
use crate::skill::SkillRegistry;
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use jcode_message_types::{ContentBlock, Role};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;

const SCHEMA_VERSION: u32 = 1;
const MAX_ID_BYTES: usize = 256;
const MAX_NAME_BYTES: usize = 64;
const MAX_RATIONALE_CHARS: usize = 500;
const MAX_SKILL_BYTES: u64 = 256 * 1024;
const MAX_RECORD_BYTES: u64 = 512 * 1024;
const MIN_CONFIDENCE: f64 = 0.80;
const MAX_SCAN: usize = 2_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct MessageDigest {
    pub message_id: String,
    pub digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct UsageRecord {
    pub schema_version: u32,
    pub usage_id: String,
    pub session_id: String,
    pub load_message_id: String,
    pub load_tool_call_id: String,
    pub skill_name: String,
    pub canonical_path: PathBuf,
    pub skill_fingerprint: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OutcomeClass {
    Helped,
    Corrected,
    Replaced,
    Unused,
}

impl OutcomeClass {
    fn as_str(self) -> &'static str {
        match self {
            Self::Helped => "helped",
            Self::Corrected => "corrected",
            Self::Replaced => "replaced",
            Self::Unused => "unused",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self> {
        match value {
            "helped" => Ok(Self::Helped),
            "corrected" => Ok(Self::Corrected),
            "replaced" => Ok(Self::Replaced),
            "unused" => Ok(Self::Unused),
            _ => bail!("outcome must be helped, corrected, replaced, or unused"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct OutcomeRecord {
    pub schema_version: u32,
    pub outcome_id: String,
    pub usage_id: String,
    pub session_id: String,
    pub outcome_message_id: String,
    pub outcome_tool_call_id: String,
    pub outcome: OutcomeClass,
    pub confidence: f64,
    pub rationale: String,
    pub related_skill: Option<String>,
    pub evidence_window: Vec<MessageDigest>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EvolutionKind {
    Refine,
    Merge,
    Retire,
}

impl EvolutionKind {
    pub(crate) fn parse(value: &str) -> Result<Self> {
        match value {
            "refine" => Ok(Self::Refine),
            "merge" => Ok(Self::Merge),
            "retire" => Ok(Self::Retire),
            _ => bail!("evolution_kind must be refine, merge, or retire"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct EvolutionSuggestion {
    pub schema_version: u32,
    pub suggestion_id: String,
    pub kind: EvolutionKind,
    pub source_names: Vec<String>,
    pub source_fingerprints: BTreeMap<String, String>,
    pub outcome_ids: Vec<String>,
    pub evidence_digest: String,
    pub summary: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct EvolutionProposal {
    pub schema_version: u32,
    pub proposal_id: String,
    pub suggestion_id: String,
    pub kind: EvolutionKind,
    pub source_names: Vec<String>,
    pub source_fingerprints: BTreeMap<String, String>,
    pub destination_name: Option<String>,
    pub proposed_content: Option<String>,
    pub proposed_fingerprint: Option<String>,
    pub outcome_ids: Vec<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum TransactionPhase {
    Staged,
    SourcesArchived,
    DestinationInstalling,
    DestinationInstalled,
    RegistryVerified,
    Finalized,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EvolutionTransaction {
    schema_version: u32,
    proposal_id: String,
    phase: TransactionPhase,
    source_names: Vec<String>,
    destination_name: Option<String>,
    destination_fingerprint: Option<String>,
    stage_path: Option<PathBuf>,
    archives: BTreeMap<String, PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DiscoveryCandidate {
    pub kind: EvolutionKind,
    pub source_names: Vec<String>,
    pub outcome_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MutationResult {
    pub proposal_id: String,
    pub kind: EvolutionKind,
    pub source_names: Vec<String>,
    pub destination_name: Option<String>,
}

pub(crate) type Suggestion = EvolutionSuggestion;

pub(crate) enum AutomaticRefresh {
    RateLimited,
    Empty,
    Unchanged(Suggestion),
    New(Suggestion),
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct InboxState {
    dismissed: BTreeSet<String>,
    suppressed_patterns: BTreeSet<String>,
    last_surfaced_suggestion_id: Option<String>,
    last_refresh_at: Option<DateTime<Utc>>,
}

pub(crate) fn refresh_automatic(interval: std::time::Duration) -> Result<AutomaticRefresh> {
    let mut state = load_inbox_state()?;
    let due = state.last_refresh_at.is_none_or(|last| {
        Utc::now()
            .signed_duration_since(last)
            .to_std()
            .unwrap_or_default()
            >= interval
    });
    if !due {
        return Ok(AutomaticRefresh::RateLimited);
    }
    let newest = discover_verified()?.into_iter().max_by(|a, b| {
        a.created_at
            .cmp(&b.created_at)
            .then_with(|| b.suggestion_id.cmp(&a.suggestion_id))
    });
    state.last_refresh_at = Some(Utc::now());
    let Some(suggestion) = newest else {
        persist_inbox_state(&state)?;
        return Ok(AutomaticRefresh::Empty);
    };
    let unchanged = state.last_surfaced_suggestion_id.as_deref() == Some(&suggestion.suggestion_id);
    persist_inbox_state(&state)?;
    Ok(if unchanged {
        AutomaticRefresh::Unchanged(suggestion)
    } else {
        AutomaticRefresh::New(suggestion)
    })
}

pub(crate) fn latest_pending() -> Result<Option<Suggestion>> {
    let state = load_inbox_state()?;
    let mut suggestions = read_recent_records::<EvolutionSuggestion>(&suggestion_dir()?, MAX_SCAN)?;
    suggestions.retain(|item| {
        !state.dismissed.contains(&item.suggestion_id)
            && !state
                .suppressed_patterns
                .contains(&suggestion_pattern(item))
            && revalidate_suggestion(item).is_ok()
    });
    suggestions.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| b.suggestion_id.cmp(&a.suggestion_id))
    });
    Ok(suggestions.into_iter().next())
}

pub(crate) fn review(id: &str) -> Result<Suggestion> {
    let suggestion = load_suggestion(id)?;
    revalidate_suggestion(&suggestion)?;
    Ok(suggestion)
}

pub(crate) fn dismiss(id: &str) -> Result<Suggestion> {
    let suggestion = review(id)?;
    let mut state = load_inbox_state()?;
    state.dismissed.insert(id.to_string());
    persist_inbox_state(&state)?;
    Ok(suggestion)
}

pub(crate) fn suppress(id: &str) -> Result<Suggestion> {
    let stored = load_suggestion(id)?;
    revalidate_suggestion(&stored)?;
    let mut state = load_inbox_state()?;
    state
        .suppressed_patterns
        .insert(suggestion_pattern(&stored));
    persist_inbox_state(&state)?;
    Ok(stored)
}

pub(crate) fn mark_surfaced(id: &str) -> Result<()> {
    review(id)?;
    let mut state = load_inbox_state()?;
    state.last_surfaced_suggestion_id = Some(id.to_string());
    persist_inbox_state(&state)
}

pub(crate) fn suggestion_output(suggestion: &Suggestion, state: &str) -> ToolOutput {
    let evidence = suggestion
        .outcome_ids
        .iter()
        .take(6)
        .filter_map(|id| load_outcome(id).ok())
        .map(|outcome| {
            format!(
                "- {:?} in session {}: {}",
                outcome.outcome, outcome.session_id, outcome.rationale
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    ToolOutput::new(format!(
        "Skill evolution suggestion {:?}: {}\n\nSources: {}\nVerified outcomes: {}\n\nEvidence:\n{}",
        suggestion.kind,
        suggestion.summary,
        suggestion
            .source_names
            .iter()
            .map(|name| format!("/{name}"))
            .collect::<Vec<_>>()
            .join(", "),
        suggestion.outcome_ids.len(),
        evidence
    ))
    .with_title("Skill evolution suggestion")
    .with_metadata(json!({
        "schema_version": 1,
        "status": state,
        "suggestion_id": suggestion.suggestion_id,
        "kind": suggestion.kind,
        "source_names": suggestion.source_names,
        "evidence_count": suggestion.outcome_ids.len()
    }))
}

pub(crate) fn state_output(suggestion: &Suggestion, state: &str) -> ToolOutput {
    ToolOutput::new(if state == "dismissed" {
        "This exact skill evolution evidence snapshot was dismissed."
    } else {
        "This skill evolution action and target pattern was suppressed."
    })
    .with_title("Skill evolution state")
    .with_metadata(json!({
        "schema_version": 1,
        "status": state,
        "suggestion_id": suggestion.suggestion_id,
        "kind": suggestion.kind
    }))
}

pub(crate) fn review_prompt(suggestion: &Suggestion) -> String {
    let instruction = match suggestion.kind {
        EvolutionKind::Refine => {
            "Draft an exact replacement SKILL.md including frontmatter. Do not mutate files."
        }
        EvolutionKind::Merge => {
            "Draft one canonical destination SKILL.md and identify both sources. Do not mutate files."
        }
        EvolutionKind::Retire => {
            "Explain the verified retirement case and propose archival. Do not mutate files."
        }
    };
    format!(
        "Review immutable skill evolution suggestion {} for sources {}. {} Read the source skills, then call skill_manage propose_skill_evolution with suggestion_id, evolution_kind, source_names, and the exact destination_name/proposed_content required by the action. Do not approve or mutate any skill.",
        suggestion.suggestion_id,
        suggestion.source_names.join(", "),
        instruction
    )
}

pub(crate) fn canonical_skill_path(name: &str) -> Result<PathBuf> {
    let name = validate_name(name)?;
    Ok(crate::storage::jcode_dir()?
        .join("skills")
        .join(name)
        .join("SKILL.md"))
}

pub(crate) fn fingerprint_raw_skill(raw: &str) -> String {
    hex_digest(normalize_raw(raw).as_bytes())
}

pub(crate) fn verified_canonical_skill(
    name: &str,
    loaded_path: &Path,
) -> Result<(PathBuf, String)> {
    let expected = canonical_skill_path(name)?;
    reject_symlink_components(&expected)?;
    if loaded_path != expected {
        bail!("Loaded skill is not the canonical global ~/.jcode skill");
    }
    let metadata = fs::symlink_metadata(&expected)
        .with_context(|| format!("Canonical skill '{}' is missing", expected.display()))?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_SKILL_BYTES {
        bail!("Canonical SKILL.md must be a bounded regular file");
    }
    let raw = read_bounded(&expected, MAX_SKILL_BYTES)?;
    Ok((expected, fingerprint_raw_skill(&raw)))
}

pub(crate) fn record_usage(
    session_id: &str,
    message_id: &str,
    tool_call_id: &str,
    skill_name: &str,
    loaded_path: &Path,
) -> Result<UsageRecord> {
    validate_component("session_id", session_id)?;
    validate_component("message_id", message_id)?;
    validate_component("tool_call_id", tool_call_id)?;
    let skill_name = validate_name(skill_name)?;
    let session = load_ordinary_session(session_id)?;
    verify_direct_tool_use(
        &session,
        message_id,
        tool_call_id,
        "load",
        Some(&skill_name),
    )?;
    let (canonical_path, skill_fingerprint) = verified_canonical_skill(&skill_name, loaded_path)?;
    let mut record = UsageRecord {
        schema_version: SCHEMA_VERSION,
        usage_id: String::new(),
        session_id: session_id.to_string(),
        load_message_id: message_id.to_string(),
        load_tool_call_id: tool_call_id.to_string(),
        skill_name,
        canonical_path,
        skill_fingerprint,
        created_at: Utc::now(),
    };
    record.usage_id = content_id(&UsageIdentity::from(&record))?;
    if let Some(existing) = existing_content_addressed(&usage_dir()?, &record.usage_id)? {
        revalidate_usage(&existing)?;
        return Ok(existing);
    }
    write_immutable(&usage_dir()?, &record.usage_id, &record)?;
    Ok(record)
}

pub(crate) fn record_outcome(
    session_id: &str,
    outcome_message_id: &str,
    outcome_tool_call_id: &str,
    usage_id: &str,
    outcome: OutcomeClass,
    confidence: f64,
    rationale: &str,
    related_skill: Option<&str>,
) -> Result<OutcomeRecord> {
    validate_hash_id("usage_id", usage_id)?;
    validate_component("session_id", session_id)?;
    validate_component("message_id", outcome_message_id)?;
    validate_component("tool_call_id", outcome_tool_call_id)?;
    if !confidence.is_finite() || !(0.0..=1.0).contains(&confidence) {
        bail!("confidence must be a finite number in [0, 1]");
    }
    let rationale = rationale.trim();
    if rationale.is_empty() || rationale.chars().count() > MAX_RATIONALE_CHARS {
        bail!("rationale must be 1 to {MAX_RATIONALE_CHARS} characters");
    }
    let related_skill = match (outcome, related_skill) {
        (OutcomeClass::Replaced, Some(name)) => Some(validate_name(name)?),
        (OutcomeClass::Replaced, None) => None,
        (_, Some(_)) => bail!("related_skill is allowed only when outcome is replaced"),
        (_, None) => None,
    };
    if let Some(name) = related_skill.as_deref() {
        let path = canonical_skill_path(name)?;
        verified_canonical_skill(name, &path)?;
    }
    let usage = load_usage(usage_id)?;
    if usage.session_id != session_id {
        bail!("Usage belongs to a different session");
    }
    if related_skill.as_deref() == Some(usage.skill_name.as_str()) {
        bail!("related_skill must differ from the loaded skill");
    }
    revalidate_usage(&usage)?;
    let session = load_ordinary_session(session_id)?;
    let persisted_input = verify_direct_tool_use(
        &session,
        outcome_message_id,
        outcome_tool_call_id,
        "record_skill_outcome",
        None,
    )?;
    let persisted_related = persisted_input
        .get("related_skill")
        .and_then(|value| value.as_str());
    if persisted_input
        .get("usage_id")
        .and_then(|value| value.as_str())
        != Some(usage_id)
        || persisted_input
            .get("outcome")
            .and_then(|value| value.as_str())
            != Some(outcome.as_str())
        || persisted_input
            .get("confidence")
            .and_then(|value| value.as_f64())
            != Some(confidence)
        || persisted_input
            .get("rationale")
            .and_then(|value| value.as_str())
            != Some(rationale)
        || persisted_related != related_skill.as_deref()
    {
        bail!("Persisted skill outcome arguments do not match the recorded evidence");
    }
    let load_index = message_index(&session, &usage.load_message_id)?;
    let outcome_index = message_index(&session, outcome_message_id)?;
    if outcome_index <= load_index {
        bail!("Outcome must be in a later distinct assistant message than the load");
    }
    let evidence_window = digest_window(&session, load_index + 1, outcome_index)?;
    let mut record = OutcomeRecord {
        schema_version: SCHEMA_VERSION,
        outcome_id: String::new(),
        usage_id: usage_id.to_string(),
        session_id: session_id.to_string(),
        outcome_message_id: outcome_message_id.to_string(),
        outcome_tool_call_id: outcome_tool_call_id.to_string(),
        outcome,
        confidence,
        rationale: rationale.to_string(),
        related_skill,
        evidence_window,
        created_at: Utc::now(),
    };
    record.outcome_id = content_id(&OutcomeIdentity::from(&record))?;
    if let Some(existing) = existing_content_addressed(&outcome_dir()?, &record.outcome_id)? {
        revalidate_outcome(&existing)?;
        return Ok(existing);
    }
    write_immutable(&outcome_dir()?, &record.outcome_id, &record)?;
    Ok(record)
}

pub(crate) fn revalidate_outcome(record: &OutcomeRecord) -> Result<()> {
    validate_hash_id("outcome_id", &record.outcome_id)?;
    if content_id(&OutcomeIdentity::from(record))? != record.outcome_id {
        bail!("Outcome record content address does not match its contents");
    }
    let usage = load_usage(&record.usage_id)?;
    revalidate_usage(&usage)?;
    let session = load_ordinary_session(&record.session_id)?;
    verify_direct_tool_use(
        &session,
        &record.outcome_message_id,
        &record.outcome_tool_call_id,
        "record_skill_outcome",
        None,
    )?;
    let load_index = message_index(&session, &usage.load_message_id)?;
    let outcome_index = message_index(&session, &record.outcome_message_id)?;
    if outcome_index <= load_index {
        bail!("Outcome must follow the load in a distinct assistant message");
    }
    if digest_window(&session, load_index + 1, outcome_index)? != record.evidence_window {
        bail!("Persisted outcome evidence window changed");
    }
    Ok(())
}

pub(crate) fn discovery_candidates(outcomes: &[OutcomeRecord]) -> Vec<DiscoveryCandidate> {
    let mut by_skill: BTreeMap<String, Vec<&OutcomeRecord>> = BTreeMap::new();
    for outcome in outcomes
        .iter()
        .filter(|item| item.confidence >= MIN_CONFIDENCE)
    {
        if let Ok(usage) = load_usage(&outcome.usage_id) {
            by_skill.entry(usage.skill_name).or_default().push(outcome);
        }
    }
    candidates_from_grouped(by_skill)
}

pub(crate) fn discover_verified() -> Result<Vec<EvolutionSuggestion>> {
    let mut records = read_recent_records::<OutcomeRecord>(&outcome_dir()?, MAX_SCAN)?;
    records.retain(|record| revalidate_outcome(record).is_ok());
    let mut suggestions = Vec::new();
    for candidate in discovery_candidates(&records) {
        suggestions.push(persist_suggestion(candidate)?);
    }
    Ok(suggestions)
}

pub(crate) fn load_suggestion(id: &str) -> Result<EvolutionSuggestion> {
    load_content_addressed(&suggestion_dir()?, id)
}

pub(crate) fn propose(
    suggestion_id: &str,
    kind: EvolutionKind,
    source_names: Vec<String>,
    destination_name: Option<String>,
    proposed_content: Option<String>,
) -> Result<EvolutionProposal> {
    let suggestion = load_suggestion(suggestion_id)?;
    revalidate_suggestion(&suggestion)?;
    if suggestion.kind != kind {
        bail!("Evolution kind does not match the immutable suggestion");
    }
    let source_names = normalize_names(source_names)?;
    if source_names != suggestion.source_names {
        bail!("Source names do not match the immutable suggestion");
    }
    let (destination_name, proposed_content, proposed_fingerprint) =
        validate_proposed_mutation(kind, &source_names, destination_name, proposed_content)?;
    let mut proposal = EvolutionProposal {
        schema_version: SCHEMA_VERSION,
        proposal_id: String::new(),
        suggestion_id: suggestion_id.to_string(),
        kind,
        source_names,
        source_fingerprints: suggestion.source_fingerprints,
        destination_name,
        proposed_content,
        proposed_fingerprint,
        outcome_ids: suggestion.outcome_ids,
        created_at: Utc::now(),
    };
    proposal.proposal_id = content_id(&ProposalIdentity::from(&proposal))?;
    if let Some(existing) = existing_content_addressed(&proposal_dir()?, &proposal.proposal_id)? {
        revalidate_proposal(&existing)?;
        return Ok(existing);
    }
    write_immutable(&proposal_dir()?, &proposal.proposal_id, &proposal)?;
    Ok(proposal)
}

pub(crate) fn load_proposal(id: &str) -> Result<EvolutionProposal> {
    load_content_addressed(&proposal_dir()?, id)
}

pub(crate) fn verify_approval(
    proposal: &EvolutionProposal,
    reference: &EvidenceReference,
) -> Result<()> {
    validate_component("approval session_id", &reference.session_id)?;
    validate_component("approval message_id", &reference.message_id)?;
    let session = Session::load(&reference.session_id)?;
    if session.id != reference.session_id || session.is_debug {
        bail!("Approval must come from the requested ordinary persisted session");
    }
    let message = session
        .messages
        .iter()
        .find(|message| message.id == reference.message_id)
        .context("Approval message was not found")?;
    if message.role != Role::User {
        bail!("Approval evidence must reference a persisted user message");
    }
    if message
        .timestamp
        .context("Approval message needs a timestamp")?
        <= proposal.created_at
    {
        bail!("Approval message predates the proposal");
    }
    let actual = durable_conversation_evidence_text(message)
        .context("Approval must be an ordinary durable conversation message")?
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let expected = format!(
        "I approve skill evolution proposal {}.",
        proposal.proposal_id
    );
    if actual != expected {
        bail!("Approval text must exactly equal: {expected}");
    }
    Ok(())
}

#[cfg(test)]
pub(crate) async fn approve(
    registry: &Arc<RwLock<SkillRegistry>>,
    proposal_id: &str,
    confirmed: bool,
    approval: &EvidenceReference,
) -> Result<MutationResult> {
    let _file_lock = acquire_operation_lock()?;
    approve_unlocked(registry, proposal_id, confirmed, approval).await
}

pub(crate) async fn approve_unlocked(
    registry: &Arc<RwLock<SkillRegistry>>,
    proposal_id: &str,
    confirmed: bool,
    approval: &EvidenceReference,
) -> Result<MutationResult> {
    if !confirmed {
        bail!("confirmed=true is required");
    }
    recover_incomplete(registry).await?;
    let proposal = load_proposal(proposal_id)?;
    revalidate_proposal(&proposal)?;
    verify_approval(&proposal, approval)?;
    apply_transaction(registry, &proposal).await?;
    Ok(MutationResult {
        proposal_id: proposal.proposal_id,
        kind: proposal.kind,
        source_names: proposal.source_names,
        destination_name: proposal.destination_name,
    })
}

pub(crate) async fn recover_if_needed(registry: &Arc<RwLock<SkillRegistry>>) -> Result<()> {
    if !transaction_path()?.exists() {
        return Ok(());
    }
    let _file_lock = acquire_operation_lock()?;
    recover_incomplete(registry).await
}

fn candidates_from_grouped(
    by_skill: BTreeMap<String, Vec<&OutcomeRecord>>,
) -> Vec<DiscoveryCandidate> {
    let mut result = Vec::new();
    for (skill, records) in by_skill {
        let mut latest_by_session = BTreeMap::<&str, &OutcomeRecord>::new();
        for record in records {
            latest_by_session
                .entry(&record.session_id)
                .and_modify(|current| {
                    if (record.created_at, &record.outcome_id)
                        > (current.created_at, &current.outcome_id)
                    {
                        *current = record;
                    }
                })
                .or_insert(record);
        }
        let records = latest_by_session.into_values().collect::<Vec<_>>();
        let distinct = |predicate: &dyn Fn(&OutcomeRecord) -> bool| {
            let mut sessions = BTreeSet::new();
            let mut ids = Vec::new();
            for record in records.iter().copied().filter(|record| predicate(record)) {
                if sessions.insert(record.session_id.clone()) {
                    ids.push(record.outcome_id.clone());
                }
            }
            ids.sort();
            ids
        };
        let corrected = distinct(&|record| record.outcome == OutcomeClass::Corrected);
        if corrected.len() >= 3 {
            result.push(DiscoveryCandidate {
                kind: EvolutionKind::Refine,
                source_names: vec![skill.clone()],
                outcome_ids: corrected,
            });
        }
        let mut replacements: BTreeMap<String, Vec<&OutcomeRecord>> = BTreeMap::new();
        for record in &records {
            if record.outcome == OutcomeClass::Replaced
                && let Some(related) = record.related_skill.as_ref()
                && related != &skill
            {
                replacements
                    .entry(related.clone())
                    .or_default()
                    .push(record);
            }
        }
        let mut stable_replacement = false;
        for (related, replacement_records) in replacements {
            let mut sessions = BTreeSet::new();
            let mut ids = Vec::new();
            for record in replacement_records {
                if sessions.insert(record.session_id.clone()) {
                    ids.push(record.outcome_id.clone());
                }
            }
            if ids.len() >= 3 {
                stable_replacement = true;
                ids.sort();
                let mut names = vec![skill.clone(), related];
                names.sort();
                result.push(DiscoveryCandidate {
                    kind: EvolutionKind::Merge,
                    source_names: names,
                    outcome_ids: ids,
                });
            }
        }
        let negative = distinct(&|record| {
            matches!(
                record.outcome,
                OutcomeClass::Replaced | OutcomeClass::Unused
            )
        });
        let helped = records.iter().any(|record| {
            record.outcome == OutcomeClass::Helped && record.confidence >= MIN_CONFIDENCE
        });
        if negative.len() >= 5 && !stable_replacement && !helped {
            result.push(DiscoveryCandidate {
                kind: EvolutionKind::Retire,
                source_names: vec![skill],
                outcome_ids: negative,
            });
        }
    }
    result
}

fn persist_suggestion(candidate: DiscoveryCandidate) -> Result<EvolutionSuggestion> {
    let mut fingerprints = BTreeMap::new();
    for name in &candidate.source_names {
        let path = canonical_skill_path(name)?;
        let (_, fingerprint) = verified_canonical_skill(name, &path)?;
        fingerprints.insert(name.clone(), fingerprint);
    }
    let evidence_digest = content_id(&candidate.outcome_ids)?;
    let summary = match candidate.kind {
        EvolutionKind::Refine => format!(
            "Refine /{} from repeated corrections",
            candidate.source_names[0]
        ),
        EvolutionKind::Merge => format!(
            "Merge /{} and /{} from repeated replacement evidence",
            candidate.source_names[0], candidate.source_names[1]
        ),
        EvolutionKind::Retire => format!(
            "Retire /{} from repeated unused or replacement evidence",
            candidate.source_names[0]
        ),
    };
    let mut suggestion = EvolutionSuggestion {
        schema_version: SCHEMA_VERSION,
        suggestion_id: String::new(),
        kind: candidate.kind,
        source_names: candidate.source_names,
        source_fingerprints: fingerprints,
        outcome_ids: candidate.outcome_ids,
        evidence_digest,
        summary,
        created_at: Utc::now(),
    };
    suggestion.suggestion_id = content_id(&SuggestionIdentity::from(&suggestion))?;
    if let Some(existing) =
        existing_content_addressed(&suggestion_dir()?, &suggestion.suggestion_id)?
    {
        revalidate_suggestion(&existing)?;
        return Ok(existing);
    }
    write_immutable(&suggestion_dir()?, &suggestion.suggestion_id, &suggestion)?;
    Ok(suggestion)
}

fn revalidate_usage(record: &UsageRecord) -> Result<()> {
    if content_id(&UsageIdentity::from(record))? != record.usage_id {
        bail!("Usage record content address does not match its contents");
    }
    let session = load_ordinary_session(&record.session_id)?;
    verify_direct_tool_use(
        &session,
        &record.load_message_id,
        &record.load_tool_call_id,
        "load",
        Some(&record.skill_name),
    )?;
    let (path, fingerprint) = verified_canonical_skill(&record.skill_name, &record.canonical_path)?;
    if path != record.canonical_path || fingerprint != record.skill_fingerprint {
        bail!("Canonical raw SKILL.md fingerprint changed");
    }
    Ok(())
}

fn revalidate_suggestion(suggestion: &EvolutionSuggestion) -> Result<()> {
    if content_id(&SuggestionIdentity::from(suggestion))? != suggestion.suggestion_id {
        bail!("Suggestion content address does not match its contents");
    }
    if content_id(&suggestion.outcome_ids)? != suggestion.evidence_digest {
        bail!("Suggestion evidence digest changed");
    }
    for outcome_id in &suggestion.outcome_ids {
        revalidate_outcome(&load_outcome(outcome_id)?)?;
    }
    for (name, expected) in &suggestion.source_fingerprints {
        let path = canonical_skill_path(name)?;
        let (_, actual) = verified_canonical_skill(name, &path)?;
        if &actual != expected {
            bail!("Suggestion source fingerprint changed for '{name}'");
        }
    }
    Ok(())
}

fn revalidate_proposal(proposal: &EvolutionProposal) -> Result<()> {
    if content_id(&ProposalIdentity::from(proposal))? != proposal.proposal_id {
        bail!("Proposal content address does not match its contents");
    }
    let suggestion = load_suggestion(&proposal.suggestion_id)?;
    revalidate_suggestion(&suggestion)?;
    if suggestion.kind != proposal.kind
        || suggestion.source_names != proposal.source_names
        || suggestion.source_fingerprints != proposal.source_fingerprints
        || suggestion.outcome_ids != proposal.outcome_ids
    {
        bail!("Proposal no longer matches its immutable suggestion");
    }
    for (name, expected) in &proposal.source_fingerprints {
        let path = canonical_skill_path(name)?;
        let (_, actual) = verified_canonical_skill(name, &path)?;
        if &actual != expected {
            bail!("Current source fingerprint changed for '{name}'");
        }
    }
    validate_proposed_mutation(
        proposal.kind,
        &proposal.source_names,
        proposal.destination_name.clone(),
        proposal.proposed_content.clone(),
    )?;
    Ok(())
}

fn validate_proposed_mutation(
    kind: EvolutionKind,
    sources: &[String],
    destination: Option<String>,
    content: Option<String>,
) -> Result<(Option<String>, Option<String>, Option<String>)> {
    match kind {
        EvolutionKind::Refine => {
            if sources.len() != 1 {
                bail!("Refine requires exactly one source skill");
            }
            let destination = destination.unwrap_or_else(|| sources[0].clone());
            if destination != sources[0] {
                bail!("Refine destination must equal its source");
            }
            let content = validate_candidate_content(&destination, content.as_deref())?;
            let fingerprint = fingerprint_raw_skill(&content);
            Ok((Some(destination), Some(content), Some(fingerprint)))
        }
        EvolutionKind::Merge => {
            if sources.len() != 2 {
                bail!("Merge requires exactly two source skills");
            }
            let destination = validate_name(destination.as_deref().unwrap_or_default())?;
            let content = validate_candidate_content(&destination, content.as_deref())?;
            let fingerprint = fingerprint_raw_skill(&content);
            if !sources.contains(&destination) && canonical_skill_path(&destination)?.exists() {
                bail!("Merge destination conflicts with an existing canonical skill");
            }
            Ok((Some(destination), Some(content), Some(fingerprint)))
        }
        EvolutionKind::Retire => {
            if sources.len() != 1 || destination.is_some() || content.is_some() {
                bail!("Retire requires one source and no destination or proposed content");
            }
            Ok((None, None, None))
        }
    }
}

fn validate_candidate_content(name: &str, raw: Option<&str>) -> Result<String> {
    let raw = raw.context("proposed_content is required")?;
    if raw.trim().is_empty() || raw.len() as u64 > MAX_SKILL_BYTES {
        bail!("proposed_content must be non-empty and bounded");
    }
    let stage_root = evolution_root()?.join("validation");
    create_private_dir(&stage_root)?;
    let unique = format!(
        "{}-{}",
        name,
        hex_digest(format!("{}:{}", Utc::now(), raw).as_bytes())
    );
    let workspace = stage_root.join(unique);
    let skill_dir = workspace.join(".jcode").join("skills").join(name);
    create_private_dir(&skill_dir)?;
    fs::write(skill_dir.join("SKILL.md"), raw)?;
    let registry = SkillRegistry::load_for_working_dir(Some(&workspace))?;
    let loaded = registry
        .get(name)
        .context("Proposed SKILL.md failed real registry parsing")?;
    if loaded.path != skill_dir.join("SKILL.md") {
        bail!("Proposed skill did not resolve from the staged SKILL.md");
    }
    let _ = fs::remove_dir_all(&workspace);
    Ok(raw.to_string())
}

async fn apply_transaction(
    registry: &Arc<RwLock<SkillRegistry>>,
    proposal: &EvolutionProposal,
) -> Result<()> {
    let tx_path = transaction_path()?;
    if tx_path.exists() {
        bail!("An incomplete skill evolution transaction requires recovery");
    }
    let stage_path = if let (Some(name), Some(content)) = (
        proposal.destination_name.as_ref(),
        proposal.proposed_content.as_ref(),
    ) {
        let path = evolution_root()?
            .join("staging")
            .join(&proposal.proposal_id)
            .join(name)
            .join("SKILL.md");
        create_private_dir(path.parent().unwrap())?;
        fs::write(&path, content)?;
        Some(path)
    } else {
        None
    };
    let mut tx = EvolutionTransaction {
        schema_version: SCHEMA_VERSION,
        proposal_id: proposal.proposal_id.clone(),
        phase: TransactionPhase::Staged,
        source_names: proposal.source_names.clone(),
        destination_name: proposal.destination_name.clone(),
        destination_fingerprint: proposal.proposed_fingerprint.clone(),
        stage_path,
        archives: BTreeMap::new(),
    };
    persist_transaction(&tx)?;
    let forward = async {
        for source in &proposal.source_names {
            let source_dir = canonical_skill_path(source)?
                .parent()
                .unwrap()
                .to_path_buf();
            let archive = archive_dir()?.join(&proposal.proposal_id).join(source);
            create_private_dir(archive.parent().unwrap())?;
            tx.archives.insert(source.clone(), archive.clone());
            persist_transaction(&tx)?;
            if source_dir.exists() {
                fs::rename(&source_dir, &archive)?;
            } else if !archive.exists() {
                bail!("Source skill '{source}' disappeared during mutation");
            }
        }
        tx.phase = TransactionPhase::SourcesArchived;
        persist_transaction(&tx)?;
        if let (Some(destination), Some(stage)) = (&proposal.destination_name, &tx.stage_path) {
            let destination_dir = canonical_skill_path(destination)?
                .parent()
                .unwrap()
                .to_path_buf();
            if destination_dir.exists() {
                bail!("Destination skill conflicts during installation");
            }
            create_private_dir(destination_dir.parent().unwrap())?;
            tx.phase = TransactionPhase::DestinationInstalling;
            persist_transaction(&tx)?;
            fs::rename(stage.parent().unwrap(), &destination_dir)?;
        }
        tx.phase = TransactionPhase::DestinationInstalled;
        persist_transaction(&tx)?;
        let fresh = SkillRegistry::load_global()?;
        verify_registry(&fresh, proposal)?;
        tx.phase = TransactionPhase::RegistryVerified;
        persist_transaction(&tx)?;
        *registry.write().await = fresh;
        tx.phase = TransactionPhase::Finalized;
        persist_transaction(&tx)?;
        fs::remove_file(&tx_path)?;
        Ok::<_, anyhow::Error>(())
    }
    .await;
    if let Err(error) = forward {
        if let Err(rollback) = rollback_transaction(registry, &tx).await {
            bail!(
                "Skill evolution failed: {error}; rollback also failed: {rollback}; recovery record: {}",
                tx_path.display()
            );
        }
        return Err(error);
    }
    Ok(())
}

async fn recover_incomplete(registry: &Arc<RwLock<SkillRegistry>>) -> Result<()> {
    let path = transaction_path()?;
    if !path.exists() {
        return Ok(());
    }
    let tx: EvolutionTransaction = read_json(&path)?;
    if tx.phase == TransactionPhase::Finalized {
        let fresh = SkillRegistry::load_global()?;
        *registry.write().await = fresh;
        fs::remove_file(path)?;
        return Ok(());
    }
    rollback_transaction(registry, &tx).await
}

async fn rollback_transaction(
    registry: &Arc<RwLock<SkillRegistry>>,
    tx: &EvolutionTransaction,
) -> Result<()> {
    if matches!(
        tx.phase,
        TransactionPhase::DestinationInstalling
            | TransactionPhase::DestinationInstalled
            | TransactionPhase::RegistryVerified
            | TransactionPhase::Finalized
    ) && let Some(destination) = &tx.destination_name
    {
        let destination_dir = canonical_skill_path(destination)?
            .parent()
            .unwrap()
            .to_path_buf();
        if destination_dir.exists() {
            let expected = tx
                .destination_fingerprint
                .as_deref()
                .context("Recovery record omitted destination fingerprint")?;
            let actual = fingerprint_raw_skill(&read_bounded(
                &destination_dir.join("SKILL.md"),
                MAX_SKILL_BYTES,
            )?);
            if actual != expected {
                bail!("Recovery refused to remove an unexpected destination skill");
            }
            fs::remove_dir_all(&destination_dir)?;
        }
    }
    for (source, archive) in tx.archives.iter().rev() {
        let source_dir = canonical_skill_path(source)?
            .parent()
            .unwrap()
            .to_path_buf();
        if !source_dir.exists() && archive.exists() {
            create_private_dir(source_dir.parent().unwrap())?;
            fs::rename(archive, source_dir)?;
        }
    }
    let fresh = SkillRegistry::load_global()?;
    *registry.write().await = fresh;
    let path = transaction_path()?;
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

fn verify_registry(registry: &SkillRegistry, proposal: &EvolutionProposal) -> Result<()> {
    for source in &proposal.source_names {
        if proposal.destination_name.as_deref() != Some(source) && registry.get(source).is_some() {
            bail!("Fresh registry still contains archived source '{source}'");
        }
    }
    if let (Some(destination), Some(expected)) = (
        proposal.destination_name.as_ref(),
        proposal.proposed_fingerprint.as_ref(),
    ) {
        let skill = registry
            .get(destination)
            .context("Fresh registry omitted installed destination")?;
        let (_, actual) = verified_canonical_skill(destination, &skill.path)?;
        if &actual != expected {
            bail!("Installed destination fingerprint differs from proposal");
        }
    }
    Ok(())
}

fn load_ordinary_session(id: &str) -> Result<Session> {
    let session = Session::load(id).with_context(|| format!("Session '{id}' was not found"))?;
    if session.id != id {
        bail!("Persisted session ID does not match requested session");
    }
    if session.is_debug {
        bail!("Debug sessions are not eligible skill evolution evidence");
    }
    Ok(session)
}

fn verify_direct_tool_use<'a>(
    session: &'a Session,
    message_id: &str,
    tool_call_id: &str,
    action: &str,
    skill_name: Option<&str>,
) -> Result<&'a serde_json::Value> {
    let message = session
        .messages
        .iter()
        .find(|message| message.id == message_id)
        .context("Persisted assistant message was not found")?;
    if message.role != Role::Assistant
        || message.display_role.is_some()
        || message.timestamp.is_none()
    {
        bail!("Evidence must be an ordinary durable assistant message");
    }
    let input = message
        .content
        .iter()
        .find_map(|block| match block {
            ContentBlock::ToolUse {
                id, name, input, ..
            } if id == tool_call_id && name == "skill_manage" => Some(input),
            _ => None,
        })
        .context("Direct persisted skill_manage tool call was not found")?;
    if input
        .get("action")
        .and_then(|value| value.as_str())
        .unwrap_or("load")
        != action
    {
        bail!("Persisted skill_manage action does not match");
    }
    if let Some(expected) = skill_name {
        let actual = input
            .get("name")
            .or_else(|| input.get("skill"))
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .trim_start_matches('/');
        if actual != expected {
            bail!("Persisted skill name does not match");
        }
    }
    Ok(input)
}

fn message_index(session: &Session, id: &str) -> Result<usize> {
    session
        .messages
        .iter()
        .position(|message| message.id == id)
        .with_context(|| format!("Persisted message '{id}' was not found"))
}

fn digest_window(session: &Session, start: usize, end: usize) -> Result<Vec<MessageDigest>> {
    let messages = session
        .messages
        .get(start..=end)
        .context("Invalid evidence message window")?;
    if messages.len() > 128 {
        bail!("Evidence message window is too large");
    }
    messages
        .iter()
        .map(|message| {
            let bytes = serde_json::to_vec(message)?;
            Ok(MessageDigest {
                message_id: message.id.clone(),
                digest: hex_digest(&bytes),
            })
        })
        .collect()
}

fn normalize_raw(raw: &str) -> String {
    raw.replace("\r\n", "\n").replace('\r', "\n")
}

fn normalize_names(mut names: Vec<String>) -> Result<Vec<String>> {
    names = names
        .into_iter()
        .map(|name| validate_name(&name))
        .collect::<Result<_>>()?;
    names.sort();
    names.dedup();
    if names.is_empty() {
        bail!("At least one source skill is required");
    }
    Ok(names)
}

fn validate_name(value: &str) -> Result<String> {
    if value.is_empty()
        || value.len() > MAX_NAME_BYTES
        || value == "."
        || value == ".."
        || value.starts_with('-')
        || value.ends_with('-')
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        bail!("skill names must be bounded lowercase kebab-case");
    }
    Ok(value.to_string())
}

fn validate_component(field: &str, value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > MAX_ID_BYTES
        || value == "."
        || value == ".."
        || value.contains('/')
        || value.contains('\\')
        || value.contains('\0')
    {
        bail!("{field} must be a bounded safe path component");
    }
    Ok(())
}

fn validate_hash_id(field: &str, value: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        bail!("{field} must be 64 lowercase hexadecimal characters");
    }
    Ok(())
}

fn reject_symlink_components(path: &Path) -> Result<()> {
    let root = crate::storage::jcode_dir()?;
    let relative = path
        .strip_prefix(&root)
        .context("Canonical skill path must remain inside JCODE_HOME")?;
    let mut current = root;
    for component in relative.components() {
        current.push(component);
        if let Ok(metadata) = fs::symlink_metadata(&current)
            && metadata.file_type().is_symlink()
        {
            bail!("Canonical skill paths may not contain symlinks");
        }
    }
    Ok(())
}

fn evolution_root() -> Result<PathBuf> {
    Ok(crate::storage::jcode_dir()?.join("skill-evolution"))
}
fn usage_dir() -> Result<PathBuf> {
    Ok(evolution_root()?.join("usage"))
}
fn outcome_dir() -> Result<PathBuf> {
    Ok(evolution_root()?.join("outcomes"))
}
fn suggestion_dir() -> Result<PathBuf> {
    Ok(evolution_root()?.join("suggestions"))
}
fn proposal_dir() -> Result<PathBuf> {
    Ok(evolution_root()?.join("proposals"))
}
fn archive_dir() -> Result<PathBuf> {
    Ok(evolution_root()?.join("archive"))
}
fn transaction_path() -> Result<PathBuf> {
    Ok(evolution_root()?.join("transaction.json"))
}
fn inbox_state_path() -> Result<PathBuf> {
    Ok(evolution_root()?.join("inbox-state.json"))
}

fn suggestion_pattern(value: &EvolutionSuggestion) -> String {
    let source_state = content_id(&value.source_fingerprints).unwrap_or_default();
    format!(
        "{:?}:{}:{source_state}",
        value.kind,
        value.source_names.join(",")
    )
}

fn load_inbox_state() -> Result<InboxState> {
    let path = inbox_state_path()?;
    if path.exists() {
        read_json(&path)
    } else {
        Ok(InboxState::default())
    }
}

fn persist_inbox_state(state: &InboxState) -> Result<()> {
    let path = inbox_state_path()?;
    create_private_dir(path.parent().unwrap())?;
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, serde_json::to_vec_pretty(state)?)?;
    fs::rename(temporary, path)?;
    Ok(())
}

fn create_private_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn read_bounded(path: &Path, max: u64) -> Result<String> {
    let file = fs::File::open(path)?;
    if file.metadata()?.len() > max {
        bail!("File exceeds {max} bytes");
    }
    let mut bytes = Vec::new();
    file.take(max + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max {
        bail!("File exceeds {max} bytes");
    }
    Ok(String::from_utf8(bytes)?)
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    Ok(serde_json::from_str(&read_bounded(
        path,
        MAX_RECORD_BYTES,
    )?)?)
}

fn write_immutable<T: Serialize + for<'de> Deserialize<'de> + PartialEq>(
    dir: &Path,
    id: &str,
    value: &T,
) -> Result<()> {
    validate_hash_id("content address", id)?;
    create_private_dir(dir)?;
    let path = dir.join(format!("{id}.json"));
    let bytes = serde_json::to_vec_pretty(value)?;
    if bytes.len() as u64 > MAX_RECORD_BYTES {
        bail!("Evolution record is oversized");
    }
    match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(mut file) => {
            file.write_all(&bytes)?;
            file.sync_all()?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let existing: T = read_json(&path)?;
            if &existing != value {
                bail!(
                    "Conflicting immutable evolution artifact at {}",
                    path.display()
                );
            }
        }
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn load_content_addressed<T: for<'de> Deserialize<'de>>(dir: &Path, id: &str) -> Result<T> {
    validate_hash_id("content address", id)?;
    read_json(&dir.join(format!("{id}.json")))
}

fn existing_content_addressed<T: for<'de> Deserialize<'de>>(
    dir: &Path,
    id: &str,
) -> Result<Option<T>> {
    validate_hash_id("content address", id)?;
    let path = dir.join(format!("{id}.json"));
    if !path.exists() {
        return Ok(None);
    }
    read_json(&path).map(Some)
}
fn load_usage(id: &str) -> Result<UsageRecord> {
    load_content_addressed(&usage_dir()?, id)
}
fn load_outcome(id: &str) -> Result<OutcomeRecord> {
    load_content_addressed(&outcome_dir()?, id)
}

fn read_recent_records<T: for<'de> Deserialize<'de>>(dir: &Path, max: usize) -> Result<Vec<T>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut paths = fs::read_dir(dir)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .map(|path| {
            let modified = path
                .metadata()
                .and_then(|metadata| metadata.modified())
                .ok();
            (modified, path)
        })
        .collect::<Vec<_>>();
    paths.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    paths.truncate(max);
    Ok(paths
        .into_iter()
        .filter_map(|(_, path)| read_json(&path).ok())
        .collect())
}

fn persist_transaction(tx: &EvolutionTransaction) -> Result<()> {
    let path = transaction_path()?;
    create_private_dir(path.parent().unwrap())?;
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, serde_json::to_vec_pretty(tx)?)?;
    fs::rename(temporary, path)?;
    Ok(())
}

fn content_id<T: Serialize>(value: &T) -> Result<String> {
    Ok(hex_digest(&serde_json::to_vec(value)?))
}
fn hex_digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[derive(Serialize)]
struct UsageIdentity<'a> {
    schema_version: u32,
    session_id: &'a str,
    load_message_id: &'a str,
    load_tool_call_id: &'a str,
    skill_name: &'a str,
    canonical_path: &'a Path,
    skill_fingerprint: &'a str,
}
impl<'a> From<&'a UsageRecord> for UsageIdentity<'a> {
    fn from(v: &'a UsageRecord) -> Self {
        Self {
            schema_version: v.schema_version,
            session_id: &v.session_id,
            load_message_id: &v.load_message_id,
            load_tool_call_id: &v.load_tool_call_id,
            skill_name: &v.skill_name,
            canonical_path: &v.canonical_path,
            skill_fingerprint: &v.skill_fingerprint,
        }
    }
}
#[derive(Serialize)]
struct OutcomeIdentity<'a> {
    schema_version: u32,
    usage_id: &'a str,
    session_id: &'a str,
    outcome_message_id: &'a str,
    outcome_tool_call_id: &'a str,
    outcome: OutcomeClass,
    confidence_bits: u64,
    rationale: &'a str,
    related_skill: &'a Option<String>,
    evidence_window: &'a [MessageDigest],
}
impl<'a> From<&'a OutcomeRecord> for OutcomeIdentity<'a> {
    fn from(v: &'a OutcomeRecord) -> Self {
        Self {
            schema_version: v.schema_version,
            usage_id: &v.usage_id,
            session_id: &v.session_id,
            outcome_message_id: &v.outcome_message_id,
            outcome_tool_call_id: &v.outcome_tool_call_id,
            outcome: v.outcome,
            confidence_bits: v.confidence.to_bits(),
            rationale: &v.rationale,
            related_skill: &v.related_skill,
            evidence_window: &v.evidence_window,
        }
    }
}
#[derive(Serialize)]
struct SuggestionIdentity<'a> {
    schema_version: u32,
    kind: EvolutionKind,
    source_names: &'a [String],
    source_fingerprints: &'a BTreeMap<String, String>,
    outcome_ids: &'a [String],
    evidence_digest: &'a str,
    summary: &'a str,
}
impl<'a> From<&'a EvolutionSuggestion> for SuggestionIdentity<'a> {
    fn from(v: &'a EvolutionSuggestion) -> Self {
        Self {
            schema_version: v.schema_version,
            kind: v.kind,
            source_names: &v.source_names,
            source_fingerprints: &v.source_fingerprints,
            outcome_ids: &v.outcome_ids,
            evidence_digest: &v.evidence_digest,
            summary: &v.summary,
        }
    }
}
#[derive(Serialize)]
struct ProposalIdentity<'a> {
    schema_version: u32,
    suggestion_id: &'a str,
    kind: EvolutionKind,
    source_names: &'a [String],
    source_fingerprints: &'a BTreeMap<String, String>,
    destination_name: &'a Option<String>,
    proposed_content: &'a Option<String>,
    proposed_fingerprint: &'a Option<String>,
    outcome_ids: &'a [String],
}
impl<'a> From<&'a EvolutionProposal> for ProposalIdentity<'a> {
    fn from(v: &'a EvolutionProposal) -> Self {
        Self {
            schema_version: v.schema_version,
            suggestion_id: &v.suggestion_id,
            kind: v.kind,
            source_names: &v.source_names,
            source_fingerprints: &v.source_fingerprints,
            destination_name: &v.destination_name,
            proposed_content: &v.proposed_content,
            proposed_fingerprint: &v.proposed_fingerprint,
            outcome_ids: &v.outcome_ids,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{ContentBlock, Role};
    use serde_json::json;

    struct TestHome {
        previous: Option<std::ffi::OsString>,
        dir: tempfile::TempDir,
    }

    impl TestHome {
        fn new() -> Self {
            let dir = tempfile::tempdir().expect("temp home");
            let previous = std::env::var_os("JCODE_HOME");
            unsafe { std::env::set_var("JCODE_HOME", dir.path()) };
            Self { previous, dir }
        }

        fn write_skill(&self, name: &str, description: &str) -> PathBuf {
            let path = self.dir.path().join("skills").join(name).join("SKILL.md");
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(
                &path,
                format!(
                    "---\nname: {name}\ndescription: {description}\n---\n\n# {name}\n\nFollow the verified workflow.\n"
                ),
            )
            .unwrap();
            path
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

    fn tool_use(id: &str, input: serde_json::Value) -> ContentBlock {
        ContentBlock::ToolUse {
            id: id.to_string(),
            name: "skill_manage".to_string(),
            input,
            thought_signature: None,
        }
    }

    #[test]
    fn raw_fingerprint_includes_frontmatter_and_normalizes_only_line_endings() {
        let unix = "---\nname: demo\ndescription: one\n---\nbody\n";
        let windows = unix.replace('\n', "\r\n");
        assert_eq!(fingerprint_raw_skill(unix), fingerprint_raw_skill(&windows));
        assert_ne!(
            fingerprint_raw_skill(unix),
            fingerprint_raw_skill(&unix.replace("one", "two"))
        );
        assert_ne!(
            fingerprint_raw_skill(unix),
            fingerprint_raw_skill("\n---\nname: demo\ndescription: one\n---\nbody\n")
        );
    }

    #[test]
    fn outcome_and_kind_values_are_closed() {
        assert_eq!(
            OutcomeClass::parse("corrected").unwrap(),
            OutcomeClass::Corrected
        );
        assert!(OutcomeClass::parse("good").is_err());
        assert_eq!(
            EvolutionKind::parse("retire").unwrap(),
            EvolutionKind::Retire
        );
        assert!(EvolutionKind::parse("delete").is_err());
    }

    #[test]
    fn identifiers_and_names_are_path_safe() {
        assert!(validate_name("good-skill2").is_ok());
        assert!(validate_name("../bad").is_err());
        assert!(validate_component("id", "call-1").is_ok());
        assert!(validate_component("id", "a/b").is_err());
    }

    #[test]
    fn durable_usage_and_outcome_are_idempotent_argument_bound_and_tamper_evident() {
        let _env_lock = crate::storage::lock_test_env();
        let home = TestHome::new();
        let skill_path = home.write_skill("demo", "First description");
        let mut session = Session::create_with_id("evolution-evidence".to_string(), None, None);
        session.add_message(
            Role::Assistant,
            vec![tool_use(
                "load-call",
                json!({"action": "load", "name": "demo"}),
            )],
        );
        let load_message_id = session.messages.last().unwrap().id.clone();
        session.save().unwrap();

        let usage = record_usage(
            &session.id,
            &load_message_id,
            "load-call",
            "demo",
            &skill_path,
        )
        .unwrap();
        let repeated_usage = record_usage(
            &session.id,
            &load_message_id,
            "load-call",
            "demo",
            &skill_path,
        )
        .unwrap();
        assert_eq!(usage.usage_id, repeated_usage.usage_id);
        assert_eq!(usage.created_at, repeated_usage.created_at);

        session.add_message(
            Role::User,
            vec![ContentBlock::Text {
                text: "Apply the loaded workflow.".to_string(),
                cache_control: None,
            }],
        );
        session.add_message(
            Role::Assistant,
            vec![tool_use(
                "outcome-call",
                json!({
                    "action": "record_skill_outcome",
                    "usage_id": usage.usage_id,
                    "outcome": "corrected",
                    "confidence": 0.9,
                    "rationale": "The workflow needed one correction."
                }),
            )],
        );
        let outcome_message_id = session.messages.last().unwrap().id.clone();
        session.save().unwrap();

        let outcome = record_outcome(
            &session.id,
            &outcome_message_id,
            "outcome-call",
            &usage.usage_id,
            OutcomeClass::Corrected,
            0.9,
            "The workflow needed one correction.",
            None,
        )
        .unwrap();
        let repeated_outcome = record_outcome(
            &session.id,
            &outcome_message_id,
            "outcome-call",
            &usage.usage_id,
            OutcomeClass::Corrected,
            0.9,
            "The workflow needed one correction.",
            None,
        )
        .unwrap();
        assert_eq!(outcome.outcome_id, repeated_outcome.outcome_id);
        assert_eq!(outcome.created_at, repeated_outcome.created_at);
        assert!(
            record_outcome(
                &session.id,
                &outcome_message_id,
                "outcome-call",
                &usage.usage_id,
                OutcomeClass::Corrected,
                0.9,
                "Different from the persisted rationale.",
                None,
            )
            .unwrap_err()
            .to_string()
            .contains("arguments do not match")
        );

        fs::write(
            &skill_path,
            "---\nname: demo\ndescription: Changed frontmatter\n---\n\n# demo\n\nFollow the verified workflow.\n",
        )
        .unwrap();
        assert!(
            revalidate_outcome(&outcome)
                .unwrap_err()
                .to_string()
                .contains("fingerprint changed")
        );
    }

    #[test]
    fn discovery_thresholds_use_distinct_sessions_and_positive_evidence_blocks_retirement() {
        fn outcome(id: usize, session: &str, class: OutcomeClass) -> OutcomeRecord {
            OutcomeRecord {
                schema_version: SCHEMA_VERSION,
                outcome_id: format!("{id:064x}"),
                usage_id: format!("{:064x}", id + 100),
                session_id: session.to_string(),
                outcome_message_id: format!("message-{id}"),
                outcome_tool_call_id: format!("call-{id}"),
                outcome: class,
                confidence: 0.9,
                rationale: "verified".to_string(),
                related_skill: None,
                evidence_window: Vec::new(),
                created_at: Utc::now(),
            }
        }

        let repeated_session = (0..3)
            .map(|id| outcome(id, "same-session", OutcomeClass::Corrected))
            .collect::<Vec<_>>();
        let mut grouped = BTreeMap::new();
        grouped.insert("demo".to_string(), repeated_session.iter().collect());
        assert!(candidates_from_grouped(grouped).is_empty());

        let corrections = (0..3)
            .map(|id| outcome(id, &format!("session-{id}"), OutcomeClass::Corrected))
            .collect::<Vec<_>>();
        let mut grouped = BTreeMap::new();
        grouped.insert("demo".to_string(), corrections.iter().collect());
        assert_eq!(
            candidates_from_grouped(grouped)[0].kind,
            EvolutionKind::Refine
        );

        let mut retirement = (0..5)
            .map(|id| outcome(id, &format!("negative-{id}"), OutcomeClass::Unused))
            .collect::<Vec<_>>();
        retirement.push(outcome(10, "helped", OutcomeClass::Helped));
        let mut grouped = BTreeMap::new();
        grouped.insert("demo".to_string(), retirement.iter().collect());
        assert!(
            candidates_from_grouped(grouped)
                .iter()
                .all(|candidate| candidate.kind != EvolutionKind::Retire)
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn recovery_preserves_sources_before_install_and_keeps_finalized_destinations() {
        let _env_lock = crate::storage::lock_test_env();
        let home = TestHome::new();
        let skill_path = home.write_skill("demo", "Stable source");
        let registry = Arc::new(RwLock::new(SkillRegistry::load_global().unwrap()));

        let staged = EvolutionTransaction {
            schema_version: SCHEMA_VERSION,
            proposal_id: "1".repeat(64),
            phase: TransactionPhase::Staged,
            source_names: vec!["demo".to_string()],
            destination_name: Some("demo".to_string()),
            destination_fingerprint: Some(fingerprint_raw_skill(
                &fs::read_to_string(&skill_path).unwrap(),
            )),
            stage_path: None,
            archives: BTreeMap::new(),
        };
        persist_transaction(&staged).unwrap();
        recover_incomplete(&registry).await.unwrap();
        assert!(
            skill_path.exists(),
            "staged recovery must not delete the source"
        );

        let finalized = EvolutionTransaction {
            phase: TransactionPhase::Finalized,
            ..staged
        };
        persist_transaction(&finalized).unwrap();
        recover_incomplete(&registry).await.unwrap();
        assert!(
            skill_path.exists(),
            "finalized recovery must keep the installed destination"
        );
        assert!(!transaction_path().unwrap().exists());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn recovery_rolls_back_a_destination_renamed_before_phase_commit() {
        let _env_lock = crate::storage::lock_test_env();
        let home = TestHome::new();
        let skill_path = home.write_skill("demo", "Original source");
        let original = fs::read_to_string(&skill_path).unwrap();
        let proposal_id = "2".repeat(64);
        let archive = archive_dir().unwrap().join(&proposal_id).join("demo");
        create_private_dir(archive.parent().unwrap()).unwrap();
        fs::rename(skill_path.parent().unwrap(), &archive).unwrap();
        let replacement = "---\nname: demo\ndescription: Interrupted replacement\n---\n\n# demo\n";
        create_private_dir(skill_path.parent().unwrap()).unwrap();
        fs::write(&skill_path, replacement).unwrap();
        let tx = EvolutionTransaction {
            schema_version: SCHEMA_VERSION,
            proposal_id,
            phase: TransactionPhase::DestinationInstalling,
            source_names: vec!["demo".to_string()],
            destination_name: Some("demo".to_string()),
            destination_fingerprint: Some(fingerprint_raw_skill(replacement)),
            stage_path: None,
            archives: BTreeMap::from([("demo".to_string(), archive)]),
        };
        persist_transaction(&tx).unwrap();
        let registry = Arc::new(RwLock::new(SkillRegistry::load_global().unwrap()));

        recover_incomplete(&registry).await.unwrap();
        assert_eq!(fs::read_to_string(&skill_path).unwrap(), original);
        assert!(!transaction_path().unwrap().exists());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn verified_refine_pipeline_requires_approval_and_swaps_the_live_registry() {
        let _env_lock = crate::storage::lock_test_env();
        let home = TestHome::new();
        let skill_path = home.write_skill("demo", "Original description");

        for index in 0..3 {
            let mut session =
                Session::create_with_id(format!("refine-evidence-{index}"), None, None);
            let load_call = format!("load-{index}");
            session.add_message(
                Role::Assistant,
                vec![tool_use(
                    &load_call,
                    json!({"action": "load", "name": "demo"}),
                )],
            );
            let load_message = session.messages.last().unwrap().id.clone();
            session.save().unwrap();
            let usage =
                record_usage(&session.id, &load_message, &load_call, "demo", &skill_path).unwrap();

            let outcome_call = format!("outcome-{index}");
            let rationale = format!("Correction observed in independent session {index}.");
            session.add_message(
                Role::User,
                vec![ContentBlock::Text {
                    text: "Finish the workflow and report its outcome.".to_string(),
                    cache_control: None,
                }],
            );
            session.add_message(
                Role::Assistant,
                vec![tool_use(
                    &outcome_call,
                    json!({
                        "action": "record_skill_outcome",
                        "usage_id": usage.usage_id,
                        "outcome": "corrected",
                        "confidence": 0.95,
                        "rationale": rationale
                    }),
                )],
            );
            let outcome_message = session.messages.last().unwrap().id.clone();
            session.save().unwrap();
            record_outcome(
                &session.id,
                &outcome_message,
                &outcome_call,
                &usage.usage_id,
                OutcomeClass::Corrected,
                0.95,
                &format!("Correction observed in independent session {index}."),
                None,
            )
            .unwrap();
        }

        let suggestion = discover_verified()
            .unwrap()
            .into_iter()
            .find(|suggestion| suggestion.kind == EvolutionKind::Refine)
            .expect("three verified corrections should suggest refinement");
        let replacement = "---\nname: demo\ndescription: Refined description\n---\n\n# demo\n\nFollow the corrected verified workflow.\n";
        let proposal = propose(
            &suggestion.suggestion_id,
            EvolutionKind::Refine,
            vec!["demo".to_string()],
            Some("demo".to_string()),
            Some(replacement.to_string()),
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(&skill_path)
                .unwrap()
                .contains("Original"),
            true
        );

        std::thread::sleep(std::time::Duration::from_millis(2));
        let mut approval = Session::create_with_id("refine-approval".to_string(), None, None);
        approval.add_message(
            Role::User,
            vec![ContentBlock::Text {
                text: format!(
                    "I approve skill evolution proposal {}.",
                    proposal.proposal_id
                ),
                cache_control: None,
            }],
        );
        let approval_message = approval.messages.last().unwrap().id.clone();
        approval.save().unwrap();
        let reference = EvidenceReference {
            session_id: approval.id,
            message_id: approval_message,
        };
        let registry = Arc::new(RwLock::new(SkillRegistry::load_global().unwrap()));
        assert!(
            approve(&registry, &proposal.proposal_id, false, &reference)
                .await
                .unwrap_err()
                .to_string()
                .contains("confirmed=true")
        );

        approve(&registry, &proposal.proposal_id, true, &reference)
            .await
            .unwrap();
        assert_eq!(fs::read_to_string(&skill_path).unwrap(), replacement);
        let loaded = registry.read().await.get("demo").cloned().unwrap();
        assert_eq!(loaded.path, skill_path);
        assert!(loaded.description.contains("Refined"));
        assert!(latest_pending().unwrap().is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn merge_and_retire_transactions_update_files_and_the_injected_registry() {
        let _env_lock = crate::storage::lock_test_env();
        let home = TestHome::new();
        let alpha = home.write_skill("alpha", "Alpha workflow");
        let beta = home.write_skill("beta", "Beta workflow");
        let merged_path = canonical_skill_path("merged").unwrap();
        let merged_content = "---\nname: merged\ndescription: Unified workflow\n---\n\n# merged\n";
        let registry = Arc::new(RwLock::new(SkillRegistry::load_global().unwrap()));
        let merge = EvolutionProposal {
            schema_version: SCHEMA_VERSION,
            proposal_id: "3".repeat(64),
            suggestion_id: "4".repeat(64),
            kind: EvolutionKind::Merge,
            source_names: vec!["alpha".to_string(), "beta".to_string()],
            source_fingerprints: BTreeMap::from([
                (
                    "alpha".to_string(),
                    fingerprint_raw_skill(&fs::read_to_string(&alpha).unwrap()),
                ),
                (
                    "beta".to_string(),
                    fingerprint_raw_skill(&fs::read_to_string(&beta).unwrap()),
                ),
            ]),
            destination_name: Some("merged".to_string()),
            proposed_content: Some(merged_content.to_string()),
            proposed_fingerprint: Some(fingerprint_raw_skill(merged_content)),
            outcome_ids: Vec::new(),
            created_at: Utc::now(),
        };
        apply_transaction(&registry, &merge).await.unwrap();
        assert!(!alpha.exists());
        assert!(!beta.exists());
        assert_eq!(fs::read_to_string(&merged_path).unwrap(), merged_content);
        assert!(registry.read().await.get("merged").is_some());

        let retired = home.write_skill("retired", "Obsolete workflow");
        *registry.write().await = SkillRegistry::load_global().unwrap();
        let retire = EvolutionProposal {
            schema_version: SCHEMA_VERSION,
            proposal_id: "5".repeat(64),
            suggestion_id: "6".repeat(64),
            kind: EvolutionKind::Retire,
            source_names: vec!["retired".to_string()],
            source_fingerprints: BTreeMap::from([(
                "retired".to_string(),
                fingerprint_raw_skill(&fs::read_to_string(&retired).unwrap()),
            )]),
            destination_name: None,
            proposed_content: None,
            proposed_fingerprint: None,
            outcome_ids: Vec::new(),
            created_at: Utc::now(),
        };
        apply_transaction(&registry, &retire).await.unwrap();
        assert!(!retired.exists());
        assert!(registry.read().await.get("retired").is_none());
        assert!(registry.read().await.get("merged").is_some());
    }
}
