use super::{Tool, ToolContext, ToolOutput};
use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use jcode_quality_types::{
    AntiSlopDeclaration, COMPLEXITY_STANDARD_ID, EvidenceKind, EvidenceRef,
    ObservationClassification, ObservationResult, QUALITY_REPORT_SCHEMA_VERSION, QualityReport,
    QualitySubject, QualityVerdict, StandardObservation,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::process::Command;

const OXLINT_TIMEOUT: Duration = Duration::from_secs(30);
const EVIDENCE_SUMMARY_LIMIT: usize = 500;

pub struct QualityTool;

impl QualityTool {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug, Deserialize)]
struct QualityInput {
    #[serde(default = "default_action")]
    action: String,
    #[serde(default)]
    files: Option<Vec<String>>,
    #[serde(default)]
    anti_slop: Option<AntiSlopDeclaration>,
}

fn default_action() -> String {
    "check".into()
}

#[derive(Debug)]
struct AnalyzerRun {
    identity: String,
    executable: PathBuf,
    version: String,
    status: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    timed_out: bool,
}

#[async_trait]
impl Tool for QualityTool {
    fn name(&self) -> &str {
        "quality"
    }

    fn description(&self) -> &str {
        "Check sourced engineering standards for changed code. The first slice runs an existing Oxlint complexity rule and returns a typed report."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["action"],
            "properties": {
                "intent": super::intent_schema_property(),
                "action": {
                    "type": "string",
                    "enum": ["check"],
                    "description": "Quality action. Only check is supported in the report-only first slice."
                },
                "files": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Optional diagnostic subset of changed JS/TS files. Omit for complete changed-file coverage."
                },
                "anti_slop": {
                    "type": "object",
                    "description": "Optional advisory accounting for abstractions, configuration, retained paths, and simplification.",
                    "properties": {
                        "removed_before_added": {"type": "string"},
                        "reused_existing_mechanism": {"type": "string"},
                        "new_abstractions": {"type": "array", "items": {"type": "object"}},
                        "retained_legacy_paths": {"type": "array", "items": {"type": "object"}},
                        "new_configuration": {"type": "array", "items": {"type": "object"}},
                        "simplification_considered": {"type": "string"},
                        "unresolved_complexity": {"type": "array", "items": {"type": "string"}}
                    }
                }
            }
        })
    }

    async fn execute(&self, input: Value, ctx: ToolContext) -> Result<ToolOutput> {
        let input: QualityInput = serde_json::from_value(input)?;
        if input.action != "check" {
            return Err(anyhow!("quality supports only action='check'"));
        }
        if !crate::config::config().quality.enabled {
            return Err(anyhow!(
                "quality check is disabled; set [quality].enabled = true in Jcode config"
            ));
        }
        let root = ctx
            .working_dir
            .clone()
            .ok_or_else(|| anyhow!("quality check requires a session working directory"))?;
        execute_quality_check(root, input).await
    }
}

async fn execute_quality_check(root: PathBuf, input: QualityInput) -> Result<ToolOutput> {
    let executable = resolve_oxlint(&root);
    let version = match executable.as_ref() {
        Some(path) => oxlint_version(path, &root)
            .await
            .unwrap_or_else(|_| "unknown".into()),
        None => "missing".into(),
    };
    let analyzer_identity = executable
        .as_ref()
        .map(|path| format!("{}\0{}", path.display(), version))
        .unwrap_or_else(|| "oxlint:missing".into());
    let requested_scope = input.files.clone();
    let before = tokio::task::spawn_blocking({
        let root = root.clone();
        let analyzer_identity = analyzer_identity.clone();
        move || {
            crate::quality::inspect_quality_workspace(
                &root,
                requested_scope.as_deref(),
                &analyzer_identity,
            )
        }
    })
    .await
    .context("quality workspace inspection task failed")??;

    let report_id = format!(
        "quality-{}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
        &before.fingerprint.changed_manifest_digest[..12]
    );
    let mut evidence = manifest_evidence(&report_id, &before);
    let mut observations = Vec::new();
    let mut verdict = QualityVerdict::Incomplete;
    let mut analyzer_run = None;

    match &before.policy.state {
        crate::quality::OxlintPolicyState::RuleNotConfigured => {
            observations.push(StandardObservation {
                standard_id: COMPLEXITY_STANDARD_ID.into(),
                result: ObservationResult::Unknown,
                classification: ObservationClassification::RuleNotConfigured,
                message:
                    "Oxlint eslint/complexity is absent or disabled in repository configuration"
                        .into(),
                evidence_ids: before
                    .policy
                    .config_relative_path
                    .as_ref()
                    .map(|_| vec!["policy-config".into()])
                    .unwrap_or_default(),
                path: before.policy.config_relative_path.clone(),
                line: None,
            });
        }
        crate::quality::OxlintPolicyState::UntrustedWeakening {
            current_threshold,
            baseline_threshold,
        } => {
            observations.push(StandardObservation {
                standard_id: COMPLEXITY_STANDARD_ID.into(),
                result: ObservationResult::Fail,
                classification: ObservationClassification::PolicyChanged,
                message: format!(
                    "Candidate change weakens or disables eslint/complexity: baseline={baseline_threshold:?}, candidate={current_threshold:?}"
                ),
                evidence_ids: vec!["policy-config".into()],
                path: before.policy.config_relative_path.clone(),
                line: None,
            });
            verdict = QualityVerdict::Fail;
        }
        crate::quality::OxlintPolicyState::Configured { threshold } => {
            if let Some(executable) = executable {
                let run = run_oxlint(
                    executable,
                    &before.manifest.repository_root,
                    &before.analyzed_scope,
                    version,
                )
                .await?;
                evidence.push(command_evidence(&report_id, &run));
                if run.timed_out {
                    observations.push(unknown_observation(
                        ObservationClassification::AnalyzerFailed,
                        "Oxlint timed out before producing a trustworthy result",
                        vec!["command-0".into()],
                    ));
                } else {
                    match parse_oxlint_findings(&run.stdout, &run.stderr) {
                        Ok(findings) => {
                            if findings.is_empty() {
                                observations.push(StandardObservation {
                                    standard_id: COMPLEXITY_STANDARD_ID.into(),
                                    result: ObservationResult::Pass,
                                    classification: ObservationClassification::CandidateThresholdClear,
                                    message: format!(
                                        "Oxlint found no eslint/complexity violations above max {threshold} in {} changed JS/TS file(s)",
                                        before.analyzed_scope.len()
                                    ),
                                    evidence_ids: vec!["command-0".into()],
                                    path: None,
                                    line: None,
                                });
                                verdict = QualityVerdict::Pass;
                            } else {
                                for (index, finding) in findings.into_iter().enumerate() {
                                    let evidence_id = format!("finding-{index}");
                                    evidence.push(EvidenceRef {
                                        id: evidence_id.clone(),
                                        kind: EvidenceKind::AnalyzerFinding,
                                        locator: finding_locator(&finding),
                                        summary: bounded(&finding.message),
                                        digest: None,
                                    });
                                    observations.push(StandardObservation {
                                        standard_id: COMPLEXITY_STANDARD_ID.into(),
                                        result: ObservationResult::Fail,
                                        classification: ObservationClassification::CandidateThresholdViolation,
                                        message: format!(
                                            "Oxlint eslint/complexity exceeds configured max {threshold}: {}",
                                            finding.message
                                        ),
                                        evidence_ids: vec!["command-0".into(), evidence_id],
                                        path: finding.path,
                                        line: finding.line,
                                    });
                                }
                                verdict = QualityVerdict::Fail;
                            }
                        }
                        Err(error) => observations.push(unknown_observation(
                            ObservationClassification::AnalyzerFailed,
                            &format!("Oxlint output could not be parsed: {error}"),
                            vec!["command-0".into()],
                        )),
                    }
                }
                analyzer_run = Some(run);
            } else {
                observations.push(unknown_observation(
                    ObservationClassification::ToolMissing,
                    "Oxlint is not installed in node_modules/.bin or PATH",
                    vec![],
                ));
            }
        }
    }

    let anti_slop_complete = input.anti_slop.as_ref().map(|declaration| {
        let issues = declaration.completeness_issues();
        evidence.push(EvidenceRef {
            id: "anti-slop-declaration".into(),
            kind: EvidenceKind::Declaration,
            locator: "declaration://anti-slop".into(),
            summary: if issues.is_empty() {
                "Anti-slop declaration is structurally complete but remains advisory".into()
            } else {
                bounded(&format!(
                    "Incomplete anti-slop declaration: {}",
                    issues.join("; ")
                ))
            },
            digest: serde_json::to_vec(declaration)
                .ok()
                .map(|bytes| crate::quality::sha256_bytes(&bytes)),
        });
        if !issues.is_empty() {
            observations.push(StandardObservation {
                standard_id: "jcode.maintainability.anti-slop-declaration@1".into(),
                result: ObservationResult::Warn,
                classification: ObservationClassification::DeclarationIncomplete,
                message: format!(
                    "Anti-slop declaration is advisory and incomplete: {}",
                    issues.join("; ")
                ),
                evidence_ids: vec!["anti-slop-declaration".into()],
                path: None,
                line: None,
            });
            if verdict == QualityVerdict::Pass {
                verdict = QualityVerdict::PassWithWarnings;
            }
        }
        issues.is_empty()
    });

    if !before.complete_changed_scope {
        observations.push(StandardObservation {
            standard_id: COMPLEXITY_STANDARD_ID.into(),
            result: ObservationResult::Warn,
            classification: ObservationClassification::DeclarationIncomplete,
            message: "Requested file scope is partial and cannot satisfy a completion gate".into(),
            evidence_ids: vec!["candidate-manifest".into()],
            path: None,
            line: None,
        });
        if verdict == QualityVerdict::Pass {
            verdict = QualityVerdict::PassWithWarnings;
        }
    }

    let after_identity = analyzer_run
        .as_ref()
        .map(|run| run.identity.as_str())
        .unwrap_or(analyzer_identity.as_str());
    let after = tokio::task::spawn_blocking({
        let root = root.clone();
        let requested_scope = input.files.clone();
        let after_identity = after_identity.to_string();
        move || {
            crate::quality::inspect_quality_workspace(
                &root,
                requested_scope.as_deref(),
                &after_identity,
            )
        }
    })
    .await
    .context("quality post-analysis inspection task failed")??;
    if before.fingerprint != after.fingerprint {
        observations.push(unknown_observation(
            ObservationClassification::CandidateChanged,
            "Candidate code, scope, policy, or analyzer identity changed during analysis; report discarded",
            vec!["candidate-manifest".into()],
        ));
        verdict = QualityVerdict::Incomplete;
    }

    let report = QualityReport {
        id: report_id,
        schema_version: QUALITY_REPORT_SCHEMA_VERSION,
        repository_root: before.manifest.repository_root.display().to_string(),
        subject: QualitySubject {
            file_scope: before.analyzed_scope.clone(),
            complete_changed_scope: before.complete_changed_scope,
            gate_eligible: false,
        },
        baseline_commit: before.manifest.baseline_commit.clone(),
        candidate: before.fingerprint,
        profile_ids: vec!["jcode:maintainable-change@1".into()],
        standards: before.policy.standard.into_iter().collect(),
        observations,
        evidence,
        anti_slop: input.anti_slop,
        anti_slop_complete,
        verdict: verdict.clone(),
        created_at_unix_secs: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    };
    let summary = render_summary(&report);
    Ok(ToolOutput::new(summary)
        .with_title("quality check")
        .with_metadata(json!({"quality_report": report})))
}

async fn oxlint_version(executable: &Path, root: &Path) -> Result<String> {
    let output = tokio::time::timeout(
        Duration::from_secs(5),
        Command::new(executable)
            .arg("--version")
            .current_dir(root)
            .stdin(Stdio::null())
            .output(),
    )
    .await
    .context("Oxlint version timed out")??;
    if !output.status.success() {
        return Err(anyhow!("Oxlint --version failed"));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

async fn run_oxlint(
    executable: PathBuf,
    root: &Path,
    files: &[String],
    version: String,
) -> Result<AnalyzerRun> {
    let identity = format!("{}\0{}", executable.display(), version);
    if files.is_empty() {
        return Ok(AnalyzerRun {
            identity,
            executable,
            version,
            status: Some(0),
            stdout: b"[]".to_vec(),
            stderr: vec![],
            timed_out: false,
        });
    }
    let mut command = Command::new(&executable);
    command
        .args(["--format", "json"])
        .args(files)
        .current_dir(root)
        .stdin(Stdio::null())
        .kill_on_drop(true);
    match tokio::time::timeout(OXLINT_TIMEOUT, command.output()).await {
        Ok(output) => {
            let output = output.context("run Oxlint")?;
            Ok(AnalyzerRun {
                identity,
                executable,
                version,
                status: output.status.code(),
                stdout: output.stdout,
                stderr: output.stderr,
                timed_out: false,
            })
        }
        Err(_) => Ok(AnalyzerRun {
            identity,
            executable,
            version,
            status: None,
            stdout: vec![],
            stderr: vec![],
            timed_out: true,
        }),
    }
}

fn resolve_oxlint(root: &Path) -> Option<PathBuf> {
    let local = root
        .join("node_modules")
        .join(".bin")
        .join(if cfg!(windows) {
            "oxlint.exe"
        } else {
            "oxlint"
        });
    if local.is_file() {
        return Some(local);
    }
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|directory| {
                directory.join(if cfg!(windows) {
                    "oxlint.exe"
                } else {
                    "oxlint"
                })
            })
            .find(|candidate| candidate.is_file())
    })
}

#[derive(Debug)]
struct OxlintFinding {
    path: Option<String>,
    line: Option<u64>,
    message: String,
}

fn parse_oxlint_findings(stdout: &[u8], stderr: &[u8]) -> Result<Vec<OxlintFinding>> {
    let bytes = if stdout.iter().any(|byte| !byte.is_ascii_whitespace()) {
        stdout
    } else {
        stderr
    };
    let value: Value = serde_json::from_slice(bytes).context("parse Oxlint JSON")?;
    let diagnostics = value
        .as_array()
        .cloned()
        .or_else(|| value.get("diagnostics").and_then(Value::as_array).cloned())
        .or_else(|| value.get("messages").and_then(Value::as_array).cloned())
        .ok_or_else(|| anyhow!("Oxlint JSON has no diagnostics array"))?;
    Ok(diagnostics
        .iter()
        .filter(|diagnostic| is_complexity_diagnostic(diagnostic))
        .map(|diagnostic| OxlintFinding {
            path: string_field(diagnostic, &["filename", "file", "path"]),
            line: u64_field(diagnostic, &["line", "line_number"]).or_else(|| {
                diagnostic
                    .get("labels")
                    .and_then(Value::as_array)
                    .and_then(|labels| labels.first())
                    .and_then(|label| u64_field(label, &["line", "line_number"]))
            }),
            message: string_field(diagnostic, &["message", "help", "description"])
                .unwrap_or_else(|| "complexity threshold violation".into()),
        })
        .collect())
}

fn is_complexity_diagnostic(value: &Value) -> bool {
    ["code", "rule", "rule_id", "ruleId"]
        .iter()
        .filter_map(|key| value.get(key).and_then(Value::as_str))
        .any(|code| code.to_ascii_lowercase().contains("complexity"))
}

fn string_field(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(key).and_then(Value::as_str))
        .map(ToOwned::to_owned)
}

fn u64_field(value: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter()
        .find_map(|key| value.get(key).and_then(Value::as_u64))
}

fn manifest_evidence(
    report_id: &str,
    snapshot: &crate::quality::QualityWorkspaceSnapshot,
) -> Vec<EvidenceRef> {
    let mut evidence = vec![EvidenceRef {
        id: "candidate-manifest".into(),
        kind: EvidenceKind::GitObject,
        locator: snapshot
            .manifest
            .candidate_head
            .as_deref()
            .map(|head| format!("git://{head}"))
            .unwrap_or_else(|| format!("command://{report_id}/manifest")),
        summary: format!(
            "{} changed file(s), {} changed JS/TS file(s), {} analyzed",
            snapshot.manifest.changed_files.len(),
            snapshot.manifest.changed_js_ts_files.len(),
            snapshot.analyzed_scope.len()
        ),
        digest: Some(snapshot.fingerprint.changed_manifest_digest.clone()),
    }];
    if let Some(path) = snapshot.policy.config_relative_path.as_ref() {
        evidence.push(EvidenceRef {
            id: "policy-config".into(),
            kind: EvidenceKind::ExternalStandard,
            locator: format!("file://{path}"),
            summary: "Repository Oxlint configuration supplying eslint/complexity".into(),
            digest: snapshot.policy.config_digest.clone(),
        });
    }
    evidence
}

fn command_evidence(report_id: &str, run: &AnalyzerRun) -> EvidenceRef {
    EvidenceRef {
        id: "command-0".into(),
        kind: EvidenceKind::CommandResult,
        locator: format!("command://{report_id}/0"),
        summary: bounded(&format!(
            "{} --format json; version={}; exit={:?}; timed_out={}; stderr={} bytes",
            run.executable.display(),
            run.version,
            run.status,
            run.timed_out,
            run.stderr.len()
        )),
        digest: Some(crate::quality::sha256_bytes(&run.stdout)),
    }
}

fn unknown_observation(
    classification: ObservationClassification,
    message: &str,
    evidence_ids: Vec<String>,
) -> StandardObservation {
    StandardObservation {
        standard_id: COMPLEXITY_STANDARD_ID.into(),
        result: ObservationResult::Unknown,
        classification,
        message: message.into(),
        evidence_ids,
        path: None,
        line: None,
    }
}

fn finding_locator(finding: &OxlintFinding) -> String {
    match (&finding.path, finding.line) {
        (Some(path), Some(line)) => format!("file://{path}#L{line}-L{line}"),
        (Some(path), None) => format!("file://{path}"),
        _ => "analyzer://oxlint/eslint-complexity".into(),
    }
}

fn bounded(value: &str) -> String {
    crate::util::truncate_str(value.trim(), EVIDENCE_SUMMARY_LIMIT).to_string()
}

fn render_summary(report: &QualityReport) -> String {
    let headline = match report.verdict {
        QualityVerdict::Pass => "Oxlint complexity: pass",
        QualityVerdict::PassWithWarnings => "Oxlint complexity: pass with advisory warnings",
        QualityVerdict::Fail => "Oxlint complexity: fail",
        QualityVerdict::Incomplete => "Oxlint complexity: unknown",
    };
    let mut lines = vec![headline.to_string()];
    lines.push(format!(
        "Scope: {} changed JS/TS file(s){}",
        report.subject.file_scope.len(),
        if report.subject.complete_changed_scope {
            ""
        } else {
            " (partial, not gate-eligible)"
        }
    ));
    for observation in report
        .observations
        .iter()
        .filter(|observation| observation.result != ObservationResult::Pass)
        .take(8)
    {
        lines.push(format!("- {}", observation.message));
    }
    lines.push("Typed report: ToolOutput.metadata.quality_report".into());
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn test_ctx(root: &Path) -> ToolContext {
        ToolContext {
            session_id: "quality-test".into(),
            message_id: "quality-test".into(),
            tool_call_id: "quality-test".into(),
            working_dir: Some(root.to_path_buf()),
            stdin_request_tx: None,
            graceful_shutdown_signal: None,
            execution_mode: super::super::ToolExecutionMode::Direct,
        }
    }

    fn git(root: &Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[cfg(unix)]
    fn fixture_repo(oxlint_json: &str) -> tempfile::TempDir {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("src")).unwrap();
        fs::create_dir_all(temp.path().join("node_modules/.bin")).unwrap();
        fs::write(
            temp.path().join(".oxlintrc.json"),
            r#"{"rules":{"eslint/complexity":["error",3]}}"#,
        )
        .unwrap();
        fs::write(
            temp.path().join("src/a.ts"),
            "export function value() { return 1; }\n",
        )
        .unwrap();
        git(temp.path(), &["init", "-q"]);
        git(
            temp.path(),
            &["config", "user.email", "quality@example.com"],
        );
        git(temp.path(), &["config", "user.name", "Quality Test"]);
        git(temp.path(), &["add", ".oxlintrc.json", "src/a.ts"]);
        git(temp.path(), &["commit", "-qm", "baseline"]);
        fs::write(
            temp.path().join("src/a.ts"),
            "export function value(x: number) { if (x) { return 1; } return 0; }\n",
        )
        .unwrap();
        let script = format!(
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo 'oxlint 1.0.0-test'; exit 0; fi\nprintf '%s\\n' '{}'\n{}\n",
            oxlint_json.replace('\'', "'\\''"),
            if oxlint_json == "[]" {
                "exit 0"
            } else {
                "exit 1"
            }
        );
        let bin = temp.path().join("node_modules/.bin/oxlint");
        fs::write(&bin, script).unwrap();
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
        temp
    }

    #[test]
    fn parses_only_complexity_diagnostics() {
        let findings = parse_oxlint_findings(
            br#"[{"code":"eslint(complexity)","message":"complexity is 18","filename":"src/a.ts","line":3},{"code":"eslint(no-debugger)","message":"debugger"}]"#,
            b"",
        )
        .unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].path.as_deref(), Some("src/a.ts"));
        assert_eq!(findings[0].line, Some(3));
    }

    #[test]
    fn summary_never_claims_general_quality_passed() {
        let report = QualityReport {
            id: "x".into(),
            schema_version: 1,
            repository_root: "/repo".into(),
            subject: QualitySubject {
                file_scope: vec![],
                complete_changed_scope: true,
                gate_eligible: false,
            },
            baseline_commit: None,
            candidate: jcode_quality_types::CandidateFingerprint {
                repository_identity_digest: "a".into(),
                baseline_commit: None,
                candidate_head: None,
                candidate_tree: None,
                changed_manifest_digest: "b".into(),
                analyzed_scope_digest: "c".into(),
                policy_digest: "d".into(),
                analyzers_digest: "e".into(),
            },
            profile_ids: vec![],
            standards: vec![],
            observations: vec![],
            evidence: vec![],
            anti_slop: None,
            anti_slop_complete: None,
            verdict: QualityVerdict::Pass,
            created_at_unix_secs: 0,
        };
        assert_eq!(
            render_summary(&report).lines().next(),
            Some("Oxlint complexity: pass")
        );
        assert!(!render_summary(&report).contains("quality passed"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn public_quality_check_returns_typed_pass_report() {
        let fixture = fixture_repo("[]");
        let output = QualityTool::new()
            .execute(
                json!({
                    "action": "check",
                    "anti_slop": {
                        "removed_before_added": "Removed no obsolete path because none existed",
                        "reused_existing_mechanism": "Used the existing function",
                        "new_abstractions": [],
                        "retained_legacy_paths": [],
                        "new_configuration": [],
                        "simplification_considered": "Kept the function direct",
                        "unresolved_complexity": []
                    }
                }),
                test_ctx(fixture.path()),
            )
            .await
            .unwrap();
        assert!(output.output.starts_with("Oxlint complexity: pass"));
        let report: QualityReport =
            serde_json::from_value(output.metadata.unwrap()["quality_report"].clone()).unwrap();
        assert_eq!(report.verdict, QualityVerdict::Pass);
        assert_eq!(report.subject.file_scope, vec!["src/a.ts"]);
        assert!(report.subject.complete_changed_scope);
        assert_eq!(report.standards[0].threshold, Some(3));
        assert!(
            report
                .evidence
                .iter()
                .all(|evidence| !evidence.summary.contains("export function"))
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn public_quality_check_ties_violation_to_source_span_and_command() {
        let fixture = fixture_repo(
            r#"[{"code":"eslint(complexity)","message":"complexity 5 exceeds max 3","filename":"src/a.ts","line":1}]"#,
        );
        let output = QualityTool::new()
            .execute(json!({"action": "check"}), test_ctx(fixture.path()))
            .await
            .unwrap();
        let report: QualityReport =
            serde_json::from_value(output.metadata.unwrap()["quality_report"].clone()).unwrap();
        assert_eq!(report.verdict, QualityVerdict::Fail);
        let finding = report
            .observations
            .iter()
            .find(|observation| {
                observation.classification == ObservationClassification::CandidateThresholdViolation
            })
            .unwrap();
        assert_eq!(finding.path.as_deref(), Some("src/a.ts"));
        assert_eq!(finding.line, Some(1));
        assert!(finding.evidence_ids.contains(&"command-0".to_string()));
        assert!(
            report
                .standards
                .iter()
                .any(|standard| standard.source.locator == ".oxlintrc.json")
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn partial_scope_is_explicitly_not_gate_eligible() {
        let fixture = fixture_repo("[]");
        fs::write(fixture.path().join("src/b.ts"), "export const b = 1;\n").unwrap();
        let output = QualityTool::new()
            .execute(
                json!({"action": "check", "files": ["src/a.ts"]}),
                test_ctx(fixture.path()),
            )
            .await
            .unwrap();
        let report: QualityReport =
            serde_json::from_value(output.metadata.unwrap()["quality_report"].clone()).unwrap();
        assert!(!report.subject.complete_changed_scope);
        assert!(!report.subject.gate_eligible);
        assert_eq!(report.verdict, QualityVerdict::PassWithWarnings);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn missing_oxlint_is_unknown_not_pass() {
        let fixture = fixture_repo("[]");
        fs::remove_file(fixture.path().join("node_modules/.bin/oxlint")).unwrap();

        let output = QualityTool::new()
            .execute(json!({"action": "check"}), test_ctx(fixture.path()))
            .await
            .unwrap();
        let report: QualityReport =
            serde_json::from_value(output.metadata.unwrap()["quality_report"].clone()).unwrap();

        assert_eq!(report.verdict, QualityVerdict::Incomplete);
        assert!(report.observations.iter().any(|observation| {
            observation.result == ObservationResult::Unknown
                && observation.classification == ObservationClassification::ToolMissing
        }));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn absent_complexity_rule_is_unknown_not_pass() {
        let fixture = fixture_repo("[]");
        fs::write(fixture.path().join(".oxlintrc.json"), r#"{"rules":{}}"#).unwrap();
        git(fixture.path(), &["add", ".oxlintrc.json"]);
        git(
            fixture.path(),
            &["commit", "-qm", "remove complexity policy"],
        );
        fs::write(
            fixture.path().join("src/a.ts"),
            "export function changed() { return 2; }\n",
        )
        .unwrap();

        let output = QualityTool::new()
            .execute(json!({"action": "check"}), test_ctx(fixture.path()))
            .await
            .unwrap();
        let report: QualityReport =
            serde_json::from_value(output.metadata.unwrap()["quality_report"].clone()).unwrap();

        assert_eq!(report.verdict, QualityVerdict::Incomplete);
        assert!(report.observations.iter().any(|observation| {
            observation.classification == ObservationClassification::RuleNotConfigured
        }));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn candidate_cannot_weaken_repository_complexity_policy() {
        let fixture = fixture_repo("[]");
        fs::write(
            fixture.path().join(".oxlintrc.json"),
            r#"{"rules":{"eslint/complexity":["error",10]}}"#,
        )
        .unwrap();

        let output = QualityTool::new()
            .execute(json!({"action": "check"}), test_ctx(fixture.path()))
            .await
            .unwrap();
        let report: QualityReport =
            serde_json::from_value(output.metadata.unwrap()["quality_report"].clone()).unwrap();

        assert_eq!(report.verdict, QualityVerdict::Fail);
        assert!(report.observations.iter().any(|observation| {
            observation.result == ObservationResult::Fail
                && observation.classification == ObservationClassification::PolicyChanged
        }));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn incomplete_anti_slop_accounting_remains_advisory_and_visible() {
        let fixture = fixture_repo("[]");
        let output = QualityTool::new()
            .execute(
                json!({
                    "action": "check",
                    "anti_slop": {
                        "removed_before_added": "",
                        "reused_existing_mechanism": "",
                        "new_abstractions": [],
                        "retained_legacy_paths": [],
                        "new_configuration": [],
                        "simplification_considered": "",
                        "unresolved_complexity": []
                    }
                }),
                test_ctx(fixture.path()),
            )
            .await
            .unwrap();
        let report: QualityReport =
            serde_json::from_value(output.metadata.unwrap()["quality_report"].clone()).unwrap();

        assert_eq!(report.verdict, QualityVerdict::PassWithWarnings);
        assert_eq!(report.anti_slop_complete, Some(false));
        assert!(report.observations.iter().any(|observation| {
            observation.classification == ObservationClassification::DeclarationIncomplete
        }));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn candidate_change_during_analysis_discards_the_report() {
        use std::os::unix::fs::PermissionsExt;

        let fixture = fixture_repo("[]");
        let bin = fixture.path().join("node_modules/.bin/oxlint");
        fs::write(
            &bin,
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo 'oxlint 1.0.0-test'; exit 0; fi\nprintf 'export const mutated = true;\\n' >> src/a.ts\nprintf '[]\\n'\n",
        )
        .unwrap();
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();

        let output = QualityTool::new()
            .execute(json!({"action": "check"}), test_ctx(fixture.path()))
            .await
            .unwrap();
        let report: QualityReport =
            serde_json::from_value(output.metadata.unwrap()["quality_report"].clone()).unwrap();

        assert_eq!(report.verdict, QualityVerdict::Incomplete);
        assert!(report.observations.iter().any(|observation| {
            observation.classification == ObservationClassification::CandidateChanged
        }));
    }
}
