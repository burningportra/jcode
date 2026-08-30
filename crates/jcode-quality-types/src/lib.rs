use serde::{Deserialize, Serialize};

pub const QUALITY_REPORT_SCHEMA_VERSION: u32 = 1;
pub const COMPLEXITY_STANDARD_ID: &str = "jcode.maintainability.cyclomatic-complexity@1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    RepositoryToolConfig,
    CuratedProfile,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StandardSource {
    pub kind: SourceKind,
    pub locator: String,
    pub version: Option<String>,
    pub digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedStandard {
    pub id: String,
    pub analyzer: String,
    pub rule: String,
    pub threshold: Option<u64>,
    pub required: bool,
    pub source: StandardSource,
    pub trusted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    CommandResult,
    FileSpan,
    GitObject,
    AnalyzerFinding,
    ExternalStandard,
    Declaration,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRef {
    pub id: String,
    pub kind: EvidenceKind,
    pub locator: String,
    pub summary: String,
    pub digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationResult {
    Pass,
    Warn,
    Fail,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationClassification {
    CandidateThresholdClear,
    CandidateThresholdViolation,
    RuleNotConfigured,
    ToolMissing,
    AnalyzerFailed,
    PolicyChanged,
    CandidateChanged,
    DeclarationIncomplete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StandardObservation {
    pub standard_id: String,
    pub result: ObservationResult,
    pub classification: ObservationClassification,
    pub message: String,
    pub evidence_ids: Vec<String>,
    pub path: Option<String>,
    pub line: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewAbstraction {
    pub name: String,
    pub reason: String,
    pub location: String,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetainedPath {
    pub path: String,
    pub reason: String,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewConfiguration {
    pub key: String,
    pub reason: String,
    pub location: String,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AntiSlopDeclaration {
    pub removed_before_added: String,
    pub reused_existing_mechanism: String,
    #[serde(default)]
    pub new_abstractions: Vec<NewAbstraction>,
    #[serde(default)]
    pub retained_legacy_paths: Vec<RetainedPath>,
    #[serde(default)]
    pub new_configuration: Vec<NewConfiguration>,
    pub simplification_considered: String,
    #[serde(default)]
    pub unresolved_complexity: Vec<String>,
}

impl AntiSlopDeclaration {
    pub fn completeness_issues(&self) -> Vec<String> {
        let mut issues = Vec::new();
        for (name, value) in [
            ("removed_before_added", self.removed_before_added.as_str()),
            (
                "reused_existing_mechanism",
                self.reused_existing_mechanism.as_str(),
            ),
            (
                "simplification_considered",
                self.simplification_considered.as_str(),
            ),
        ] {
            if value.trim().is_empty() {
                issues.push(format!("{name} is empty"));
            }
        }
        for abstraction in &self.new_abstractions {
            if abstraction.name.trim().is_empty()
                || abstraction.reason.trim().is_empty()
                || abstraction.location.trim().is_empty()
                || abstraction.evidence.is_empty()
            {
                issues.push("new abstraction lacks name, reason, location, or evidence".into());
            }
        }
        for retained in &self.retained_legacy_paths {
            if retained.path.trim().is_empty()
                || retained.reason.trim().is_empty()
                || retained.evidence.is_empty()
            {
                issues.push("retained legacy path lacks path, reason, or evidence".into());
            }
        }
        for config in &self.new_configuration {
            if config.key.trim().is_empty()
                || config.reason.trim().is_empty()
                || config.location.trim().is_empty()
                || config.evidence.is_empty()
            {
                issues.push("new configuration lacks key, reason, location, or evidence".into());
            }
        }
        issues
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QualitySubject {
    pub file_scope: Vec<String>,
    pub complete_changed_scope: bool,
    pub gate_eligible: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateFingerprint {
    pub repository_identity_digest: String,
    pub baseline_commit: Option<String>,
    pub candidate_head: Option<String>,
    pub candidate_tree: Option<String>,
    pub changed_manifest_digest: String,
    pub analyzed_scope_digest: String,
    pub policy_digest: String,
    pub analyzers_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QualityVerdict {
    Pass,
    PassWithWarnings,
    Fail,
    Incomplete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QualityReport {
    pub id: String,
    pub schema_version: u32,
    pub repository_root: String,
    pub subject: QualitySubject,
    pub baseline_commit: Option<String>,
    pub candidate: CandidateFingerprint,
    pub profile_ids: Vec<String>,
    pub standards: Vec<ResolvedStandard>,
    pub observations: Vec<StandardObservation>,
    pub evidence: Vec<EvidenceRef>,
    pub anti_slop: Option<AntiSlopDeclaration>,
    pub anti_slop_complete: Option<bool>,
    pub verdict: QualityVerdict,
    pub created_at_unix_secs: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quality_report_round_trips() {
        let report = QualityReport {
            id: "quality-1".into(),
            schema_version: QUALITY_REPORT_SCHEMA_VERSION,
            repository_root: "/repo".into(),
            subject: QualitySubject {
                file_scope: vec!["src/a.ts".into()],
                complete_changed_scope: true,
                gate_eligible: false,
            },
            baseline_commit: Some("abc".into()),
            candidate: CandidateFingerprint {
                repository_identity_digest: "a".into(),
                baseline_commit: Some("abc".into()),
                candidate_head: Some("def".into()),
                candidate_tree: Some("tree".into()),
                changed_manifest_digest: "b".into(),
                analyzed_scope_digest: "c".into(),
                policy_digest: "d".into(),
                analyzers_digest: "e".into(),
            },
            profile_ids: vec!["jcode:maintainable-change@1".into()],
            standards: vec![],
            observations: vec![],
            evidence: vec![],
            anti_slop: None,
            anti_slop_complete: None,
            verdict: QualityVerdict::Pass,
            created_at_unix_secs: 1,
        };
        let encoded = serde_json::to_string(&report).unwrap();
        assert_eq!(
            serde_json::from_str::<QualityReport>(&encoded).unwrap(),
            report
        );
    }

    #[test]
    fn declaration_reports_empty_required_answers() {
        let declaration = AntiSlopDeclaration {
            removed_before_added: String::new(),
            reused_existing_mechanism: "used the existing tool boundary".into(),
            new_abstractions: vec![],
            retained_legacy_paths: vec![],
            new_configuration: vec![],
            simplification_considered: String::new(),
            unresolved_complexity: vec![],
        };
        assert_eq!(declaration.completeness_issues().len(), 2);
    }
}
