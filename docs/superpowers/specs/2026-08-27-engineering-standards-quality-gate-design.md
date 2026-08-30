# Engineering standards and anti-slop quality gate

Status: first report-only slice implemented
Date: 2026-08-27

## Implementation evidence

The first report-only slice is implemented in `jcode-quality-types`,
`jcode-base::quality`, and the public `quality check` tool. Focused public-tool
tests cover a sourced pass, a source-linked threshold violation, missing Oxlint,
an absent complexity rule, candidate policy weakening, partial scope, incomplete
anti-slop accounting, and candidate mutation during analysis. The full quality
tool test module, stable contract tests, formatting, and feature-disabled app-core
build passed on macOS arm64 with Rust 1.98.0.

The server-authoritative todo completion transition remains intentionally out of
scope for this slice, as specified below. Every report therefore sets
`gate_eligible = false`; the tool is evidence-producing and report-only rather
than claiming enforcement that does not yet exist.

## Summary

Jcode should not invent a programming style for the user. It should resolve standards from explicit project rules, established tool configuration, official guidance, and a small curated set of maintainability rules. Every enforced rule must name its source. Model opinion may explain a result, but it cannot create a blocking rule.

This design adds four connected pieces:

1. A standards registry that resolves applicable, sourced rules for a project and change.
2. A complexity governor that initially detects sourced complexity-threshold violations and later supports measured regressions without pretending one metric defines quality.
3. A future anti-slop completion gate that rejects unsupported completion transitions and unexplained complexity increases at the server boundary.
4. An evidence model that links requirements, checks, observations, exceptions, and verdicts.

The first implementation slice is deliberately narrow and report-only. It supports JS and TS projects that already use Oxlint and returns a typed report through one `quality check` tool action. It does not add project-specific registry configuration, persistence, todo completion enforcement, TUI state, a general static-analysis framework, or an AI style judge. Project overrides follow after the report contract proves stable. Completion enforcement follows only after Jcode has a server-owned identity and change epoch for the code owned by one goal.

## Problem

Jcode already tracks completion confidence, feedback-loop relevance, coverage, traceability, delivery state, and typed swarm handoffs. Those checks answer whether an agent gathered evidence and carried work far enough. They do not answer whether the change added needless complexity or followed a defensible engineering standard.

The missing behavior causes two common failures.

First, an agent can pass tests while leaving behind wrapper layers, duplicated paths, speculative configuration, or branch-heavy code. Second, a quality agent can object based on taste without naming a rule, source, baseline, or observed regression. Neither outcome is acceptable.

A useful quality system must separate four things:

- the rule
- the authority behind the rule
- the observation made on this change
- the policy that turns observations into a verdict

Without that separation, "quality" becomes another prompt.

## Goals

- Resolve project standards from explicit, versioned, inspectable sources.
- Prefer proven external standards over inferred personal style.
- Make reduced complexity and lower reader load default maintainability goals.
- Compare changed code with its baseline so old debt does not block unrelated work.
- Prevent metric gaming, especially helper extraction that lowers one score while adding indirection.
- Require evidence before a code-change goal is marked complete.
- Extend the todo system with a server-authoritative completion transition for gated code-change goals.
- Keep stable data contracts outside the TUI and provider layers.
- Allow language-specific analyzers without coupling the registry to one tool.
- Make exceptions explicit, sourced, scoped, and temporary where possible.

## Non-goals

- Defining one universal style guide.
- Copying a user's incidental coding habits.
- Automatically rewriting code until a metric turns green.
- Treating cyclomatic complexity as a complete measure of maintainability.
- Blocking a repository because of unrelated pre-existing debt.
- Fetching standards from the web during normal agent execution.
- Ranking individual programmers or treating popularity as authority.
- Replacing Clippy, Oxlint, Ruff, repository CI, or human review.
- Building the final TUI quality dashboard in the first slice.
- Applying the completion gate to prose-only, research-only, or administrative goals.

## Design principles

### Every blocking standard has provenance

A model can propose a standard, but the proposal stays advisory until a human or a versioned Jcode profile accepts it. A blocking standard must point to project configuration, existing tool configuration, official documentation, or a reviewed built-in profile.

### Changed code pays for new complexity

The governor evaluates the changed scope and compares it with a baseline. Existing violations remain visible but do not fail unrelated work. A change fails when it adds a new hard violation, worsens a protected metric beyond budget, or claims a required check passed when no observation exists.

### Metrics are evidence, not design instructions

The governor reports branch count, function size, nesting, file growth, and analyzer output. It does not prescribe "extract a helper" or any other mechanical fix. Splitting one readable function into six wrappers may lower a local score and still increase total reader load.

### Unknown is not pass

If a required analyzer is unavailable, the observation is `unknown`. Policy decides whether `unknown` blocks. The default maintainability profile blocks unknown required checks for code-change completion.

### Exceptions are quality debt

A waiver needs a reason, scope, owner, source, and optional expiry. The report preserves it as debt. An agent cannot silently downgrade a failure to a warning.

## System placement

The design follows Jcode's layered workspace rules.

### `jcode-quality-types`

A new dependency-light crate owns stable serialized contracts:

- standards and sources
- resolved profiles
- evidence references
- observations and metric deltas
- exceptions
- anti-slop declarations
- quality reports and verdicts

Dependencies should be limited to `serde`, `serde_json`, and small sibling type crates if later required. It must not access the filesystem, execute commands, know about the TUI, or depend on providers.

### `jcode-base::quality`

Filesystem-backed domain behavior lives in a focused module or later a dedicated runtime crate when the boundary is proven. It owns:

- standards registry resolution
- repository and analyzer configuration discovery
- baseline and candidate fingerprint calculation
- report assembly
- later report persistence and lookup

It does not execute external analyzers. The first slice should remain in `jcode-base` rather than creating a runtime crate before more than one caller exists.

### `jcode-app-core::tool::quality`

The first slice exposes one `check` action. Its result includes the resolved standards, so a separate `resolve` action is not needed yet. The app-core tool owns analyzer process execution, timeouts, cancellation, output limits, and conversion into domain observations. A later slice adds `show` after reports have durable storage and goal bindings.

The tool returns concise human-readable output plus a bounded typed report in `ToolOutput.metadata`. Raw analyzer output stays in the underlying command evidence and is not duplicated into metadata.

### Server-authoritative completion transition

The existing todo evaluator and TUI auto-poke behavior are advisory follow-through, not a hard gate: completion can be persisted before a later continuation message runs. The quality gate must therefore be a new server-authoritative transition rule, not another TUI check.

When a client requests a gated code-change goal transition to `completed`, the session server validates the current binding and report before persistence. A failed validation rejects the transition with a typed reason, leaving the goal and its todos in their prior non-complete state. Local tools and remote clients use the same server path. The current post-turn continuation remains defense in depth, not the authority.

This integration is not part of the first slice. It requires a stable goal identity, server-owned change scope, and a change epoch. A global worktree diff or mutable todo group label is not sufficient in a repository with concurrent agents.

### Swarm integration

The first slice does not change the swarm DAG. A later phase adds `quality_report_id` to `HandoffArtifact` or a typed evidence attachment. Deep-mode implementation gates can then require a passing report before closing.

## Standards registry

### Source hierarchy

The registry resolves standards in this order:

1. Explicit project configuration in `.jcode/standards.toml`, beginning in the project-override slice after the first report-only slice.
2. Existing repository tool configuration, such as `oxlint.config.ts` or `.oxlintrc.json`.
3. Official language or framework guidance stored in a reviewed Jcode profile.
4. Jcode's curated maintainability profile.
5. Inferred repository convention, advisory only.
6. Model recommendation, advisory only.

A blocking rule's authority is separate from its location and precedence. Candidate code can propose a project override, but it cannot make that override trusted merely by editing a higher-ranked file. Weakening, disabling, waiving, changing the comparison baseline, or raising a limit requires a user-approved policy decision stored in server-owned application state outside the candidate repository diff. Tightening may come directly from reviewed project configuration. Reports preserve both the proposed repository policy and the effective trusted policy.

### Standard identity

A standard has a stable namespaced ID and version. Examples:

- `jcode.maintainability.cyclomatic-complexity@1`
- `jcode.maintainability.function-size@1`
- `oxlint.eslint.complexity@1`
- `project.no-duplicate-provider-routes@1`

Renaming a standard creates an alias or a new version. Reports keep the resolved ID and version so old evidence remains interpretable.

### Core contracts

```rust
pub struct StandardDefinition {
    pub id: StandardId,
    pub title: String,
    pub rationale: String,
    pub source: StandardSource,
    pub applies_to: Applicability,
    pub check: CheckSpec,
    pub default_severity: Severity,
    pub tags: Vec<String>,
}

pub struct StandardSource {
    pub kind: SourceKind,
    pub locator: String,
    pub version: Option<String>,
    pub digest: Option<String>,
    pub reviewed_at_unix_secs: Option<u64>,
}

pub enum SourceKind {
    ProjectConfig,
    RepositoryToolConfig,
    OfficialGuide,
    CuratedProfile,
    InferredConvention,
    ModelSuggestion,
}

pub enum Severity {
    Advisory,
    Warning,
    Error,
}

pub enum CheckSpec {
    AnalyzerRule {
        adapter: String,
        rule: String,
        options: serde_json::Value,
    },
    MetricBudget(MetricBudget),
    Declaration(DeclarationKind),
}
```

A resolved standard preserves an ordered provenance chain rather than one winning source. Each step records the field it changed, its previous and resulting value, source digest, trust state, and resolution effect. The resolver has explicit effects for `define`, `tighten`, `weaken-requested`, `disable-requested`, `waive`, and `reject-conflict`. This makes a built-in definition, analyzer configuration, project proposal, and server-approved exception independently auditable.

### Project configuration

```toml
version = 1
extends = ["jcode:maintainable-change@1"]

[quality]
required_for_code_changes = true
unknown_required_check = "fail"
comparison = "merge-base"

[standards."oxlint.eslint.complexity@1"]
severity = "error"
max = 15

[[exceptions]]
standard = "oxlint.eslint.complexity@1"
path = "src/parser/legacy.ts"
reason = "Parser state machine mirrors the wire grammar. Splitting it obscures transitions."
owner = "project"
expires = "2026-12-31"
evidence = ["docs/adr/014-parser-state-machine.md"]
```

The loader rejects unknown top-level versions, duplicate standard IDs, invalid severities, expired exceptions, and exceptions without reasons.

### Built-in profile

The future built-in profile is `jcode:maintainable-change@1`. It should stay small.

It contains:

- no new analyzer-reported complexity threshold errors in changed code
- required reconciliation between derived change facts and the anti-slop declaration
- required evidence for each blocking observation
- no required check reported as pass without an observation

It should not contain broad formatting preferences, naming taste, or framework-specific advice.

## Complexity governor

### What it measures

The complete design supports several independent signals:

- cyclomatic complexity
- cognitive complexity when an analyzer provides it
- nesting depth
- branch count
- function size
- file growth
- dependency fan-out
- number of new public types or configuration switches
- call-path indirection added by extraction
- duplicated implementation paths
- retained legacy paths

The first slice implements only analyzer-reported cyclomatic-complexity threshold violations for JS and TS through Oxlint. It also records changed-line counts from Git for context, but changed-line count is not a pass or fail metric. It checks the complete set of changed JS and TS files discovered from the candidate manifest. A caller may request a narrower diagnostic scope, but such a report is explicitly partial and can never satisfy a future completion gate.

### Baseline and classification

The default baseline is the merge base with the configured upstream branch. If no upstream exists, use candidate-start `HEAD` recorded before analysis. The report records the exact baseline commit and candidate HEAD or tree identity.

The Oxlint-only first slice does not claim continuous complexity deltas. A threshold diagnostic can establish only:

- `candidate_threshold_violation`
- `candidate_threshold_clear`
- `unknown`

A later AST-aware adapter may add stable function identities, before and after metric values, and classifications such as `worsened`, `improved`, and `unchanged_existing_debt`. Until then, Jcode must not describe a below-threshold increase as a measured regression.

### Scope

The governor checks changed JS and TS files, including untracked files selected by Git status. Deleted files do not require analysis. Generated files and vendored paths are excluded only through explicit project patterns or analyzer configuration.

### Oxlint adapter

The first adapter:

1. Resolves an already-installed `oxlint` executable from a repository dependency or `PATH` without invoking a package manager, downloading code, or running install scripts.
2. Detects existing Oxlint configuration.
3. Requires an effective, enabled `eslint/complexity` threshold from repository configuration. If the rule is absent or disabled, it reports `rule_not_configured`, not pass. Jcode does not invent a threshold.
4. Runs Oxlint against the complete changed JS and TS file set using machine-readable output.
5. Captures command, exit status, tool version, configuration path and digest, rule ID, effective threshold when observable, file span, and message.
6. Returns `unknown` if the executable is unavailable, the effective rule cannot be established, or output cannot be parsed.

The adapter must not install Oxlint. Setup remains an explicit user or agent action.

### Preventing metric gaming

The report includes the anti-slop declaration and changed-line context beside analyzer metrics. Later adapters may measure indirection and fan-out, but the first slice uses a simpler rule: a passing complexity score never proves the change is simple. It only proves that this analyzer found no blocking cyclomatic-complexity regression.

The first-slice summary uses coverage-specific language such as `Oxlint complexity: pass`. It must never render `quality passed` when only one analyzer ran.

## Anti-slop completion gate

### Structured declaration

Every gated code-change report includes:

```rust
pub struct AntiSlopDeclaration {
    pub removed_before_added: String,
    pub reused_existing_mechanism: String,
    pub new_abstractions: Vec<NewAbstraction>,
    pub retained_legacy_paths: Vec<RetainedPath>,
    pub new_configuration: Vec<NewConfiguration>,
    pub simplification_considered: String,
    pub unresolved_complexity: Vec<String>,
}
```

Empty free-text answers are invalid. Empty lists require an explicit `none` or `not_applicable` reason. Each new abstraction, retained path, or configuration item needs a reason, source location, and evidence reference.

Self-attestation alone is advisory and cannot clear a gate. Before completion enforcement ships, Jcode must derive a `ChangedFact` set from the owned diff, including at minimum new files, public types, configuration keys, and retained compatibility paths that deterministic analysis can identify. The gate reconciles those facts with declaration entries. Unmatched derived facts, unsupported `none` claims, and filler-only answers make the declaration incomplete. Facts Jcode cannot derive remain visible as self-attestation, not proof.

### Gate rules

The gate is evaluated as part of the server-owned todo transition described above. It must not persist a newly completed state and then ask the model to undo it. Existing confidence, delivery, relevance, coverage, and traceability checks remain independent conditions on the same transition.

The transition is accepted only when:

- the report is bound to the same goal and owned change scope
- a report exists for the current change fingerprint
- the report resolves at least one project or built-in profile
- every required standard has an observation
- no required observation is `fail` or policy-blocking `unknown`
- every exception is valid and unexpired
- the anti-slop declaration reconciles with derived change facts
- report evidence references still resolve
- the report fingerprint matches the current baseline and candidate state

A stale report never passes. New edits inside the bound scope invalidate it. Edits owned by another goal do not invalidate it.

The gate must not infer ownership from the aggregate worktree. Before this integration ships, Jcode needs a server-owned stable goal identity and `GoalQualityBinding`. A mutable todo group label is display text, not identity. If Jcode cannot establish the binding, the report remains advisory and the gate stays off.

### Continuation message

When the gate blocks completion, Jcode should name the failed category without dumping internal scoring:

```text
[auto] The code-change quality gate is incomplete. Run the quality check for the current diff, address blocking complexity regressions, and account for new abstractions, configuration, and retained legacy paths. Update the todo after the report matches the current change.
```

The user-facing summary should be shorter:

```text
Quality evidence is stale or incomplete.
```

### Binding a report to a goal

The completion gate must operate on owned change scope, not every dirty file in the repository. Concurrent agents may share a repository, and unrelated work must not block or falsely satisfy another goal.

The gate integration therefore requires a server-owned binding:

```rust
pub struct GoalQualityBinding {
    pub id: GoalQualityBindingId,
    pub goal_id: GoalId,
    pub owner: QualityOwner,
    pub baseline_commit: Option<String>,
    pub owned_paths: Vec<String>,
    pub change_epoch: u64,
    pub current_report_id: Option<QualityReportId>,
}

pub enum QualityOwner {
    Session(String),
    Lane(String),
}
```

The server creates the stable `goal_id` and binding. Changing owned paths, baseline, or an owned file increments `change_epoch` and clears `current_report_id` atomically. Renaming a todo group does not create a new identity or reset the epoch. A report can satisfy completion only when its binding ID and epoch equal the current server state.

A documentation-only or research goal has no binding and does not invoke the code-change gate. A future mission contract may declare work kind directly.

## Evidence model

### Requirements

Evidence must answer a named requirement or standard. A report cannot attach one generic test command to every item without showing what it establishes.

```rust
pub struct EvidenceRef {
    pub id: EvidenceId,
    pub kind: EvidenceKind,
    pub locator: String,
    pub summary: String,
    pub digest: Option<String>,
    pub observed_at_unix_secs: u64,
}

pub enum EvidenceKind {
    ToolCall,
    CommandResult,
    FileSpan,
    GitObject,
    AnalyzerFinding,
    TestResult,
    Benchmark,
    Review,
    ExternalStandard,
}

pub struct StandardObservation {
    pub standard_id: StandardId,
    pub result: ObservationResult,
    pub classification: ObservationClassification,
    pub message: String,
    pub evidence_ids: Vec<EvidenceId>,
}

pub enum ObservationClassification {
    CandidateThresholdClear,
    CandidateThresholdViolation,
    ExistingDebt,
    Improved,
    Worsened,
    Unknown,
}

pub enum ObservationResult {
    Pass,
    Warn,
    Fail,
    Unknown,
}
```

### Evidence locators

The first slice supports:

- `tool://<session-id>/<tool-call-id>`
- `file://<repo-relative-path>#L<start>-L<end>`
- `git://<object-id>`
- `command://<report-id>/<command-index>`
- `url://<public-url>`

Reports store summaries and digests, not unlimited raw command output. Existing session tool records remain the source for full output.

### Quality report

```rust
pub struct QualitySubject {
    pub binding_id: Option<GoalQualityBindingId>,
    pub goal_id: Option<GoalId>,
    pub owner: Option<QualityOwner>,
    pub change_epoch: Option<u64>,
    pub file_scope: Vec<String>,
}

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

pub struct QualityReport {
    pub id: QualityReportId,
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
    pub exceptions: Vec<AppliedException>,
    pub verdict: QualityVerdict,
    pub created_at_unix_secs: u64,
}

pub enum QualityVerdict {
    Pass,
    PassWithWarnings,
    Fail,
    Incomplete,
}
```

`QualitySubject` records the explicit file scope used by the check. Later gate-enabled reports also carry the stable binding ID, goal ID, owner, and change epoch supplied by the server. Report-only checks leave those fields empty and cannot satisfy a completion transition.

`PassWithWarnings` clears a later completion gate only when all warnings are advisory. Required warnings must be waived explicitly.

### Candidate fingerprint

The candidate fingerprint is stored in `QualityReport.candidate`. It covers:

- repository root identity
- baseline commit
- candidate `HEAD` and tree object IDs
- full changed-file manifest, including staged, unstaged, committed-since-baseline, and untracked entries
- exact analyzed scope and excluded paths
- content digests for analyzed untracked files
- ordered effective-policy provenance and digests
- analyzer executable identity, version, controlled environment digest, and relevant configuration digests

Changing code, scope, policy, analyzer identity or configuration, candidate tree, or baseline invalidates the report. Only a report over the binding's complete owned changed-file manifest may later satisfy completion.

## Tool behavior

### `quality check`

Inputs:

- optional anti-slop declaration for advisory accounting in the first slice
- optional diagnostic file scope; omitted means all changed JS and TS files

A baseline override, policy weakening, exception, or disabled check is not accepted from the agent tool. Those changes require a later user-approved server policy path.

Behavior:

1. Resolve the effective repository rule and provenance.
2. Build the full candidate manifest and requested scope.
3. Run the supported adapter through app-core's supervised process boundary.
4. Build bounded evidence references.
5. Record declaration completeness as advisory evidence.
6. Apply verdict policy.
7. Return a concise summary with typed metadata.

### `quality show`

A later persistence slice adds `quality show`. It shows the latest report or a report by ID and marks stale reports clearly.

## Persistence

The report-only first slice does not persist reports. `quality check` returns a bounded typed report in live `ToolOutput.metadata`. Normal session history does not currently preserve arbitrary tool metadata, so this output is not durable and cannot support `/quality`, remote status, or completion enforcement.

The persistence slice adds a server-owned quality sidecar in Jcode's application data directory, scoped by repository identity and stable goal binding. It adds a protocol `QualitySnapshot` to history and update events plus a server request used by remote `/quality`. Writes are atomic and schema-versioned. It should keep a bounded history, initially the latest 20 reports per repository. Reports referenced by a persisted goal binding are exempt from pruning until the reference disappears.

Generated reports must not be written into the user's repository by default. Report JSON should not be embedded directly in `TodoGoal`.

## TUI behavior

The report-only first slice adds no persistent TUI state. The existing tool-result renderer shows the concise verdict, and the full report remains available in tool metadata.

A later gate integration adds:

- a one-line coverage-specific state, such as `Oxlint complexity: pass`, `Required JS/TS checks: blocked`, or `Quality evidence: stale`
- `/quality` output that opens the latest persisted report in the side panel

The side panel groups content in this order:

1. verdict and scope
2. blocking findings
3. anti-slop declaration
4. warnings and exceptions
5. evidence
6. resolved standards and sources

A later design can add mission-level quality history and repository complexity maps.

## Failure handling

- Missing analyzer: record `unknown`; do not claim pass.
- Analyzer crash: capture stderr summary and exit status as evidence; verdict becomes incomplete.
- Invalid project config, once project overrides exist: fail closed with an actionable path and field error.
- Baseline unavailable: fall back to pre-change `HEAD` when possible; otherwise mark baseline comparison unknown.
- Dirty changes during analysis: recompute the candidate fingerprint after checks and discard the report if it changed.
- Expired exception: treat it as absent and report the expiry.
- Evidence target missing: mark the report stale or incomplete.
- Unsupported language: resolve declaration-only standards, report analyzer coverage honestly, and follow project unknown-check policy.

## Privacy and security

- Do not send source code or reports to a model to calculate deterministic metrics.
- Do not fetch public standards during a check.
- Persist public URLs and local locators, never credentials or private remote URLs.
- Redact command output using the same secret handling rules as tool results.
- Execute analyzers only through a supervised app-core process boundary with timeout, cancellation, bounded output, a controlled environment, no-download executable resolution, secret redaction, and existing command-risk handling.

## Smallest implementation slice

The first slice proves that Jcode can resolve one sourced standard, execute one deterministic analyzer, and return anti-slop accounting through one public tool action without changing completion behavior.

### Scope

1. Add `jcode-quality-types` with only the contracts used by the first report: source, resolved standard, observation, bounded evidence summary, declaration, fingerprint, and verdict.
2. Add one built-in standard, `jcode.maintainability.cyclomatic-complexity@1`, mapped to an existing repository Oxlint `eslint/complexity` rule.
3. Discover the repository's existing Oxlint configuration. Do not add `.jcode/standards.toml` yet.
4. Implement the Oxlint adapter for the complete changed JS and TS file manifest.
5. Add one `quality` tool with a `check` action. The report includes the resolved standard and its source.
6. Return the bounded typed report through `ToolOutput.metadata`.
7. Accept an optional anti-slop declaration and report its completeness as advisory evidence. It does not affect the first-slice analyzer verdict.

### Explicit exclusions

- project overrides, `.jcode/standards.toml`, and exceptions
- multi-rule built-in profiles
- durable report persistence and `quality show`
- todo completion gating
- goal or lane ownership bindings
- info-widget and side-panel integration
- Rust, Python, Go, and other analyzer adapters
- model-based code review
- automatic refactoring
- swarm gate integration
- remote registry updates
- inferred repository conventions
- mission-level quality history
- organizational policy distribution
- a general plugin API for analyzers

### Acceptance behavior

The slice is complete when these workflows pass:

1. A JS project with an enabled, configured Oxlint complexity maximum receives `Oxlint complexity: pass` for a simple changed function.
2. An over-threshold changed function produces a blocking observation tied to the Oxlint rule, file span, command result, tool version, effective threshold when observable, and config source.
3. Every changed JS and TS file appears in the candidate manifest and analyzed scope. A narrower requested diagnostic scope is marked partial and non-gate-eligible.
4. Missing Oxlint produces `unknown`, not pass.
5. An absent or disabled `eslint/complexity` rule produces `rule_not_configured`, not pass and not `tool_missing`.
6. An incomplete anti-slop declaration is visible as advisory evidence and does not become proof of simplicity.
7. The report metadata round-trips without parsing human-readable output and excludes raw analyzer output.
8. A code, candidate tree, scope, or discovered analyzer-config edit during analysis invalidates the candidate fingerprint and discards the result.
9. A candidate edit that weakens or disables the effective rule is reported as an untrusted policy change and cannot manufacture a pass.


### Next slices: trusted policy, then completion enforcement

The next design and implementation slice adds project overrides and exceptions in `.jcode/standards.toml`. The following slice adds durable reports, stable goal IDs, `GoalQualityBinding` with change epochs, `quality show`, server-authoritative todo transition enforcement, remote protocol state, and small TUI views. It must prove that concurrent work in the same repository cannot block or satisfy the wrong goal.

## Testing strategy

### Contract tests

- serde round trips for every quality type
- unknown-field and version behavior
- stable standard IDs and aliases
- candidate fingerprint determinism

### Registry tests

The first slice tests repository-rule discovery, ordered provenance, missing or disabled rule behavior, and rejection of candidate policy weakening. Hierarchy, project override, exception, expiry, and duplicate-ID tests begin with the `.jcode/standards.toml` slice.

### Adapter tests

Use fixture repositories and a controlled fake Oxlint executable for parsing, command construction, crashes, missing tools, versions, config locations, and changed-file selection. Add one real Oxlint acceptance test when the binary is available in CI.

### Gate tests

These belong to the persistence and completion-enforcement slice:

- current passing report clears only the new quality condition
- fail, incomplete, unknown, and stale reports block
- edits inside owned scope invalidate reports
- unrelated concurrent edits do not invalidate or satisfy another goal
- non-code goals are unaffected
- existing todo continuation behavior remains intact
- local and remote session behavior match

### Runtime acceptance test

The first slice drives the `quality` tool through Jcode's public interface in a fixture JS repository:

1. introduce a complexity violation
2. run `quality check` over the complete changed JS and TS manifest, optionally with an advisory declaration
3. inspect the resolved source, threshold-specific observation, bounded evidence summaries, advisory declaration status, and typed report metadata
4. simplify the function
5. rerun the check and observe the changed verdict

The next slice adds todo completion, reload, concurrent-agent ownership, and persistence to this workflow.

## Rollout

The report-only tool starts behind a typed experimental quality setting in Jcode's normal configuration contract. It logs no telemetry beyond existing privacy rules and does not change completion behavior.

After real-project use stabilizes registry resolution and the report schema:

1. add project overrides and exceptions after the report schema survives real-project use
2. design and implement stable goal IDs, goal-owned change bindings with epochs, persistence, and server-authoritative completion transitions
3. enable the built-in profile by default for new projects with detected Oxlint configuration only after the gate slice passes its concurrency acceptance tests
4. add Rust and Python adapters through separate reviewed designs
5. connect deep swarm implementation gates
6. add complexity trend views only after reports provide enough reliable history

## Success criteria

- Every blocking quality result names a standard, source, observation, and evidence reference.
- No required check can silently become pass when its analyzer is missing.
- Unrelated legacy debt does not block changed work.
- A stale report cannot clear completion.
- The first slice adds one stable type crate, one filesystem-backed domain module, and one app-core tool without persistence, TUI state, completion coupling, project policy syntax, or a general plugin framework.
- Agents account for abstractions, configuration, and legacy paths before claiming completion.
- The system describes analyzer coverage precisely and never equates one metric with overall code quality.

## Deferred decisions

These are intentionally deferred to later designs because the first slice does not need them:

- the shared analyzer plugin ABI
- organization-managed standards distribution
- cryptographic signing of curated profiles
- automatic source refresh and review workflow
- cross-language complexity normalization
- model-assisted detection of conceptual duplication
- quality trend retention beyond bounded local reports
