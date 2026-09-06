# Graph-powered code intelligence (jcode-hlz docs)

Empryo-style Soul Map port: live per-repo code graph, blast radius before
edits, symbol-level edits. All tools read-only except `symbol_relocate`
(same approval tier as `edit`).

## Tools

- `code_impact {action, file, refresh?}` — dependents / dependencies /
  cochanges / blast_radius. Every output carries `source: index|live|degraded`
  plus `partial` when degraded. Path escapes rejected.
- `code_query {pipeline}` — composable stages: search, find, filter, deps
  (imports|imported_by), outline, read, limit. Caps: 12 ops, 200 files,
  40 outline, 10 read.
- `symbol_relocate {file, symbol, new_name, dst?}` — atomic rename/move over
  text refs: mandatory blast preview, temp-copy staging, all-or-nothing swap
  with restore, typecheck gate offering revert (never auto-revert).
  `dst` unset = rename.

## Advisory blast radius

`edit`/`multiedit` results append a `[codegraph]` line when the edited file
has dependents or co-change partners. Display-only: never blocks or allows.

## Prompt map block

Flag `agents.codegraph_map` (env `JCODE_CODEGRAPH_MAP`), off by default.
Top-30 files by dependents as `path (→N)`, rank-ordered (cache-stable),
≤2k tokens, injected into the static prompt.

## Index

Per-repo SQLite at `<root>/.jcode/codegraph.db` (WAL, busy_timeout 5s).
Lifecycle: Absent → Building → Ready ⇄ Stale → Rebuilding, plus Disabled
(`JCODE_CODEGRAPH=0`) and Corrupt → Absent (auto-delete + rebuild).
Kill switch: `JCODE_CODEGRAPH=0` → all tools degraded, zero sqlite opens.
Benchmark gate: `scripts/bench_codegraph.sh` (5 tasks, ≥30% token cut or
documented why).

## Compaction

`/compact mode structural` — no-LLM mode: file mentions replaced by
outline stubs (app-core symbol version; jcode-base line-count fallback).
Cheapest mode; precision lowest. Other modes unchanged.
