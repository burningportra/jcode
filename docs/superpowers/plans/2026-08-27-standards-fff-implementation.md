# Approved standards and FFF implementation plan

Date: 2026-08-27

## Scope

Implement the approved first production slices from:

- `2026-08-27-engineering-standards-quality-gate-design.md`
- `2026-08-27-fff-agentgrep-backend-design.md`

The standards slice is report-only. It does not add todo completion enforcement, persistence, project overrides, or TUI state. The FFF slice routes only ready, case-sensitive literal `grep` requests with `paths_only=true` and no extra scope or ignore options.

## Sequence

1. Add `jcode-quality-types` and serde contract tests.
2. Add `jcode-base::quality` for Git manifests, Oxlint rule discovery, fingerprints, and report assembly inputs.
3. Add the app-core `quality check` tool, supervised Oxlint execution, bounded evidence, anti-slop completeness reporting, and tool registration.
4. Add focused fixture tests and public tool acceptance tests for pass, violation, missing tool, missing rule, partial scope, metadata round-trip, and stale-candidate rejection.
5. Add optional `fff-search = 0.10.5` packaging behind a typed feature and config mode (`off`, `shadow`, `prefer`).
6. Add the private single-root FFF registry and Phase 1A dispatcher inside AgentGrep.
7. Add deterministic ready-index injection, byte-for-byte path parity, fallback metadata, watcher mutation, case, and resource-bound tests.
8. Run focused tests, `cargo check`, dependency/package inspection, coordinated TUI build-reload, and isolated public-interface acceptance.
9. Commit each coherent slice and record any platform acceptance that remains CI-only.

## Acceptance rule

No test may pass merely because a fallback path ran. Standards tests inspect typed report metadata. FFF tests that claim FFF behavior assert `search_backend = "fff"` and the expected index generation/state.
