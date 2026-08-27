use super::crystallization::{EvidenceReference, verify_evidence};
use crate::message::Role;
use crate::session::{
    Session, durable_conversation_evidence_text, session_journal_path_from_snapshot,
};
use crate::tool::ToolOutput;
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

const SCHEMA_VERSION: u32 = 1;
const DEFAULT_SESSION_LIMIT: usize = 100;
const MAX_EVIDENCE: usize = 12;
const MIN_DISTINCT_SESSIONS: usize = 3;
const MIN_TEXT_CHARS: usize = 20;
const MAX_TEXT_CHARS: usize = 2_000;
const MAX_STATE_BYTES: u64 = 256 * 1024;
const MAX_STATE_IDS: usize = 10_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Suggestion {
    pub schema_version: u32,
    pub suggestion_id: String,
    pub pattern_id: String,
    pub workflow_text: String,
    pub evidence: Vec<EvidenceReference>,
    pub evidence_digests: Vec<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct DiscoveryState {
    schema_version: u32,
    dismissed_suggestion_ids: HashSet<String>,
    suppressed_pattern_ids: HashSet<String>,
}

#[derive(Debug)]
struct Occurrence {
    reference: EvidenceReference,
    text: String,
    timestamp: Option<DateTime<Utc>>,
}

pub fn discover() -> Result<Option<Suggestion>> {
    let state = load_state()?;
    let sessions_dir = crate::storage::jcode_dir()?.join("sessions");
    let mut files = recent_session_files(&sessions_dir, DEFAULT_SESSION_LIMIT)?;
    let mut groups: HashMap<String, Vec<Occurrence>> = HashMap::new();

    for path in files.drain(..) {
        let Ok(session) = Session::load_from_path(&path) else {
            continue;
        };
        let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
            continue;
        };
        if session.id != stem {
            continue;
        }
        let mut seen_patterns = HashSet::new();
        for message in &session.messages {
            if message.role != Role::User {
                continue;
            }
            let Some(text) = durable_conversation_evidence_text(message) else {
                continue;
            };
            let normalized = normalize_workflow(&text);
            if !eligible_workflow(&normalized) || !seen_patterns.insert(normalized.clone()) {
                continue;
            }
            groups.entry(normalized).or_default().push(Occurrence {
                reference: EvidenceReference {
                    session_id: session.id.clone(),
                    message_id: message.id.clone(),
                },
                text,
                timestamp: message.timestamp,
            });
        }
    }

    let mut candidates = groups
        .into_iter()
        .filter_map(|(normalized, mut occurrences)| {
            if occurrences.len() < MIN_DISTINCT_SESSIONS {
                return None;
            }
            let distinct_session_count = occurrences.len();
            occurrences.sort_by(|a, b| {
                b.timestamp
                    .cmp(&a.timestamp)
                    .then_with(|| a.reference.session_id.cmp(&b.reference.session_id))
                    .then_with(|| a.reference.message_id.cmp(&b.reference.message_id))
            });
            occurrences.truncate(MAX_EVIDENCE);
            let pattern_id = digest(normalized.as_bytes());
            if state.suppressed_pattern_ids.contains(&pattern_id) {
                return None;
            }
            let evidence = occurrences
                .iter()
                .map(|item| item.reference.clone())
                .collect::<Vec<_>>();
            let suggestion_id = suggestion_id(&pattern_id, &evidence);
            if state.dismissed_suggestion_ids.contains(&suggestion_id) {
                return None;
            }
            let newest = occurrences.iter().filter_map(|item| item.timestamp).max();
            Some((
                distinct_session_count,
                newest,
                pattern_id,
                suggestion_id,
                occurrences[0].text.clone(),
                evidence,
            ))
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| b.1.cmp(&a.1))
            .then_with(|| a.2.cmp(&b.2))
    });

    let Some((_, _, pattern_id, suggestion_id, workflow_text, evidence)) =
        candidates.into_iter().next()
    else {
        return Ok(None);
    };
    let verified = verify_evidence(&evidence)?;
    let suggestion = Suggestion {
        schema_version: SCHEMA_VERSION,
        suggestion_id,
        pattern_id,
        workflow_text,
        evidence,
        evidence_digests: verified
            .into_iter()
            .map(|item| item.message_digest)
            .collect(),
        created_at: Utc::now(),
    };
    Ok(Some(persist_suggestion(&suggestion)?))
}

pub fn review(suggestion_id: &str) -> Result<Suggestion> {
    let suggestion = load_suggestion(suggestion_id)?;
    let evidence = verify_evidence(&suggestion.evidence)?;
    let current_digests = evidence
        .into_iter()
        .map(|item| item.message_digest)
        .collect::<Vec<_>>();
    if current_digests != suggestion.evidence_digests {
        bail!("Discovery evidence changed after suggestion creation");
    }
    Ok(suggestion)
}

fn load_suggestion(suggestion_id: &str) -> Result<Suggestion> {
    validate_digest("suggestion_id", suggestion_id)?;
    let path = suggestions_dir()?.join(format!("{suggestion_id}.json"));
    let bytes = read_bounded(&path)?;
    let suggestion: Suggestion = serde_json::from_slice(&bytes)
        .with_context(|| format!("Invalid discovery suggestion {}", path.display()))?;
    validate_suggestion(&suggestion, suggestion_id)?;
    Ok(suggestion)
}

pub fn dismiss(suggestion_id: &str) -> Result<Suggestion> {
    let suggestion = load_suggestion(suggestion_id)?;
    let mut state = load_state()?;
    state
        .dismissed_suggestion_ids
        .insert(suggestion.suggestion_id.clone());
    save_state(&state)?;
    Ok(suggestion)
}

pub fn suppress(suggestion_id: &str) -> Result<Suggestion> {
    let suggestion = load_suggestion(suggestion_id)?;
    let mut state = load_state()?;
    state
        .suppressed_pattern_ids
        .insert(suggestion.pattern_id.clone());
    save_state(&state)?;
    Ok(suggestion)
}

pub fn no_suggestion_output() -> ToolOutput {
    ToolOutput::new(
        "No high-confidence repeated workflow was found in the 100 most recent persisted sessions.",
    )
    .with_title("Skill discovery: No suggestion")
    .with_metadata(json!({
        "schema_version": 1,
        "action": "discover_crystallization",
        "status": "no_suggestion"
    }))
}

pub fn suggestion_output(suggestion: &Suggestion, status: &str) -> ToolOutput {
    let review = json!({
        "action": "review_crystallization",
        "suggestion_id": suggestion.suggestion_id
    });
    let dismiss = json!({
        "action": "dismiss_crystallization",
        "suggestion_id": suggestion.suggestion_id
    });
    let suppress = json!({
        "action": "suppress_crystallization",
        "suggestion_id": suggestion.suggestion_id
    });
    let crystallize = json!({
        "action": "crystallize",
        "name": "<draft a concise skill name>",
        "description": "<draft a precise description>",
        "content": "<draft the exact SKILL.md body from the reviewed evidence>",
        "evidence": suggestion.evidence
    });
    let evidence = suggestion
        .evidence
        .iter()
        .map(|item| format!("- session={} message={}", item.session_id, item.message_id))
        .collect::<Vec<_>>()
        .join("\n");
    let proposal_section = if status == "reviewed" {
        format!(
            "\n\nDraft a focused skill and use the existing proposal call:\n```json\n{}\n```",
            serde_json::to_string_pretty(&crystallize).expect("JSON serialization cannot fail")
        )
    } else {
        String::new()
    };
    ToolOutput::new(format!(
        "Repeated workflow found in {} distinct sessions:\n\n> {}\n\n## Evidence\n{}\n\n**Review**\n```json\n{}\n```\n\n**Dismiss**\n```json\n{}\n```\n\n**Never suggest this**\n```json\n{}\n```{}",
        suggestion.evidence.len(),
        suggestion.workflow_text,
        evidence,
        serde_json::to_string_pretty(&review).expect("JSON serialization cannot fail"),
        serde_json::to_string_pretty(&dismiss).expect("JSON serialization cannot fail"),
        serde_json::to_string_pretty(&suppress).expect("JSON serialization cannot fail"),
        proposal_section
    ))
    .with_title("Skill discovery: Repeated workflow")
    .with_metadata(json!({
        "kind": "skill_crystallization_discovery",
        "schema_version": 1,
        "status": status,
        "suggestion_id": suggestion.suggestion_id,
        "pattern_id": suggestion.pattern_id,
        "workflow_text": suggestion.workflow_text,
        "evidence": suggestion.evidence,
        "actions": {
            "review": review,
            "dismiss": dismiss,
            "never_suggest": suppress,
            "propose": (status == "reviewed").then_some(crystallize)
        }
    }))
}

pub fn state_output(suggestion: &Suggestion, status: &str) -> ToolOutput {
    let message = if status == "dismissed" {
        "This evidence snapshot was dismissed. A later snapshot with new examples may be suggested."
    } else {
        "This workflow pattern will not be suggested again."
    };
    ToolOutput::new(message)
        .with_title(if status == "dismissed" {
            "Skill discovery: Dismissed"
        } else {
            "Skill discovery: Suppressed"
        })
        .with_metadata(json!({
            "kind": "skill_crystallization_discovery",
            "schema_version": 1,
            "status": status,
            "suggestion_id": suggestion.suggestion_id,
            "pattern_id": suggestion.pattern_id
        }))
}

fn recent_session_files(dir: &Path, limit: usize) -> Result<Vec<PathBuf>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut files = fs::read_dir(dir)?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .filter_map(|path| {
            let snapshot_modified = path.metadata().ok()?.modified().ok()?;
            let journal_modified = session_journal_path_from_snapshot(&path)
                .metadata()
                .and_then(|metadata| metadata.modified())
                .unwrap_or(snapshot_modified);
            Some((snapshot_modified.max(journal_modified), path))
        })
        .collect::<Vec<_>>();
    files.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    files.truncate(limit);
    Ok(files.into_iter().map(|(_, path)| path).collect())
}

fn normalize_workflow(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn eligible_workflow(normalized: &str) -> bool {
    let chars = normalized.chars().count();
    (MIN_TEXT_CHARS..=MAX_TEXT_CHARS).contains(&chars)
        && !normalized.starts_with("i approve skill crystallization proposal ")
        && !normalized.starts_with("[auto]")
        && !normalized.starts_with("**background task**")
        && !normalized.starts_with("🐝 **swarm await finished**")
        && !normalized.starts_with("⚠ file activity:")
        && !(normalized.starts_with("you have ") && normalized.contains(" incomplete todos"))
}

fn suggestion_id(pattern_id: &str, evidence: &[EvidenceReference]) -> String {
    let mut canonical = evidence
        .iter()
        .map(|item| format!("{}:{}", item.session_id, item.message_id))
        .collect::<Vec<_>>();
    canonical.sort();
    digest(format!("{pattern_id}\n{}", canonical.join("\n")).as_bytes())
}

fn validate_suggestion(suggestion: &Suggestion, expected_id: &str) -> Result<()> {
    if suggestion.schema_version != SCHEMA_VERSION {
        bail!("Unsupported discovery suggestion schema");
    }
    validate_digest("suggestion_id", &suggestion.suggestion_id)?;
    validate_digest("pattern_id", &suggestion.pattern_id)?;
    if suggestion.suggestion_id != expected_id {
        bail!("Discovery suggestion ID does not match its path");
    }
    let normalized = normalize_workflow(&suggestion.workflow_text);
    if !eligible_workflow(&normalized) || digest(normalized.as_bytes()) != suggestion.pattern_id {
        bail!("Discovery suggestion workflow fingerprint is invalid");
    }
    if suggestion.evidence.len() < MIN_DISTINCT_SESSIONS
        || suggestion.evidence.len() > MAX_EVIDENCE
        || suggestion.evidence_digests.len() != suggestion.evidence.len()
        || suggestion
            .evidence_digests
            .iter()
            .any(|digest| validate_digest("evidence digest", digest).is_err())
        || suggestion_id(&suggestion.pattern_id, &suggestion.evidence) != suggestion.suggestion_id
    {
        bail!("Discovery suggestion evidence is invalid");
    }
    Ok(())
}

fn validate_digest(label: &str, value: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("{label} must be exactly 64 lowercase hexadecimal characters");
    }
    Ok(())
}

fn persist_suggestion(suggestion: &Suggestion) -> Result<Suggestion> {
    let dir = suggestions_dir()?;
    create_private_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", suggestion.suggestion_id));
    let bytes = serde_json::to_vec_pretty(suggestion)?;
    match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(mut file) => {
            file.write_all(&bytes)?;
            file.sync_all()?;
            Ok(suggestion.clone())
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let existing: Suggestion = serde_json::from_slice(&read_bounded(&path)?)?;
            validate_suggestion(&existing, &suggestion.suggestion_id)?;
            if existing.pattern_id != suggestion.pattern_id
                || existing.workflow_text != suggestion.workflow_text
                || existing.evidence != suggestion.evidence
                || existing.evidence_digests != suggestion.evidence_digests
            {
                bail!("Conflicting discovery suggestion already exists");
            }
            Ok(existing)
        }
        Err(error) => Err(error.into()),
    }
}

fn load_state() -> Result<DiscoveryState> {
    let path = state_path()?;
    if !path.exists() {
        return Ok(DiscoveryState {
            schema_version: SCHEMA_VERSION,
            ..DiscoveryState::default()
        });
    }
    let bytes = read_bounded(&path)?;
    let state: DiscoveryState = serde_json::from_slice(&bytes)
        .with_context(|| format!("Invalid discovery state {}", path.display()))?;
    if state.schema_version != SCHEMA_VERSION {
        bail!("Unsupported discovery state schema");
    }
    Ok(state)
}

fn save_state(state: &DiscoveryState) -> Result<()> {
    if state.dismissed_suggestion_ids.len() > MAX_STATE_IDS
        || state.suppressed_pattern_ids.len() > MAX_STATE_IDS
    {
        bail!("Discovery state exceeds its bounded identifier limit");
    }
    let dir = discovery_dir()?;
    create_private_dir_all(&dir)?;
    let path = state_path()?;
    let temporary = dir.join(format!(".state-{}.tmp", std::process::id()));
    let bytes = serde_json::to_vec_pretty(state)?;
    if bytes.len() as u64 > MAX_STATE_BYTES {
        bail!("Discovery state exceeds its bounded size limit");
    }
    fs::write(&temporary, bytes)?;
    fs::rename(&temporary, &path)?;
    Ok(())
}

fn read_bounded(path: &Path) -> Result<Vec<u8>> {
    let metadata = fs::metadata(path)
        .with_context(|| format!("Discovery record {} was not found", path.display()))?;
    if !metadata.is_file() || metadata.len() > MAX_STATE_BYTES {
        bail!("Discovery record is not a bounded regular file");
    }
    Ok(fs::read(path)?)
}

fn discovery_dir() -> Result<PathBuf> {
    Ok(crate::storage::jcode_dir()?
        .join("skill-crystallization")
        .join("discovery"))
}

fn suggestions_dir() -> Result<PathBuf> {
    Ok(discovery_dir()?.join("suggestions"))
}

fn state_path() -> Result<PathBuf> {
    Ok(discovery_dir()?.join("state.json"))
}

fn create_private_dir_all(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
