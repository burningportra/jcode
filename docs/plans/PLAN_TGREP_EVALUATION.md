# tgrep fit assessment

Historical preliminary assessment. The user subsequently selected both surfaces and delegated lifecycle decisions. See the [executed spike and adoption decisions](../reviews/2026-09-08-tgrep-spike.md), which also accounts for the existing FFF backend.

## Elicitation and bounded scope

The user proposed https://github.com/microsoft/tgrep after delivery of @ file selection. Whether they intend filename completion or agent content search is unanswered. Proceed under the lowest-risk interpretation: evaluate both and recommend a next step, without installing software, starting an index/server, or changing production search. This is a compressed single-session goal, not an approved integration project.

Done means a source-grounded recommendation, explicit uncertainties and an executable adoption gate. No performance claim is a local measurement.

## Seed and foundation

Read upstream README on 2026-09-08 and inspected Jcode's current implementation. AGENTS.md exists and requires staying on this branch. Jcode is Rust, as is tgrep, but shared language alone does not establish API stability. Memory search returned no relevant prior tgrep decision.

Jcode's `crates/jcode-app-core/src/tool/agentgrep.rs` delegates search to ripgrep; its structured tool contract also offers modes beyond textual grep. `crates/jcode-tui/src/tui/app/file_mentions.rs` lists paths asynchronously, includes hidden files, disables symlink traversal and applies gitignore rules outside Git repos. Preserve those behaviors unless a separately approved product change says otherwise.

## Recommendation and alternatives

| Approach | Decision | Reason |
|---|---|---|
| Replace @ walker with tgrep now | Do not adopt now | Path completion does not need content trigrams. Index lifecycle and membership compatibility add cost without measured benefit. |
| Replace all agentgrep modes | Reject | Text search does not replace outline/relationship functionality. |
| Optional accelerator for agentgrep textual searches | Best candidate for a follow-up experiment | Repeated selective content searches in large repos are the upstream target. Preserve existing output contract and ripgrep fallback. |
| Keep current implementation | Current decision | No local performance or equivalence evidence yet justifies migration. |

Upstream advertises large warm-index speedups, especially on macOS. These exclude initial index construction and cannot be extrapolated to Jcode without measurement. Upstream now supports `--files`, including a filename-only sidecar, so it would be incorrect to dismiss it as incapable of filename enumeration.

## Contracts and failure risks

- Upstream documents a default 64 MiB file cap versus ripgrep's uncapped default. Never silently introduce that omission into existing search.
- Upstream documents repaired invalid UTF-8 and JSON `lines.text` instead of ripgrep's base64 `lines.bytes`. Matching and offset semantics need fixtures, not a claim of drop-in compatibility.
- Hidden and traversal-changing flags can bypass the index. The @ picker includes hidden files and uses `require_git(false)`, so a compatible request may erase the indexed advantage.
- `--files` local-index results are snapshots. A watcher is not a proof that all changes are indexed. Initial build documentation is internally inconsistent about empty versus partial query results. Pin and inspect a version before deciding readiness semantics.
- Index and server traversal flags must agree. Root, canonical identity, policy and version must form the cache/server identity.
- Missing executable, startup failure, rebuilding, stale/uncertain index, corrupt index, timeout and unsupported options must retain a correct scan path. An empty indexed result must not be treated as authoritative while readiness is uncertain.
- A future server must have explicit ownership, loopback/authentication review, bounded requests/output, shutdown and disk/index privacy policy. No network service is started for this assessment.

## Executable follow-up plan, not implementation authorization

1. Pin an upstream revision and inspect its CLI/library contracts, readiness protocol and transport security. Use integration discovery before actually adopting/installing the external tool. Record license and supported targets.
2. Build isolated fixtures and compare normalized results with Jcode's current ripgrep invocation: literal/regex, case, glob/type, contextual regions, nested roots, hidden/ignored/tracked paths, large/binary/invalid-UTF8 files, unusual names, deletes/renames, immediate edits, and branch switches. Capture stderr and exit statuses as well as match data.
3. Benchmark cold startup, initial build, warm selective/broad searches, short patterns, concurrent clients, mutation-to-visible latency, CPU/RSS and index disk size. Use this repository and a representative larger fixture, with identical inclusion policies. Record median and p95 rather than best runs.
4. Adoption gate: zero unexplained result omissions on supported requests; reliable fallback for every unsupported/uncertain state; proposed warm p95 improvement of at least 2x on the intended large-repo workload; explicit acceptable cold/resource budgets derived from baseline. This threshold is a proposed decision rule, not an observed result.
5. Only if the gate passes, plan an optional textual-search adapter behind the existing agentgrep contract. Do not add a second agent-facing search tool or migrate outline/trace modes. Reuse existing process cancellation/output limits where possible. Keep @ unchanged unless separate filename benchmarks justify it.
6. Verify adapter through real tool requests, not only direct CLI comparisons. Prove current fallback works without tgrep installed. Rollback is disabling the optional backend, with no persistent schema migration.

## Refinement and convergence

Compressed review, single self-draft, no independent or cross-model review was performed. This is sufficient for a non-adoption assessment, not a production integration design.

- Architecture pass: rejected universal replacement; separated path lookup from content search and identified existing tool contract boundary.
- Failure/safety pass: added readiness uncertainty, documented JSON and file membership differences, server ownership and mandatory scan fallback.
- Simplicity pass: removed installation and benchmarking from this ambiguous request's immediate scope. The deliverable is a recommendation and testable next-step gate, not speculative code.

Relevant assessment rubric is satisfied: scope, alternatives, contracts, failure risks, resources, safety, sequencing and verification are explicit. Production data model and exact latency/resource budgets are intentionally deferred until a pinned-source experiment. No numeric self-score substitutes for missing benchmarks. Residual risks: unpinned upstream README, no runtime comparison, unresolved user intent and no independent review.

## Handoff and distillation

No beads: this bounded assessment completes in one session and has no implementation handoff. A future integration goal starts with the six ordered steps above and must resolve the user-intent fork. Lesson: evaluate indexed search against freshness and membership equivalence before warm-query speed. A content index supporting filename listing does not automatically make it the right file-picker backend.
