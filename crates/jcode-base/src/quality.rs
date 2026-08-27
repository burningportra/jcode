use anyhow::{Context, Result, anyhow, bail};
use jcode_quality_types::{
    COMPLEXITY_STANDARD_ID, CandidateFingerprint, ResolvedStandard, SourceKind, StandardSource,
};
use regex::Regex;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

const OXLINT_CONFIG_NAMES: &[&str] = &[
    ".oxlintrc.json",
    ".oxlintrc",
    "oxlint.json",
    "oxlint.config.js",
    "oxlint.config.cjs",
    "oxlint.config.mjs",
    "oxlint.config.ts",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedFile {
    pub path: String,
    pub content_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateManifest {
    pub repository_root: PathBuf,
    pub baseline_commit: Option<String>,
    pub candidate_head: Option<String>,
    pub candidate_tree: Option<String>,
    pub changed_files: Vec<ChangedFile>,
    pub changed_js_ts_files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OxlintPolicyState {
    Configured {
        threshold: u64,
    },
    RuleNotConfigured,
    UntrustedWeakening {
        current_threshold: Option<u64>,
        baseline_threshold: Option<u64>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedOxlintPolicy {
    pub config_path: Option<PathBuf>,
    pub config_relative_path: Option<String>,
    pub config_digest: Option<String>,
    pub state: OxlintPolicyState,
    pub standard: Option<ResolvedStandard>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualityWorkspaceSnapshot {
    pub manifest: CandidateManifest,
    pub analyzed_scope: Vec<String>,
    pub complete_changed_scope: bool,
    pub policy: ResolvedOxlintPolicy,
    pub fingerprint: CandidateFingerprint,
}

pub fn inspect_quality_workspace(
    working_dir: &Path,
    requested_scope: Option<&[String]>,
    analyzer_identity: &str,
) -> Result<QualityWorkspaceSnapshot> {
    let manifest = build_candidate_manifest(working_dir)?;
    let policy = discover_oxlint_policy(&manifest)?;
    let all_scope = manifest.changed_js_ts_files.clone();
    let analyzed_scope = match requested_scope {
        None => all_scope.clone(),
        Some(paths) => normalize_requested_scope(&manifest.repository_root, paths, &all_scope)?,
    };
    let complete_changed_scope = analyzed_scope == all_scope;
    let fingerprint = fingerprint(&manifest, &analyzed_scope, &policy, analyzer_identity);
    Ok(QualityWorkspaceSnapshot {
        manifest,
        analyzed_scope,
        complete_changed_scope,
        policy,
        fingerprint,
    })
}

pub fn build_candidate_manifest(working_dir: &Path) -> Result<CandidateManifest> {
    let repository_root = repository_root(working_dir)?;
    let candidate_head = git_optional(&repository_root, &["rev-parse", "HEAD"]);
    let candidate_tree = git_optional(&repository_root, &["rev-parse", "HEAD^{tree}"]);
    let upstream = git_optional(
        &repository_root,
        &[
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ],
    );
    let baseline_commit = upstream
        .as_deref()
        .and_then(|upstream| git_optional(&repository_root, &["merge-base", "HEAD", upstream]))
        .or_else(|| candidate_head.clone());

    let mut paths = BTreeSet::new();
    if let Some(baseline) = baseline_commit.as_deref() {
        for path in git_nul_paths(
            &repository_root,
            &["diff", "--name-only", "--diff-filter=ACMR", "-z", baseline],
        )? {
            paths.insert(path);
        }
    }
    for path in git_nul_paths(
        &repository_root,
        &["ls-files", "--others", "--exclude-standard", "-z"],
    )? {
        paths.insert(path);
    }

    let mut changed_files = Vec::new();
    for path in paths {
        let absolute = repository_root.join(&path);
        if !absolute.is_file() {
            continue;
        }
        let content = fs::read(&absolute)
            .with_context(|| format!("read changed file {}", absolute.display()))?;
        changed_files.push(ChangedFile {
            path,
            content_digest: sha256_bytes(&content),
        });
    }
    let changed_js_ts_files = changed_files
        .iter()
        .filter(|file| is_js_ts_path(&file.path))
        .map(|file| file.path.clone())
        .collect();

    Ok(CandidateManifest {
        repository_root,
        baseline_commit,
        candidate_head,
        candidate_tree,
        changed_files,
        changed_js_ts_files,
    })
}

pub fn discover_oxlint_policy(manifest: &CandidateManifest) -> Result<ResolvedOxlintPolicy> {
    let Some(config_path) = OXLINT_CONFIG_NAMES
        .iter()
        .map(|name| manifest.repository_root.join(name))
        .find(|path| path.is_file())
    else {
        return Ok(ResolvedOxlintPolicy {
            config_path: None,
            config_relative_path: None,
            config_digest: None,
            state: OxlintPolicyState::RuleNotConfigured,
            standard: None,
        });
    };
    let relative = config_path
        .strip_prefix(&manifest.repository_root)
        .map_err(|_| anyhow!("Oxlint config escaped repository root"))?
        .to_string_lossy()
        .replace('\\', "/");
    let content = fs::read_to_string(&config_path)
        .with_context(|| format!("read Oxlint config {}", config_path.display()))?;
    let digest = sha256_bytes(content.as_bytes());
    let current_threshold = parse_complexity_threshold(&content);
    let config_changed = manifest
        .changed_files
        .iter()
        .any(|file| file.path == relative);
    let baseline_threshold = if config_changed {
        manifest.baseline_commit.as_deref().and_then(|baseline| {
            git_show_file(&manifest.repository_root, baseline, &relative)
                .and_then(|text| parse_complexity_threshold(&text))
        })
    } else {
        current_threshold
    };
    let weakening = config_changed
        && baseline_threshold
            .map(|baseline| current_threshold.is_none_or(|current| current > baseline))
            .unwrap_or(true);

    let state = if weakening {
        OxlintPolicyState::UntrustedWeakening {
            current_threshold,
            baseline_threshold,
        }
    } else if let Some(threshold) = current_threshold {
        OxlintPolicyState::Configured { threshold }
    } else {
        OxlintPolicyState::RuleNotConfigured
    };
    let standard = match state {
        OxlintPolicyState::Configured { threshold } => Some(ResolvedStandard {
            id: COMPLEXITY_STANDARD_ID.into(),
            analyzer: "oxlint".into(),
            rule: "eslint/complexity".into(),
            threshold: Some(threshold),
            required: true,
            source: StandardSource {
                kind: SourceKind::RepositoryToolConfig,
                locator: relative.clone(),
                version: None,
                digest: Some(digest.clone()),
            },
            trusted: true,
        }),
        _ => None,
    };
    Ok(ResolvedOxlintPolicy {
        config_path: Some(config_path),
        config_relative_path: Some(relative),
        config_digest: Some(digest),
        state,
        standard,
    })
}

pub fn parse_complexity_threshold(content: &str) -> Option<u64> {
    static COMPLEXITY_RE: OnceLock<Regex> = OnceLock::new();
    let regex = COMPLEXITY_RE.get_or_init(|| {
        Regex::new(
            r#"(?is)[\"'](?:eslint/)?complexity[\"']\s*:\s*(?:\[\s*[\"'](?:error|warn)[\"']\s*,\s*|\{[^}]*?(?:max|threshold)\s*:\s*)?(\d+)"#,
        )
        .expect("valid complexity config regex")
    });
    regex
        .captures(content)
        .and_then(|captures| captures.get(1))
        .and_then(|value| value.as_str().parse().ok())
        .filter(|threshold| *threshold > 0)
}

fn normalize_requested_scope(
    root: &Path,
    requested: &[String],
    all_scope: &[String],
) -> Result<Vec<String>> {
    let allowed: BTreeSet<&str> = all_scope.iter().map(String::as_str).collect();
    let canonical_root = root
        .canonicalize()
        .context("canonicalize repository root")?;
    let mut normalized = BTreeSet::new();
    for raw in requested {
        let absolute = root.join(raw);
        let canonical = absolute
            .canonicalize()
            .with_context(|| format!("resolve quality scope {raw}"))?;
        if !canonical.starts_with(&canonical_root) {
            bail!("quality scope escapes repository root: {raw}");
        }
        let relative = canonical
            .strip_prefix(&canonical_root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if !allowed.contains(relative.as_str()) {
            bail!("quality scope is not a changed JS/TS file: {raw}");
        }
        normalized.insert(relative);
    }
    Ok(normalized.into_iter().collect())
}

fn fingerprint(
    manifest: &CandidateManifest,
    analyzed_scope: &[String],
    policy: &ResolvedOxlintPolicy,
    analyzer_identity: &str,
) -> CandidateFingerprint {
    let manifest_material = manifest
        .changed_files
        .iter()
        .map(|file| format!("{}\0{}", file.path, file.content_digest))
        .collect::<Vec<_>>()
        .join("\0");
    let policy_material = format!(
        "{:?}\0{}\0{}",
        policy.state,
        policy.config_relative_path.as_deref().unwrap_or(""),
        policy.config_digest.as_deref().unwrap_or("")
    );
    CandidateFingerprint {
        repository_identity_digest: sha256_bytes(
            manifest.repository_root.to_string_lossy().as_bytes(),
        ),
        baseline_commit: manifest.baseline_commit.clone(),
        candidate_head: manifest.candidate_head.clone(),
        candidate_tree: manifest.candidate_tree.clone(),
        changed_manifest_digest: sha256_bytes(manifest_material.as_bytes()),
        analyzed_scope_digest: sha256_bytes(analyzed_scope.join("\0").as_bytes()),
        policy_digest: sha256_bytes(policy_material.as_bytes()),
        analyzers_digest: sha256_bytes(analyzer_identity.as_bytes()),
    }
}

fn repository_root(working_dir: &Path) -> Result<PathBuf> {
    if let Some(root) = git_optional(working_dir, &["rev-parse", "--show-toplevel"]) {
        return PathBuf::from(root)
            .canonicalize()
            .context("canonicalize Git repository root");
    }
    working_dir
        .canonicalize()
        .context("canonicalize quality working directory")
}

fn git_optional(root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|value| !value.is_empty())
}

fn git_nul_paths(root: &Path, args: &[&str]) -> Result<Vec<String>> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .with_context(|| format!("run git {}", args.join(" ")))?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| String::from_utf8_lossy(path).replace('\\', "/"))
        .collect())
}

fn git_show_file(root: &Path, commit: &str, path: &str) -> Option<String> {
    let spec = format!("{commit}:{path}");
    let output = Command::new("git")
        .args(["show", &spec])
        .current_dir(root)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

fn is_js_ts_path(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "mts" | "cts"
            )
        })
}

pub fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_json_and_javascript_complexity_rules() {
        assert_eq!(
            parse_complexity_threshold(r#"{"rules":{"eslint/complexity":["error",15]}}"#),
            Some(15)
        );
        assert_eq!(
            parse_complexity_threshold("rules: { complexity: ['warn', 9] }"),
            None,
            "unquoted keys are not accepted because the effective rule cannot be proven"
        );
        assert_eq!(
            parse_complexity_threshold("rules: { 'complexity': ['warn', 9] }"),
            Some(9)
        );
        assert_eq!(
            parse_complexity_threshold(r#"{"rules":{"complexity":"off"}}"#),
            None
        );
    }
}
