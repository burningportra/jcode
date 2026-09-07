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

## Risks / open questions

- Assumption check (user away): outcome = research only, no prototype. Confirm before building C1+.
- License compatibility of vendored tree-sitter grammars if we port parsers (verify before copying).
- ripwire's Rust grammar quality vs jcode's regex-based symbol scan — unknown; test on this repo.
- Skill-loading cost: ripwire's own agent-loop pilot showed +80% token overhead from reading SKILL.md
  bodies mid-task before they added frontmatter stop rules. Our skills must carry stop rules in frontmatter.
- No cross-model review was possible in this spike (single session); recommendation is author-only.

## Verification (if built)

- Same-correct-answer checks: ranked bundle must surface the same touch points a human grep pass finds
  (ripwire's own honesty bar).
- Token accounting: self-report bundle cost; measure against naive grep-and-read on frozen tasks.
- Degraded-first: missing index / non-git repo → partial + disclose, never hard error (existing contract).
