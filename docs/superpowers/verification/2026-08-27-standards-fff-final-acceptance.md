# Standards and FFF final acceptance traceability

Date: 2026-08-27
Base implementation under test: `10c205a9d`; this report and its acceptance tests are part of the final verification commit
Host: Apple Silicon macOS, Rust 1.98.0

This report maps the approved first-slice requirements to checks run over the final result. It distinguishes observed local acceptance from release-matrix work that only CI can perform.

## Engineering standards first slice

| Requirement or public output | Concrete check | Observed result |
|---|---|---|
| Configured Oxlint complexity rule can pass changed JS/TS | `public_quality_check_returns_typed_pass_report`; `public_quality_workflow_changes_from_fail_to_pass_after_simplification` | Public `QualityTool::execute` returned `Oxlint complexity: pass`, threshold 3, complete `src/a.ts` scope, and `Pass`. The same workflow changed from `Fail` to `Pass` after simplification while remaining changed from baseline. |
| Threshold violation names rule, file span, command, version, threshold, and config source | `public_quality_check_ties_violation_to_source_span_and_command` | Returned `Fail`; observation points to `src/a.ts:1`, `command-0`, configured max 3, and `.oxlintrc.json`. |
| Complete changed JS/TS manifest; narrower scope is explicitly partial | `partial_scope_is_explicitly_not_gate_eligible`; base quality manifest tests | Complete checks include all changed JS/TS paths. A requested subset returned `complete_changed_scope=false`, `gate_eligible=false`, and `PassWithWarnings`. |
| Missing Oxlint is unknown, never pass | `missing_oxlint_is_unknown_not_pass` | Returned `Incomplete` with `ToolMissing` and `Unknown`. |
| Missing or disabled rule is `rule_not_configured`, not tool missing | `absent_complexity_rule_is_unknown_not_pass` | Returned `Incomplete` with `RuleNotConfigured`. |
| Anti-slop declaration is advisory and structurally checked | `incomplete_anti_slop_accounting_remains_advisory_and_visible` | Returned `PassWithWarnings`, `anti_slop_complete=false`, and `DeclarationIncomplete`; analyzer result was not converted into proof of simplicity. |
| Typed metadata round-trips and does not expose raw source/analyzer output | `jcode-quality-types` serde tests; `public_quality_check_returns_typed_pass_report`; `analyzer_failure_is_incomplete_with_bounded_command_evidence` | `quality_report` deserialized directly from metadata. Evidence summaries excluded source text and raw `not-json` stderr while retaining bounded exit/version facts. |
| Candidate changes during analysis invalidate the result | `candidate_change_during_analysis_discards_the_report` | Returned `Incomplete` with `CandidateChanged`; the earlier analyzer pass was discarded. |
| Candidate cannot weaken or disable the effective rule to manufacture a pass | `candidate_cannot_weaken_repository_complexity_policy` | Returned `Fail` with `PolicyChanged` when max changed from 3 to 10. |
| Analyzer crash or malformed output fails closed | `analyzer_failure_is_incomplete_with_bounded_command_evidence` | Exit 2 and malformed JSON produced `Incomplete` with `AnalyzerFailed` and bounded command evidence. |
| Human summary is coverage-specific, not a general quality claim | `summary_never_claims_general_quality_passed`; live `quality check` on Jcode | Summary says `Oxlint complexity`, never `quality passed`. The live final daemon returned `unknown` for Jcode because no sourced JS/TS complexity rule applied. |
| First slice remains report-only | report subject assertions and schema inspection | Every report sets `gate_eligible=false`; no persistence, todo transition, TUI state, project policy syntax, analyzer plugin trait, or model judge was added. |

## FFF-backed AgentGrep Phase 1A

| Requirement or public output | Concrete check | Observed result |
|---|---|---|
| Keep the public `agentgrep` tool and schema | `schema_only_advertises_common_public_fields`; live debug `tool:agentgrep` | Existing tool name and inputs remained usable. No FFF types crossed the public schema. |
| Cold or warming index falls back without blocking search | isolated prefer daemon on the actual Jcode repository | First request returned linked output with `index_state=warming`, `fallback_reason=index_warming`, and generation 1. |
| Ready compatible request routes through FFF, not a false fallback pass | `prefer_mode_routes_ready_literal_paths_only_search_through_fff`; actual-repository isolated prefer daemon | Ready request reported `search_backend=fff`, `index_state=ready`, generation 1, and the same three Jcode paths as cold linked output. |
| Exact byte parity, lexicographic order, zero-match behavior | prefer, zero-match, whitespace, and actual-repository daemon checks | Outputs matched byte-for-byte; zero matches rendered an empty string; paths were lexicographic. |
| Preserve case-sensitive and literal-whitespace semantics | `prefer_mode_routes_ready_literal_paths_only_search_through_fff`; `ready_fff_preserves_literal_whitespace_semantics` | Mixed-case query excluded lowercase-only files. Leading and trailing query spaces were preserved. |
| Preserve CRLF, Unicode path, ignore, and binary-file behavior | `ready_fff_matches_linked_for_unicode_crlf_ignored_and_binary_files` | FFF and linked output matched exactly and returned only `unicodé.txt`; ignored and binary fixtures did not broaden results. |
| Shadow answers with linked output and reports parity | `shadow_mode_returns_linked_output_and_reports_completed_parity`; actual-repository isolated shadow daemon | Returned `search_backend=linked_agentgrep`, `fallback_reason=shadow_mode`, and `shadow_parity=true` for the same three Jcode paths. |
| Unsupported Phase 1A request classes use linked backend with explicit reasons | isolated prefer daemon plus `phase_one_ineligible_requests_fall_back_with_machine_readable_reason` | Regex, excerpt, path, glob, type, hidden, and no-ignore requests returned linked output with `regex_deferred`, `excerpt_mode_deferred`, `path_scope_deferred`, `glob_scope_deferred`, `type_scope_deferred`, `hidden_files_requested`, and `ignored_files_requested`. |
| Outline, smart, trace, exact-file behavior, exposure, and output caps remain linked | full `tool::agentgrep::tests` module | Existing linked grep, exact-file, smart, trace exposure, compaction, match cap, aliases, and schema tests passed unchanged. |
| Watcher reflects repository mutations within two seconds | `ready_fff_index_observes_created_and_deleted_files`; isolated daemon fixture workflow | Create, modify, rename, and delete each appeared in ready FFF output within the polling deadline. |
| Single-root lifecycle and generation safety | isolated two-repository daemon workflow; `active_index_is_not_replaced_and_global_search_capacity_is_bounded` | Inactive replacement advanced generations 1 to 2 to 3. Active entry replacement returned `index_busy`; a third concurrent permit returned `search_capacity`. |
| Unsafe roots are not indexed | `filesystem_and_home_roots_are_rejected` | Filesystem root and canonical home root both returned no index root. |
| Operator can disable routing | isolated daemon with `fff_backend=prefer`, `max_fff_indexes=0`; feature-disabled compilation | Runtime returned linked backend with `fallback_reason=disabled`. `cargo check -p jcode-app-core --no-default-features` passed and uses `feature_disabled` metadata. |
| Stable packaging without Zig or external service | `cargo tree -e features -p jcode-app-core`; lock inspection; coordinated TUI build | Used `fff-search 0.10.5` with default `ripgrep`; no `zlob`, external FFF binary, Node package, MCP server, or Zig. TUI self-dev build and reload passed on macOS arm64. |
| Warm search is actually better on Jcode | `benchmark_warm_fff_against_linked_agentgrep`, 5 warmups plus 50 measured public Tool calls | Linked p50/p95: 23,783/25,306 µs. FFF p50/p95: 14,010/16,023 µs. FFF improved both and no query approached the five-second foreground budget. |
| Footprint is explicit | with/without feature self-dev binary build | With FFF: 337,556,368 bytes. Without: 326,804,144 bytes. Local self-dev delta: 10,752,224 bytes. |

## Integration boundaries and honest constraints

- The public runtime boundary was exercised with the built Jcode binary, isolated `JCODE_HOME`, isolated runtime directories, custom daemon sockets, headless sessions, and direct debug `tool:agentgrep` calls. Prefer and shadow were also exercised against the actual `/Volumes/1tb/Projects/jcode` repository, not only copied source or a fake index.
- The live active daemon exercised the new `quality` tool and correctly returned `unknown`, not pass, because Jcode's final change contains no changed JS/TS scope with a sourced Oxlint complexity rule. Deterministic JS pass/fail workflow coverage uses real public Tool execution in disposable Git repositories with a controlled Oxlint executable, as required by the approved test strategy.
- Only `aarch64-apple-darwin` and `wasm32-unknown-unknown` targets are installed locally. Linux and Windows native-link and release-profile acceptance cannot be represented honestly on this host. The existing CI and release matrices remain the acceptance boundary for macOS, Linux x86_64, Linux arm64, portable Linux, and Windows.
- The local size measurement uses the self-dev profile because project guidance explicitly avoids slow release builds during self-development. Release-profile size remains a CI/release observation.

## Final verification command

The final whole-result matrix is:

```text
cargo fmt --all -- --check
cargo test -p jcode-quality-types --lib
cargo test -p jcode-base quality::tests --lib
cargo test -p jcode-app-core tool::quality::tests --lib
cargo test -p jcode-app-core tool::agentgrep::tests --lib
cargo test -p jcode-config-types --lib
cargo check -p jcode-app-core --no-default-features
cargo test -p jcode-app-core benchmark_warm_fff_against_linked_agentgrep --lib -- --ignored --nocapture
```

Observed result: the combined matrix completed with exit 0 against the final source and acceptance tests.
