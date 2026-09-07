# Spike: ripwire integration into jcode

## What ripwire is (grounded 2026-09-07)

- **Not a sandbox.** ripwire ("ripgrep of AI context", redhat-et/ripwire) is a
  zero-dependency C++23 CLI + optional MCP server (`ripwire --mcp`, `.mcp.json` is 3 lines). License: Apache-2.0.
- Gives coding agents a **ranked, deterministic call-graph map** of any repo: relevant symbols, callers, blast radius,
  tests-to-run, quality deltas. Offline, no API key, no embeddings, no daemon, no index server.
- Token story: signatures at ~80% fewer bytes than bodies; conceptual answers at ~5% of a grep-and-read pass.
  Self-reports `est_tokens` on every bundle. Cold parse ~0.15s, warm ~0.10s on its own repo.
- Key verbs: `--for=` (ranked task query, auto-routes name-exact vs conceptual BM25), `--callers=`, `--impact=` +
  `--uses=`, `--expand=`, `--situ` (diff → blast radius + tests), `--pr-context=`, `--quality-panel`,
  `--quality-delta`, `--pack-task=` (+ `--partition=N` for multi-agent splits), `--report`/`--tree`/`--communities`/
  `--zoom`/`--hotspots`, `--recall=` (docs/memory), `--note-add` (persistent gotchas in `.ripwire_notes`).
- 21 vendored tree-sitter grammars including **Rust** and TypeScript. Adding a language = vendored grammar + one
  table row.
- Ships **task-shaped skills** (`ripwire-orient`, `-navigate`, `-find-bug`, `-change-check`, `-fresh-eyes`,
  `-efficient`, `-quality-bar`, `-layers`, `-partition`-aware `explore`, …) auto-installed for detected agents.
  Skills teach *when* to reach for it, not just how. This is the part most worth copying even if we never run
  their binary.
- Honesty surfaces: every guess labeled, every truncation disclosed, unindexed languages / skipped files itemized.

## What jcode already has (grounded in tree)

- `crates/jcode-app-core/src/tool/codegraph/` (~1.8k lines): `live.rs` (blocking fns over `rg` + git),
  `indexed.rs` (SQLite via `jcode-codegraph` crate: scan, rank incl. pagerank + co-change from log, query,
  symbols), `cache.rs`, `map.rs`, `structural.rs`. Degraded-first contract (partial results, never hard errors).
- `code_impact` tool: dependents / dependencies / co-changes / blast radius + advisory one-liner on edits.
- `code_query` tool: 12-op pipeline (search → find → filter → deps → outline → read → limit) over agentgrep +
  codegraph. Zero file I/O until explicit `read`.
- `agentgrep` with fff backend for fast search.
- Gap vs ripwire: **no symbol-level ranking** (file-level dependents, grep-ordered callers), **no token budgeting**
  (full bodies / whole files), **no churn/complexity annotation**, **no quality panel**, **no task-shaped skills**
  guiding when to use these tools, **no conceptual (BM25/doc-comment) retrieval lane**.

## Integration options

| # | Option | Shape | Cost | Value |
|---|--------|-------|------|-------|
| A | **External-tool convention (docs + skills only)** | Document ripwire as an optional external binary; add jcode skills that shell out to it when present, degrade to codegraph when absent | S: no Rust code, skills markdown + detection | Medium: token savings for users who install it; zero benefit otherwise |
| B | **Bundled sidecar invocation** | Vendor or auto-install the ripwire binary; `code_query`/`code_impact` shell out to it with fallback to native codegraph | M: install/download plumbing, output parsing, fallback logic | High where installed; new binary dependency + platform matrix (macOS arm64 ok? verify releases) + version skew |
| C | **Port the ideas natively** | Add ranked symbol retrieval + token budgets + churn/cx annotation to `jcode-codegraph`/`code_query`; add task-shaped skills | L (fits existing roadmap: graph-AI phases) | Highest long-term: no dependency, works offline everywhere, compounds with existing index; slowest payoff |
| D | **MCP client mode** | jcode speaks to `ripwire --mcp` as an MCP server | M: MCP client plumbing for one server | Low: jcode already has `mcp` tool + CLI is simpler; MCP adds framing overhead for a local binary |
| E | **Hybrid: native first, ripwire fallback** | `code_query` ranks from the native SQLite index; when the query hits languages/symbols the native scan handles poorly, shell out to an optional ripwire binary for that slice only | M: fallback detection + slice merging + ranking merge rule | Medium-high: best answer quality without full sidecar dependence; complexity is the merge rule (whose rank wins) |

## Data flow (options C/E)

```
agent → code_query pipeline [search → rank → filter → outline → read]
                       │
              ┌────────┴────────┐
              │ rank stage (new) │
              │ name-exact BM25  │── symbols + doc comments from jcode-codegraph index
              │ conceptual BM25  │── (E only) ripwire slice merged here on fallback
              └────────┬────────┘
                       │ ranked rows + route= disclosure + est_tokens
              ┌────────┴────────┐
              │ annotate (C3)   │── churn (git log) + complexity + tested flags
              └────────┬────────┘
                       │ budget cut (C1): top-k / max-tokens / signatures-only
                  outline/read
```

`code_impact` is unchanged structurally: it consumes the same annotated index for dependents/co-changes.
`agentgrep` stays the lexical substrate; ranking sits above it, not instead of it.

## Recommendation

**C primary, A as the cheap immediate step.** Rationale:

1. jcode already paid for ~80% of the infrastructure (SQLite index, pagerank, co-change, live fallback).
   The missing 20% (symbol ranking, BM25 lanes, token budgets, skills) is exactly ripwire's proven design —
   its EVALS.md is a free spec with measured numbers and published counterexamples.
2. A sidecar (B) buys speed but inherits a C++ build dependency, release-matrix risk, and output-format
   coupling for a component that duplicates our index. Revisit B only if native ranking stalls.
3. D adds nothing over B's CLI for a local binary.
4. A is nearly free and useful regardless: task-shaped skills (`orient` before editing, `change-check`
   before push, `find-bug` ladder) improve tool use whether the backend is ripwire or our codegraph.

## Smallest shippable (if pursued)

1. **A0 — skills only:** add `orient`/`change-check` skills routing to existing `code_query`/`code_impact`;
   measure token spend before/after on 3–5 real tasks. (No code changes.)
2. **C1 — token budgets:** `--max-tokens`/`--top-k` + signatures-only mode on `code_query` output.
3. **C2 — ranked symbols:** BM25 name-exact + conceptual lanes over indexed symbols + doc comments,
   with disclosed `route=` equivalent.
4. **C3 — annotation:** churn (git log) + complexity + test-coverage flags inline on ranked rows.
5. Only then consider B (sidecar) for languages jcode's index handles poorly.

## C2 redesign: all-items detection (implemented)

Scope: regex-widen Rust detection in `jcode-codegraph`; focused tests; benchmark. BM25 and tree-sitter deferred.

- Widen `exported_symbols()` Rust patterns in `crates/jcode-codegraph/src/symbols.rs` from pub-only to
  all-items: `fn` (any visibility, incl. `async`/`unsafe`/`extern`), methods inside `impl` blocks
  (kind `method` via brace-depth span tracking), `struct`/`enum`/`trait`/`mod`/`const`/`static`/`type`
  at any visibility, `#[test]` fns (kind `test`). Mirror the same widening in `exported_symbols_live()`
  (`crates/jcode-app-core/src/tool/codegraph/live.rs`) now delegates to the shared crate function
  instead of duplicating patterns.
- Dedup key must become (name, line), not name: methods named `new`/`execute` repeat across impl blocks
  (the current `seen` set on name alone would collapse them).
- Kind vocabulary addition: `method`, `test`, `type`. Existing consumers (`query.rs` FTS insert,
  `code_query`, `code_impact`) read kind as opaque string; no schema change (`symbols` table already
  stores kind/line).
- Tests (in `symbols.rs` `mod tests`): private fn detected; `pub(crate)` fn detected; method in
  `impl Foo` detected as `method`; `#[test] fn` detected as `test`; line numbers correct; no-duplicate
  collapse for same-named methods at different lines; existing pub-only behavior preserved as subset.
- Benchmark bar (from kev-a7h probe): native count on `crates/jcode-app-core/src/tool/` (99 files) moves
  290 toward ~2551 (ripwire 2548); the 5 probe queries (CodeImpactTool, ImpactReport, compute_report,
  advisory_blast_line_async, render) must resolve in the widened index (name-exact lookup, pre-BM25).
- Non-goals: BM25 lanes, route disclosure, tree-sitter, other languages (Rust only in this step).

### C2 results (executed)

- `cargo test -p jcode-codegraph`: **15 passed, 0 failed** (7 symbol tests incl. 5 new focused tests).
- `cargo test -p jcode-app-core --lib codegraph`: 23 passed, 1 failed —
  `repo_root_stops_at_home_boundary` fails on the clean tree too (pre-existing, environment-dependent:
  asserts `~/.git` exists; unrelated to this change; verified via stash).
- Benchmark: widened patterns yield **2557 symbols over 97 `.rs` files in 0.20s** vs ripwire 2548
  (over 99 files incl. 2 `.html` testdata docs) and probe naive count 2551. Delta of +6-9 is within
  regex-vs-tree-sitter noise. Bar met: 290 -> 2557 (~8.8x).
- Rank check: all 5 probe queries are pub items in the widened set (strict superset of old detection),
  so name-exact resolution holds pre-BM25.
- Follow-up for BM25 bead: method rows need parent-type context (kind only today) to disambiguate
  same-named methods.

## Probe results (kev-a7h, executed 2026-09-07)

Binary: ripwire v0.3.8 macos-arm64 (sha256-verified, kept out of repo). Corpus: `crates/jcode-app-core/src/tool`
(99 files). Report: **2548 symbols, 7368 edges, 324 modules, 0 skipped, ~0.12s cold.**

- Coverage parity: native ALL-items regex count on the same tree = 2551 (1984 fn + 567 types).
  Ripwire's tree-sitter Rust grammar misses nothing measurable.
- Native gap confirmed: `exported_symbols()` (pub-only regexes) finds **290 symbols (11%)**. It misses
  private fns, all methods, impl blocks, test fns. **C2 must widen symbol detection to all-items first**
  (mirror the 2551-count set; prefer tree-sitter over more regex), then add BM25 lanes + route disclosure.
- Ranking works on Rust: 5/5 name queries rank-1; conceptual query landed the right rows with
  cx/churn/amp inline; `--callers`/`--uses` correct (edit.rs:68 + multiedit.rs:78 for the async variant).
- Limitations: single-file path arg indexes nothing (root must be a dir); `--tree` shows top-3 symbols/file;
  call edges are name-based heuristics (trait dispatch invisible, disclosed); 0-counts need verify-before-trust;
  v0.3.8 is 2026-08-13.
- Verdict: **PROCEED TO C2 WITH REDESIGN.** C1 (token budgets) unaffected, proceeds independently.

## Risks / open questions

- Assumption check (user away): outcome = research only, no prototype. Confirm before building C1+.
- Tree-sitter grammars are vendored C++ in ripwire's tree; porting parsers means porting grammar + table row
  under Apache-2.0 with attribution (NOTICE compliance). Prefer reusing existing Rust tree-sitter crates over
  copying ripwire's vendored sources.
- ripwire's Rust grammar quality vs jcode's regex-based symbol scan — unknown; test on this repo.
  Concrete probe: index jcode with ripwire, compare symbol counts against `jcode-codegraph` scan on
  `crates/jcode-app-core/src/tool/`; investigate the delta before committing to C2.
- Skill-loading cost: ripwire's own agent-loop pilot showed +80% token overhead from reading SKILL.md
  bodies mid-task before they added frontmatter stop rules. Our skills must carry stop rules in frontmatter.
- No cross-model review was possible in this spike: the spawned reviewer (gemma-4-31b via Cerebras) failed
  on endpoint auth, so all passes are same-model fresh-eyes. Recommendation stays author-only; treat the
  options table as provisional until a second model weighs in.

## Non-goals (explicit)

- No vendored ripwire binary in the jcode release; no new daemon or background indexer.
- No quality-panel / quality-delta port in this initiative (flag as follow-up; C1–C3 are retrieval only).
- No `--partition` multi-agent split support until single-agent ranking proves out.
- No changes to approval policy: ranking is advisory, never blocks edits (existing contract).

## Edge cases and failure handling

- **ripwire absent (A/E):** detect via PATH probe at tool init; degrade silently to native codegraph, disclose
  `source: native (ripwire not found)` on the bundle. Never hard-error a query because an optional binary is missing.
- **Stale or partial index:** reuse the existing degraded-first contract — partial rows + `partial: true` +
  named source; `refresh` flag forces reindex.
- **Non-git repos:** churn annotation degrades to `churn: unknown`; ranking still works off symbols + doc comments.
- **Ambiguous symbol names:** refuse to guess; return did-you-mean candidates (mirrors ripwire's resolver).
- **Token budget overflow:** cut at the relevance cliff, disclose `capped=N`; never silently drop top-ranked rows.
- **Version skew (B/E):** pin minimum ripwire version; on parse failure of its output, fall back to native and log.

## Verification (if built)

- Same-correct-answer checks: ranked bundle must surface the same touch points a human grep pass finds
  (ripwire's own honesty bar).
- Token accounting: self-report bundle cost; measure against naive grep-and-read on frozen tasks.
- Degraded-first: missing index / non-git repo → partial + disclose, never hard error (existing contract).
