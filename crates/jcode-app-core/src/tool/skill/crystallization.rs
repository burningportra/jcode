use crate::session::{Session, durable_conversation_evidence_text};
use crate::skill::SkillRegistry;
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use jcode_message_types::Role;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const SCHEMA_VERSION: u32 = 1;
const MAX_NAME_BYTES: usize = 64;
const MAX_DESCRIPTION_BYTES: usize = 500;
const MAX_BODY_BYTES: usize = 64 * 1024;
const MAX_ID_BYTES: usize = 256;
const MAX_EVIDENCE: usize = 12;
const MAX_EXCERPT_CHARS: usize = 500;
const MAX_PENDING: usize = 1_000;
const MAX_PROPOSAL_BYTES: u64 = 128 * 1024;

pub struct OperationLock(fs::File);

impl Drop for OperationLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        unsafe {
            libc::flock(std::os::fd::AsRawFd::as_raw_fd(&self.0), libc::LOCK_UN);
        }
        #[cfg(windows)]
        unsafe {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::Storage::FileSystem::UnlockFileEx;
            let mut overlapped = std::mem::zeroed();
            UnlockFileEx(
                self.0.as_raw_handle() as _,
                0,
                u32::MAX,
                u32::MAX,
                &mut overlapped,
            );
        }
    }
}

pub fn acquire_operation_lock() -> Result<OperationLock> {
    let dir = proposal_dir()?;
    create_private_dir_all(&dir)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join(".operation.lock"))?;
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
        if result != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{LOCKFILE_EXCLUSIVE_LOCK, LockFileEx};
        let mut overlapped = unsafe { std::mem::zeroed() };
        let result = unsafe {
            LockFileEx(
                file.as_raw_handle() as _,
                LOCKFILE_EXCLUSIVE_LOCK,
                0,
                u32::MAX,
                u32::MAX,
                &mut overlapped,
            )
        };
        if result == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    Ok(OperationLock(file))
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EvidenceReference {
    pub session_id: String,
    pub message_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerifiedEvidence {
    pub session_id: String,
    pub message_id: String,
    pub role: String,
    pub timestamp: Option<DateTime<Utc>>,
    pub excerpt: String,
    pub message_digest: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Proposal {
    pub schema_version: u32,
    pub proposal_id: String,
    pub name: String,
    pub description: String,
    pub content: String,
    pub content_fingerprint: String,
    pub evidence: Vec<VerifiedEvidence>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct CanonicalProposal<'a> {
    schema_version: u32,
    name: &'a str,
    description: &'a str,
    content: &'a str,
    content_fingerprint: &'a str,
    evidence: &'a [VerifiedEvidence],
}

pub fn propose(
    registry: &SkillRegistry,
    name: Option<String>,
    description: Option<String>,
    content: Option<String>,
    evidence: Option<Vec<EvidenceReference>>,
) -> Result<Proposal> {
    let name = validate_name(name.as_deref().unwrap_or_default())?;
    let description =
        bounded_required(description.as_deref(), "description", MAX_DESCRIPTION_BYTES)?;
    let content = bounded_required(content.as_deref(), "content", MAX_BODY_BYTES)?;
    let verified = verify_evidence(evidence.as_deref().unwrap_or_default())?;
    let fingerprint = fingerprint_content(&content);
    check_deduplication(registry, &name, &fingerprint, None, false)?;

    let mut proposal = Proposal {
        schema_version: SCHEMA_VERSION,
        proposal_id: String::new(),
        name,
        description,
        content,
        content_fingerprint: fingerprint,
        evidence: verified,
        created_at: Utc::now(),
    };
    proposal.proposal_id = compute_proposal_id(&proposal)?;
    write_proposal_noclobber(&proposal)?;
    Ok(proposal)
}

pub fn load_for_approval(proposal_id: &str) -> Result<Proposal> {
    validate_proposal_id(proposal_id)?;
    let path = proposal_dir()?.join(format!("{proposal_id}.json"));
    let proposal = read_bounded_proposal(&path)?;
    validate_persisted(&proposal, proposal_id)?;
    Ok(proposal)
}

pub fn revalidate_for_approval(registry: &SkillRegistry, proposal: &Proposal) -> Result<()> {
    validate_persisted(proposal, &proposal.proposal_id)?;
    let references = proposal
        .evidence
        .iter()
        .map(|item| EvidenceReference {
            session_id: item.session_id.clone(),
            message_id: item.message_id.clone(),
        })
        .collect::<Vec<_>>();
    let current = verify_evidence(&references)?;
    if current != proposal.evidence {
        bail!("Crystallization evidence changed after proposal creation");
    }
    check_deduplication(
        registry,
        &proposal.name,
        &proposal.content_fingerprint,
        Some(&proposal.proposal_id),
        true,
    )
}

pub fn verify_approval(proposal: &Proposal, reference: &EvidenceReference) -> Result<()> {
    validate_component_id("approval session_id", &reference.session_id)?;
    validate_component_id("approval message_id", &reference.message_id)?;
    let session = Session::load(&reference.session_id)
        .with_context(|| format!("Approval session '{}' was not found", reference.session_id))?;
    if session.id != reference.session_id {
        bail!("Approval session stored ID does not match requested ID");
    }
    let message = session
        .messages
        .iter()
        .find(|message| message.id == reference.message_id)
        .with_context(|| format!("Approval message '{}' was not found", reference.message_id))?;
    if message.role != Role::User {
        bail!("Approval evidence must reference a persisted user message");
    }
    let timestamp = message
        .timestamp
        .context("Approval message must have a persisted timestamp")?;
    if timestamp <= proposal.created_at {
        bail!("Approval message predates the proposal");
    }
    let text = durable_conversation_evidence_text(message)
        .context("Approval evidence is not an eligible conversation message")?;
    let normalized = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    let expected = format!(
        "i approve skill crystallization proposal {}.",
        proposal.proposal_id
    );
    if normalized != expected {
        bail!("Approval message must exactly match: {expected}");
    }
    Ok(())
}

pub fn install_skill(proposal: &Proposal) -> Result<(PathBuf, String)> {
    let root = crate::storage::jcode_dir()?.join("skills");
    fs::create_dir_all(&root)?;
    let skill_dir = root.join(&proposal.name);
    let path = skill_dir.join("SKILL.md");
    let preview = skill_preview(proposal);
    if let Ok(metadata) = fs::symlink_metadata(&skill_dir) {
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            bail!(
                "Skill destination is not a regular directory: {}",
                skill_dir.display()
            );
        }
        let file_metadata = fs::symlink_metadata(&path).with_context(|| {
            format!("Skill destination already exists: {}", skill_dir.display())
        })?;
        if file_metadata.file_type().is_symlink() || !file_metadata.is_file() {
            bail!(
                "Skill destination is not a regular file: {}",
                path.display()
            );
        }
        if fs::read_to_string(&path)? == preview {
            return Ok((path, preview));
        }
        bail!("Skill destination already exists: {}", skill_dir.display());
    }

    let staging = root.join(format!(".crystallize-{}", proposal.proposal_id));
    if !staging.exists() {
        fs::create_dir(&staging)?;
    } else {
        let metadata = fs::symlink_metadata(&staging)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            bail!(
                "Crystallization staging path is unsafe: {}",
                staging.display()
            );
        }
    }
    let staged_path = staging.join("SKILL.md");
    let write_result = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staged_path)
        .and_then(|mut file| {
            file.write_all(preview.as_bytes())?;
            file.sync_all()
        });
    if let Err(error) = write_result {
        if !staged_path.is_file() || fs::read_to_string(&staged_path)? != preview {
            return Err(error)
                .with_context(|| format!("Failed to create {}", staged_path.display()));
        }
    }
    fs::rename(&staging, &skill_dir)
        .with_context(|| format!("Failed to install {}", skill_dir.display()))?;
    Ok((path, preview))
}

pub fn verify_reloaded(
    registry: &SkillRegistry,
    proposal: &Proposal,
    expected_path: &Path,
) -> Result<()> {
    let skill = registry.get(&proposal.name).with_context(|| {
        format!(
            "Installed skill '{}' was not readable after registry reload",
            proposal.name
        )
    })?;
    if skill.path != expected_path {
        bail!(
            "Installed skill '{}' reloaded from unexpected path {} instead of {}",
            proposal.name,
            skill.path.display(),
            expected_path.display()
        );
    }
    if fingerprint_content(&skill.content) != proposal.content_fingerprint {
        bail!(
            "Installed skill '{}' content failed reload verification",
            proposal.name
        );
    }
    Ok(())
}

pub fn archive(proposal: &Proposal) -> Result<PathBuf> {
    let approved = proposal_dir()?.join("approved");
    create_private_dir_all(&approved)?;
    let destination = approved.join(format!("{}.json", proposal.proposal_id));
    if destination.exists() {
        let archived = read_bounded_proposal(&destination)?;
        validate_persisted(&archived, &proposal.proposal_id)?;
        if archived != *proposal {
            bail!("Approved proposal archive does not match the pending proposal");
        }
    } else {
        write_json_noclobber(&destination, proposal)?;
    }
    let pending = proposal_dir()?.join(format!("{}.json", proposal.proposal_id));
    if pending.exists() {
        fs::remove_file(pending)?;
    }
    Ok(destination)
}

pub fn skill_preview(proposal: &Proposal) -> String {
    format!(
        "---\nname: {}\ndescription: {}\n---\n\n{}",
        serde_json::to_string(&proposal.name).expect("string serialization cannot fail"),
        serde_json::to_string(&proposal.description).expect("string serialization cannot fail"),
        proposal.content
    )
}

fn validate_persisted(proposal: &Proposal, expected_id: &str) -> Result<()> {
    if proposal.schema_version != SCHEMA_VERSION {
        bail!(
            "Unsupported crystallization proposal schema version: {}",
            proposal.schema_version
        );
    }
    validate_proposal_id(expected_id)?;
    if proposal.proposal_id != expected_id {
        bail!("Crystallization proposal ID does not match its filename");
    }
    validate_name(&proposal.name)?;
    bounded_required(
        Some(&proposal.description),
        "description",
        MAX_DESCRIPTION_BYTES,
    )?;
    bounded_required(Some(&proposal.content), "content", MAX_BODY_BYTES)?;
    if fingerprint_content(&proposal.content) != proposal.content_fingerprint {
        bail!("Crystallization proposal content fingerprint is invalid");
    }
    if compute_proposal_id(proposal)? != expected_id {
        bail!("Crystallization proposal was modified under its approved ID");
    }
    Ok(())
}

fn verify_evidence(references: &[EvidenceReference]) -> Result<Vec<VerifiedEvidence>> {
    if !(2..=MAX_EVIDENCE).contains(&references.len()) {
        bail!("evidence must contain between 2 and {MAX_EVIDENCE} references");
    }
    let mut sessions = HashSet::new();
    let mut verified = Vec::with_capacity(references.len());
    for reference in references {
        validate_component_id("session_id", &reference.session_id)?;
        validate_component_id("message_id", &reference.message_id)?;
        if !sessions.insert(reference.session_id.clone()) {
            bail!("evidence must come from distinct sessions");
        }
        let session = Session::load(&reference.session_id).with_context(|| {
            format!("Evidence session '{}' was not found", reference.session_id)
        })?;
        if session.id != reference.session_id {
            bail!("Evidence session stored ID does not match requested ID");
        }
        let message = session
            .messages
            .iter()
            .find(|message| message.id == reference.message_id)
            .with_context(|| {
                format!("Evidence message '{}' was not found", reference.message_id)
            })?;
        let text = durable_conversation_evidence_text(message).with_context(|| {
            format!(
                "Evidence message '{}' is not eligible persisted user or assistant text",
                reference.message_id
            )
        })?;
        let message_digest = hex_digest(&serde_json::to_vec(message)?);
        verified.push(VerifiedEvidence {
            session_id: reference.session_id.clone(),
            message_id: reference.message_id.clone(),
            role: match message.role {
                Role::User => "user",
                Role::Assistant => "assistant",
            }
            .to_string(),
            timestamp: message.timestamp,
            excerpt: bounded_excerpt(&text),
            message_digest,
        });
    }
    verified.sort_by(|a, b| (&a.session_id, &a.message_id).cmp(&(&b.session_id, &b.message_id)));
    Ok(verified)
}

fn bounded_excerpt(text: &str) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = normalized.chars();
    let excerpt = chars.by_ref().take(MAX_EXCERPT_CHARS).collect::<String>();
    if chars.next().is_some() {
        format!("{excerpt}…")
    } else {
        excerpt
    }
}

fn validate_name(value: &str) -> Result<String> {
    if value.is_empty() || value.len() > MAX_NAME_BYTES {
        bail!("name must be 1 to {MAX_NAME_BYTES} bytes");
    }
    if value == "." || value == ".." || value.starts_with('-') || value.ends_with('-') {
        bail!("name must be lowercase kebab-case");
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        bail!("name must be lowercase kebab-case");
    }
    Ok(value.to_string())
}

fn bounded_required(value: Option<&str>, field: &str, max: usize) -> Result<String> {
    let value = value.unwrap_or_default();
    if value.trim().is_empty() || value.len() > max {
        bail!("{field} must be non-empty and at most {max} bytes");
    }
    Ok(value.to_string())
}

fn validate_component_id(field: &str, value: &str) -> Result<()> {
    if value.is_empty() || value.len() > MAX_ID_BYTES || value == "." || value == ".." {
        bail!("{field} must be a bounded non-empty path component");
    }
    if value.contains('/') || value.contains('\\') || value.contains('\0') {
        bail!("{field} must be a single path component");
    }
    Ok(())
}

fn validate_proposal_id(value: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        bail!("proposal_id must be 64 lowercase hexadecimal characters");
    }
    Ok(())
}

fn normalize_content(content: &str) -> String {
    let normalized_line_endings = content.replace("\r\n", "\n").replace('\r', "\n");
    let mut lines = normalized_line_endings
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>();
    while lines.first().is_some_and(|line| line.is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
    let mut output = Vec::new();
    let mut blanks = 0;
    for line in lines {
        if line.is_empty() {
            blanks += 1;
        } else {
            blanks = 0;
        }
        if blanks <= 2 {
            output.push(line);
        }
    }
    output.join("\n")
}

fn fingerprint_content(content: &str) -> String {
    hex_digest(normalize_content(content).as_bytes())
}

fn compute_proposal_id(proposal: &Proposal) -> Result<String> {
    let canonical = CanonicalProposal {
        schema_version: proposal.schema_version,
        name: &proposal.name,
        description: &proposal.description,
        content: &proposal.content,
        content_fingerprint: &proposal.content_fingerprint,
        evidence: &proposal.evidence,
    };
    Ok(hex_digest(&serde_json::to_vec(&canonical)?))
}

fn hex_digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn check_deduplication(
    registry: &SkillRegistry,
    name: &str,
    fingerprint: &str,
    exclude: Option<&str>,
    allow_exact_installed: bool,
) -> Result<()> {
    if registry.get(name).is_some()
        && !(allow_exact_installed && exact_canonical_skill_exists(name, fingerprint)?)
    {
        bail!("A global skill named '{name}' already exists");
    }
    let root = crate::storage::jcode_dir()?.join("skills");
    if root.is_dir() {
        for entry in fs::read_dir(&root)?.flatten() {
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let path = entry.path().join("SKILL.md");
            if !path.is_file() {
                continue;
            }
            let raw = fs::read_to_string(&path)?;
            let installed_name = entry.file_name().to_string_lossy().into_owned();
            let installed_fingerprint = fingerprint_content(strip_skill_frontmatter(&raw));
            if allow_exact_installed
                && installed_name == name
                && installed_fingerprint == fingerprint
            {
                continue;
            }
            if installed_name == name {
                bail!("A global skill named '{name}' already exists");
            }
            if installed_fingerprint == fingerprint {
                bail!("An installed global skill has the same normalized content");
            }
        }
    }
    for pending in pending_proposals()? {
        if exclude == Some(pending.proposal_id.as_str()) {
            continue;
        }
        if pending.name == name {
            bail!("A pending proposal named '{name}' already exists");
        }
        if pending.content_fingerprint == fingerprint {
            bail!("A pending proposal has the same normalized content");
        }
    }
    Ok(())
}

fn exact_canonical_skill_exists(name: &str, fingerprint: &str) -> Result<bool> {
    let path = crate::storage::jcode_dir()?
        .join("skills")
        .join(name)
        .join("SKILL.md");
    if !path.is_file() {
        return Ok(false);
    }
    let raw = fs::read_to_string(path)?;
    Ok(fingerprint_content(strip_skill_frontmatter(&raw)) == fingerprint)
}

fn strip_skill_frontmatter(raw: &str) -> &str {
    let normalized = raw.strip_prefix("---\n").unwrap_or(raw);
    normalized
        .find("\n---\n")
        .map(|end| &normalized[end + 5..])
        .unwrap_or(raw)
}

fn proposal_dir() -> Result<PathBuf> {
    Ok(crate::storage::jcode_dir()?.join("skill-proposals"))
}

fn pending_proposals() -> Result<Vec<Proposal>> {
    let dir = proposal_dir()?;
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut paths = fs::read_dir(&dir)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
        .collect::<Vec<_>>();
    paths.sort();
    if paths.len() > MAX_PENDING {
        bail!("Too many pending skill crystallization proposals");
    }
    paths
        .into_iter()
        .map(|path| read_bounded_proposal(&path))
        .collect()
}

fn read_bounded_proposal(path: &Path) -> Result<Proposal> {
    let file = fs::File::open(path)?;
    if file.metadata()?.len() > MAX_PROPOSAL_BYTES {
        bail!("Crystallization proposal exceeds {MAX_PROPOSAL_BYTES} bytes");
    }
    let mut bytes = Vec::new();
    file.take(MAX_PROPOSAL_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_PROPOSAL_BYTES {
        bail!("Crystallization proposal exceeds {MAX_PROPOSAL_BYTES} bytes");
    }
    Ok(serde_json::from_slice(&bytes)?)
}

fn write_proposal_noclobber(proposal: &Proposal) -> Result<()> {
    let dir = proposal_dir()?;
    create_private_dir_all(&dir)?;
    write_json_noclobber(
        &dir.join(format!("{}.json", proposal.proposal_id)),
        proposal,
    )
}

fn write_json_noclobber<T: Serialize>(destination: &Path, value: &T) -> Result<()> {
    let parent = destination.parent().context("destination has no parent")?;
    create_private_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temp.as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    serde_json::to_writer_pretty(&mut temp, value)?;
    temp.write_all(b"\n")?;
    temp.as_file().sync_all()?;
    temp.persist_noclobber(destination).map_err(|error| {
        anyhow::anyhow!(
            "Destination already exists: {}: {}",
            destination.display(),
            error.error
        )
    })?;
    Ok(())
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
