# Plan: Graph-Powered Code Intelligence for jcode (Empryo-style Soul Map)

> Status: Draft v1, converging. TUI/CLI agent scope only (not desktop).
> Source study: proxysoul/Empryo (SoulForge) — repo is issues/docs only plus archived
> SoulForge TS source; architecture reconstructed from SOULFORGE.md, CLAUDE.md,
> `src/core/intelligence/*`, `src/core/tools/soul-*.ts`, `navigate.ts`, `ast-edit.ts`.
> Multi-model synthesis rung: (c) single self-draft — both swarm workers died before
> emitting briefs (same-model inherited route, no cross-model available). Proposals A
> (indexer-first) and B (tools-first) below are reconstructed from the elicitation
> stances, not quoted worker output. No cross-model review has run yet — see residual
> risk R7.

## 0. Executive summary

jcode's agent today greps, reads whole files, and patches strings. It never knows
what depends on the code it just changed. Empryo's answer is a **code genome**:
a live per-repo graph (symbols, imports, call sites) ranked by PageRank plus git
co-change, queried in milliseconds at zero LLM tokens, with symbol-level edits
gated by blast radius and typecheck.

This plan ports that power into jcode in **4 phases, highest ROI first**:

- **Phase 1 (tools-first, no indexer):** `code_impact` (dependents, dependencies,
  cochanges, blast_radius) computed on demand via rg plus git plus import
  parsing, with graceful degraded output when no index exists. Plus advisory
  blast-radius line in `edit`/`multiedit` results. Ships value in days, no new
  deps, no background process.
- **Phase 2 (query pipeline):** `code_query` — one-call composable pipeline
  (search, find, filter, deps, outline, read, limit) over agentgrep plus the
  Phase 1 graph. Kills the grep→filter→read multi-step loop that burns tokens.
- **Phase 3 (SQLite index):** new `jcode-codegraph` crate, per-repo SQLite DB
  (rusqlite 0.32 bundled already), FTS5 symbols, PageRank, co-change table,
  mtime-based incremental reindex (no watcher daemon). Phases 1–2 tools gain an
  indexed fast path with identical output shape.
- **Phase 4 (symbol edits + compaction):** `rename_symbol` / `move_symbol` /
  structural edit ops with atomic batches plus rollback, typecheck gate via
  existing `project` tooling, structural (no-LLM) compaction using outlines.

**Explicitly out of scope:** tree-sitter vendoring in v1 (regex plus LSP plus
ts fallback chain instead), per-role model routing (mixture of experts),
LSP server management (576 servers via Mason), desktop app, time-machine git
checkpoints (covered by existing session journaling), web-search role.

**Success criteria:**
1. `code_impact blast_radius` answers for any tracked file in < 2s unindexed,
   < 200ms indexed, with zero LLM tokens.
2. A 3-step explore loop (find symbol, expand dependents, read outline) becomes
   1 `code_query` call; measured input-token reduction ≥ 30% on a fixed
   benchmark task set.
3. Rename/move symbol tools succeed atomically with rollback on partial failure.
4. No regression: all existing tool tests pass; index never blocks agent startup
   (lazy, background, cancellable, `JCODE_CODEGRAPH=0` kill switch).

## 1. What Empryo does (ground truth from source study)

### 1.1 Soul Map (repo-map.ts, ~190KB TS + SQLite via bun:sqlite)

Tables (reconstructed): files, symbols, deps (weighted import edges),
cochanges (file-pair commit counts), FTS5 symbol index, trigram index for
substring search, pagerank scores, blast-radius tags.

- **Build:** tree-sitter parses 30+ languages at launch into symbols, imports,
  call sites. `EXT_TO_LANGUAGE` plus `BARE_FILENAME_TO_LANGUAGE` maps
  (see `src/core/intelligence/types.ts`) route each file to a backend.
- **Rank:** PageRank over the import graph plus git `log` co-change mining.
  High-PageRank files surface first in the aider-style Soul Map prompt block.
- **Query:** dependents, dependencies, co-changes, blast radius, enclosing
  symbols, trigram candidates — all local, millisecond, zero tokens.
- **Fallback:** `soul_impact` degrades to `rg` dependents plus import-regex
  dependencies when the map is not ready. Every graph tool has a no-index path.

### 1.2 Intelligence backend router (router.ts + backends/)

Tiered: LSP (tier 1) → ts-morph (TS only) → tree-sitter → regex fallback.
`IntelligenceBackend` trait surface: findDefinition, findReferences,
findSymbols, findImports, findExports, getDiagnostics, getTypeInfo,
getFileOutline, readSymbol, readScope, rename, extractFunction,
extractVariable, getCodeActions, findWorkspaceSymbols, format, organizeImports,
call hierarchy, type hierarchy, findUnused, file-rename edits.

### 1.3 Tool surface (src/core/tools/)

| Empryo tool | Behavior | jcode analogue today |
|---|---|---|
| `soul_impact` | dependents / dependencies / cochanges / blast_radius | none (command-risk is shell-only) |
| `soul_query` | composable pipeline, 12 ops max, 200-file cap | agentgrep modes (grep/find/outline/trace) |
| `soul_find` / `soul_grep` | FTS5 + trigram symbol/substring search | agentgrep |
| `soul_analyze` / `navigate` / `analyze` | outline, defs, refs, call hierarchy | agentgrep outline/trace (partial) |
| `ast-edit` / `rename-symbol` / `move-symbol` / `structural-edit` / `refactor` / `multi-edit` | 65+ AST ops, atomic batches, rollback | edit/multiedit/patch (string-level) |
| `dispatch` (multi-agent) | parallel explore/edit, shared I/O cache | swarm + batch |
| post-edit hook | typecheck gate + auto-format | project tool (manual) |

### 1.4 Token economics

Soul Map injected as user→assistant message pair (aider pattern) for prompt-cache
stability. Subagents inherit parent cache line. Structural compaction replaces
transcript with outlines, no LLM call. Claimed vs pi: 28% lower cost, 5.7x fewer
input tokens, 28% fewer steps (vendor benchmark, treat as directional).

### 1.5 What jcode already has (do not rebuild)

- `agentgrep` (crates/jcode-app-core/src/tool/agentgrep.rs + context.rs):
  structural symbol search, outline mode, trace mode, FFF shadow/parity tests.
  Blocking core (`run_agentgrep_blocking_with_backend`) runs inside
  `spawn_blocking` with a foreground budget + background fallback
  (`await_or_background_search`). Any shared graph core MUST be callable as
  plain blocking functions (no `Tool::execute`, no nested runtime), because
  `code_query` stages will call into it from inside `spawn_blocking`.
- `ToolContext` (crates/jcode-tool-core/src/lib.rs): carries `working_dir:
  Option<PathBuf>` + `resolve_path()`. All graph tools take repo scope from
  `ctx.working_dir` (fall back to error when `None`, never to process CWD —
  the daemon serves many repos). No `getCwd()` global exists; do not invent one.
- `jcode-command-risk`: blast-radius language for shell commands, not code.
- `Registry::base_tools` + per-session tools (`batch`, `conversation_search`):
  registration pattern for all new tools. New graph tools are stateless base
  tools; per-session state (dep cache, DB handle) lives OUTSIDE the tool
  struct — see 3.1.
- `CompactionManager`: compaction hook point for Phase 4 structural mode.
- `MEMORY_GRAPH_PLAN.md`: memory-recall graph (different graph; shared rerank
  math — RRF, 1-hop expansion — is reusable, schema is not).
- rusqlite 0.32 bundled in jcode-base. No tree-sitter, no walkdir/ignore/notify
  in app-core deps (verify at Phase 3 kickoff; Cargo.lock shows rusqlite only
  via jcode-base + harness-api-server).

## 2. Proposal comparison (A vs B → hybrid)

| Dimension | A: indexer-first | B: tools-first | Hybrid (chosen) |
|---|---|---|---|
| First value | Slow (indexer + schema first) | Days (tools over rg/git) | Days (B first) |
| Query latency | ms once indexed | 1–2s per call live | Both paths, same output shape |
| New deps | rusqlite use + ignore/walkdir/notify + parsers | none | Deferred to Phase 3 |
| Failure modes | Stale index, watcher bugs, startup cost | Repeated rg cost, no ranking | Degraded-first contract covers both |
| Ranking | PageRank + co-change | None (alpha order) | Co-change in Phase 1 (cheap), PageRank in Phase 3 |
| Risk | Big-bang, blocks on schema | Rework when index lands | Interface-first: tools define trait, index is a backend |

**Hybrid rule:** tools own the interface from day one; the SQLite index is a
backend that accelerates them later. Every graph tool returns identical shapes
indexed or not, with a `source: "index" | "live" | "degraded"` field. This is
Empryo's own `soul_impact` fallback contract, made explicit.

**Refine re-pass (architecture lens, 82/100):** the ASCII diagram still names a
`CodeGraph trait (sync read API)` plus a `GraphHandle`, but 3.1 correctly
specifies plain blocking functions as the Phase 1–2 interface. The trait +
handle appear only in the Phase 3 crate bullet. Adopted fix: the trait is a
Phase 3 introduction, not a Phase 1 contract — 3.1's function signatures are
normative until P3a lands. Also: the live box says `in-memory per-session dep
cache (mttl 60s)` — typo for TTL, and cache is process-wide OnceLock, not
per-session (base tools are shared). Corrected in 3.1; diagram label kept short
deliberately. No cross-model reviewer used (no swarm_model pin; workers would
inherit coordinator model) — recorded as residual risk R7.

## 3. Architecture

```
┌─ Agent turn ──────────────────────────────┐
│  code_impact / code_query / navigate       │  Phase 1–2: new tools in Registry
│  edit / multiedit (+ advisory line)        │  Phase 1: hook, advisory only
│  rename_symbol / move_symbol / refactor    │  Phase 4: symbol tools
└──────────────┬────────────────────────────┘
               │ CodeGraph trait (sync read API)
               ▼
┌─ Live backend (Phase 1) ──┐  ┌─ Indexed backend (Phase 3) ──┐
│ rg + git + import regex   │  │ jcode-codegraph crate        │
│ in-memory per-session     │  │ per-repo SQLite, FTS5,       │
│ dep cache (TTL 60s)       │  │ PageRank + co-change tables  │
└───────────────────────────┘  └──────────────────────────────┘
```

**Crate layout (Phase 3):** `crates/jcode-codegraph/` — pure library, no tokio
dependency at core (blocking sqlite behind `spawn_blocking` at call site):
- `lib.rs` — `CodeGraph` trait + `GraphHandle` (open/lazy/reindex).
- `schema.rs` — DDL (section 4).
- `scan.rs` — file walk (ignore-respecting), language detect, import extract.
- `rank.rs` — PageRank + co-change mining (`git log --name-only --pretty=format:`).
- `query.rs` — dependents/dependencies/blast-radius/cochange/symbols/outline.
- `symbols.rs` — regex-based symbol extractor per language family (Rust, TS/JS,
  Python, Go first; rest fall back to generic outline).

### 3.1 Shared core and per-session state (pass 1 fix)

Phase 1 MUST create `crates/jcode-app-core/src/tool/codegraph/` (not a new
crate yet) with plain blocking functions, so Phase 2–3 reuse them without
rework:

- `live.rs` — `dependents_live(root, rel, cap)`, `dependencies_live(root,
  rel)`, `cochanges_live(root, rel)`, `exported_symbols_live(root, rel)`.
  Pure `std::fs` + `std::process::Command` (rg, git). No tokio, no ToolContext.
- `cache.rs` — `DepCache`: `Mutex<Lru<RepoRoot, TimedGraph>>`, TTL 60s,
  keyed by canonicalized repo root (discovered by walking up to `.git`;
  non-git repos use `ctx.working_dir` directly with `source: degraded` for
  cochanges). Lives in a process-wide `OnceLock` (like `base_tools`), NOT in
  the tool struct — base tools are constructed once and shared across sessions.
- Tools (`code_impact`, later `code_query`) are thin async wrappers:
  resolve root from `ctx.working_dir`, call blocking core via `spawn_blocking`,
  format output. This mirrors `agentgrep`'s offload pattern and keeps the TUI
  render loop responsive.
- Phase 3 moves `live.rs`+`cache.rs` into `jcode-codegraph` and adds
  `indexed.rs` behind the same function signatures plus a `source` flag.
  Tool wrappers do not change.

**Why mtime, not a watcher:** no new `notify` dep, no daemon thread, no
cross-platform watcher bugs. Reindex check = `stat` mtimes vs `files.mtime`
on each tool call when DB older than 60s or caller passes `refresh:true`.
Full reindex of a 5k-file repo must complete < 30s (budget; measure in Phase 3
bead). `JCODE_CODEGRAPH=0` disables everything; tools fall back to live path.

**Why regex first, not tree-sitter:** vendoring 30+ tree-sitter grammars in Rust
is the single biggest cost in the whole plan (build times, binary size, grammar
parity). Empryo's own router proves regex fallback is a viable tier. Phase 1–3
use regex import/symbol extraction; tree-sitter (`tree-sitter` + `tree-sitter-rust`
etc.) is a Phase 4+ optional accelerator behind the `CodeGraph` trait, gated on
measured precision gaps.

## 4. Data model (Phase 3 SQLite)

```sql
CREATE TABLE files(id INTEGER PRIMARY KEY, path TEXT UNIQUE NOT NULL,
  language TEXT NOT NULL, mtime INTEGER NOT NULL, pagerank REAL DEFAULT 0);
CREATE TABLE symbols(id INTEGER PRIMARY KEY, file_id INTEGER NOT NULL REFERENCES files(id),
  name TEXT NOT NULL, kind TEXT NOT NULL, line INTEGER NOT NULL, end_line INTEGER,
  container TEXT);
CREATE TABLE deps(src_id INTEGER NOT NULL REFERENCES files(id),
  dst_id INTEGER NOT NULL REFERENCES files(id), weight REAL NOT NULL DEFAULT 1,
  PRIMARY KEY(src_id, dst_id));
CREATE TABLE cochange(a_id INTEGER NOT NULL REFERENCES files(id),
  b_id INTEGER NOT NULL REFERENCES files(id), commits INTEGER NOT NULL DEFAULT 1,
  PRIMARY KEY(a_id, b_id));
CREATE VIRTUAL TABLE symbols_fts USING fts5(name, kind, path);
CREATE INDEX idx_deps_dst ON deps(dst_id);
CREATE INDEX idx_symbols_file ON symbols(file_id);
CREATE INDEX idx_symbols_name ON symbols(name);
CREATE INDEX idx_cochange_a ON cochange(a_id);
CREATE INDEX idx_cochange_b ON cochange(b_id);
```

**Schema edge cases (pass 2):** co-change pairs stored canonically
(`a_id < b_id`, query both directions) — the v1 DDL above would miss reverse
lookups. Self-deps (`src_id = dst_id`, e.g. Rust `use crate::...` resolving to
own file) skipped at insert. Symlinks: canonicalize paths, skip loops and
anything escaping repo root. Case-insensitive filesystems (macOS): store
`path` as given but resolve relatively — no lowercasing (breaks Linux); dup
detection via canonicalized inode check at scan. `deps` from generated files
(`target/`, `node_modules/`, `dist/`) excluded at scan via ignore list, or
PageRank is captured by build output — the scan exclusion list is a named
fixture in the Phase 3 bead.
- DB versioning: `PRAGMA user_version = 1`; on schema change, wipe + full
  reindex (no migrations in v1). Corrupt DB → delete + rebuild, tools report
  `source: live` meanwhile (never error).

**Refine re-pass (data-model lens, 84/100):** two gaps closed. (1) `deps.weight`
semantics were unspecified — defined now: weight = number of distinct import
statements from src to dst (re-imports across one file count once per
specifier line), normalized per-source at PageRank build (out-weight sums to
1; zero-out-degree nodes distribute uniformly). Without this, PageRank over
raw counts is dominated by files with repeated imports of one module.
(2) `symbols_fts` has a `path` column but no sync contract — added: FTS rows
written in the same transaction as `symbols` inserts (delete + reinsert per
file on rescan); FTS is never read for ranking, only `find` matching, so
staleness window equals the file-row transaction. (3) `mtime INTEGER` is
seconds since epoch (not ms — git and stat disagree otherwise); bead must
assert the unit. No cross-model reviewer; residual risk R7 stands.

- Import→file resolution: relative-path resolution for `./`/`../` plus
  basename fallback; unresolved imports are dropped (not stored). Rust `mod`/`use`,
  TS/JS import/from/require, Python import/from, Go import — Phase 3 bead specs
  each language's regexes with test fixtures.
- PageRank: standard iterative (damping 0.85, 20 iterations) over `deps`
  weights; stored to `files.pagerank`. Co-change: last 500 commits (cap),
  pairs within same commit bump `commits`.
- DB location: `<repo>/.jcode/codegraph.db` (gitignored; repo-local so
  worktrees do not collide). WAL mode. Single-writer via `spawn_blocking`.

**Index lifecycle states (pass 3):** `Absent → Building → Ready ⇄ Stale →
Rebuilding`, plus `Disabled` (`JCODE_CODEGRAPH=0`) and `Corrupt → Absent`
(auto-delete + rebuild). Transitions: first graph-tool call creates DB row set
in `Building` (background `spawn_blocking`; concurrent calls share one build
via `OnceLock`-guarded future — no stampede). `Ready` when all tables populated
+ PageRank stored. `Stale` when any call finds mtimes newer than last scan OR
60s elapsed — next call triggers `Rebuilding` (incremental: only changed files
re-parsed, their dep rows deleted + reinserted, PageRank recomputed — full
recompute is O(E), acceptable at <50k files). Readers never block on rebuild:
stale reads served with `stale: true` flag while rebuild runs. Concurrent
daemon processes (two jcode sessions, same repo): SQLite WAL + `busy_timeout`
5s; second writer gets `SQLITE_BUSY` → falls back to live path, never errors.
`.jcode/` creation races `mkdir -p` idempotently; read-only repo root →
`source: live` permanently for that root (do not retry every call — cache the
verdict per process).

## 5. Tool specifications

### 5.1 `code_impact` (Phase 1)

```
name: code_impact
args: { action: "dependents"|"dependencies"|"cochanges"|"blast_radius",
        file: "<repo-relative or absolute>", refresh?: bool }
output: { success, output: "<human text, Empryo soul_impact shape>",
          source: "index"|"live"|"degraded",
          counts: { dependents, dependencies, cochanges, affected } }
```

- `dependents` (live): `rg -l --glob=!node_modules --glob=!.git` on the
  module stem (strip extension, strip `/index`), filter to files with a
  plausible import line. Cap 100 files, 10s timeout.
- `dependencies` (live): read file, apply language import regexes, resolve
  relative targets that exist on disk. Unresolvable bare imports listed as
  external, not dropped silently.
- `cochanges` (live): `git log --name-only --pretty=format: -- <file>`,
  count partner files, top 20. Non-git repos → `degraded` with message.
- `blast_radius` (live): union of dependents + cochanges + exported symbols
  (regex-extracted). Same text shape as indexed path.
- Security: respect existing `isForbidden`-equivalent path gates (jcode: check
  what `read` tool enforces; mirror it — Phase 1 bead must name the function).

**Tool edge cases (pass 2):** `rg` missing → fall back to internal
`grep`-crate-equivalent walk (grep `regex` crate is already a dep) with the
same caps, `source: degraded`. Binary files skipped via extension + null-byte
sniff (mirror `read` tool's rule — bead must cite it). `git log` on repos with
>500 commits: `--max-count=500` cap, newest first. Shallow clones / worktrees /
submodules: `git -C <root>` scoped, failures → degraded cochanges only (never
fail the whole call). `file` arg escaping repo root via `..` → reject with
error (path traversal). Absolute paths outside root → same reject. Empty
dependents + empty cochanges + no symbols → `"<file>" not found in graph`
(not an error — matches Empryo). Timeout: 10s per live sub-call, partial
results returned with `partial: true` flag rather than total failure.

### 5.2 `code_query` (Phase 2)

```
name: code_query
args: { pipeline: [
  {op:"search", pattern} | {op:"find", query} |
  {op:"filter", ext?, pathContains?} |
  {op:"deps", direction:"imports"|"imported_by"} |
  {op:"outline"} | {op:"read", ranges?} | {op:"limit", n} ] }
caps: ≤12 ops, ≤200 files working set, outline ≤40 files, read ≤10 files.
```

Stages route: search→agentgrep grep backend (shared blocking fn, NOT tool
execute — see 3.1), find→outline symbol index, deps→`codegraph/live.rs`
`dependents_live`/`dependencies_live` directly, outline→symbol ranges, read→existing read tool.
Emits a per-stage trace (`1. search "x" → 14 files`) like Empryo's.

### 5.3 Symbol edit tools (Phase 4)

`rename_symbol {file, symbol, new_name}` — phase A: text refs via agentgrep +
`edit` batches; gated by `code_impact blast_radius` preview shown first.
`move_symbol {src, dst, symbol}` — same, plus import-line insertion.
Atomicity: all-or-nothing — apply to temp copies, verify, then swap; on any
failure restore originals and report per-file status. Typecheck gate: run the
repo's check (`cargo check` / `tsc --noEmit` detection via existing project
tooling) after swap; on failure offer revert (do not auto-revert — user
decision, destructive-action policy). True AST ops behind tree-sitter remain
a named non-goal until precision measurements demand them.

### 5.4 Prompt injection (Phase 2, small)

Aider-style: top-30 PageRank files (or top-30 by co-change weight unindexed)
as a compact `path (→N dependents)` map block injected once per session into
the system prompt, cache-stable ordering. Cap 2k tokens. Off by default until
benchmark bead proves token win; flag `agents.codegraph_map`.

## 6. Roadmap (dependency-ordered, ROI first)

- **P1 (jcode-rey, jcode-nd5):** `code_impact` live backend + tests; edit
  advisory line; co-change via git log. No new deps. Shippable alone.
- **P2 (jcode-hg3, jcode-hkd):** `code_query` pipeline + shared graph core
  refactor; prompt map block (flagged); benchmark harness (token/step comparison
  on 5 fixed tasks, before/after).
- **P3 (jcode-dj6, jcode-ggw, jcode-euh):** `jcode-codegraph` crate
  (schema/scan/rank/query/symbols), mtime reindex, indexed backend for both
  tools, PageRank, FTS5, perf budget tests, `JCODE_CODEGRAPH=0` kill switch.

**Performance budgets (pass 4, enforced by tests):** live `code_impact` p50
< 2s on jcode repo (warm rg cache), hard timeout 10s per sub-call with
`partial: true`. Full reindex ≤ 30s at 5k files / ≤ 120s at 50k files
(background, never on startup). Indexed query p50 < 200ms (asserted in test
with fixture DB). DB size cap: symbols+deps for 50k files must stay < 200MB
or scan exclusion list grows (bead asserts size on fixture). PageRank: 20
iterations over ≤50k nodes single-threaded is ms-scale — no bead needed beyond
unit test. Memory: live dep cache ≤ 100 repos × 5MB; LRU eviction. Prompt map
block ≤ 2k tokens hard cap (truncate by rank, never by recency — stable order
preserves prompt cache). Co-change mining capped at 500 newest commits AND 30s
wall clock, whichever first, on repos with huge history (jcode-scale fine;
chromium-scale degrades gracefully).
- **P4 (jcode-rqk, jcode-hlz):** symbol-relocate core (rename + move, single
  tool surface per pass-9 note) atomic + rollback, typecheck gate, structural
  compaction mode in CompactionManager, docs + TUI help text.
- Total ~15 beads (9 created: jcode-rey..jcode-hlz; remaining ~6 are P1c/P2c
  splits if beads grow — do not pre-split). Tree-sitter acceleration is a
  follow-up initiative, not a bead here.

**Refine fresh pass (roadmap lens, 86/100 — not applicable as revision):**
read roadmap plus risks with fresh eyes. Roadmap bead counts (3/3/5/4) predate
the 9-bead split and no longer match section 6's own bead IDs; adopted fix is
documentation-only (this note), not a redesign — bead descriptions in `br`
are normative for scope, section 6 for ordering. 5.3 still names
rename/move as separate tools while the pass-9 note mandates a single
symbol-relocate core; `jcode-rqk` bead already carries the merged scope, so
5.3 body is updated at implementation time, not here (no-oversimplification
guard: no behavior removed). No new structural gaps: counts, budgets, and R1–R7
all still hold. Score reflects confirmation, not new changes.

## 7. Risks and mitigations

- R1 Stale/wrong graph misleads the agent → every output carries `source`;
  degraded messages name the missing capability; indexed path revalidates
  mtimes per call.
- R2 Indexing cost on huge repos → cap files (50k), skip vendored/lockfile
  dirs, 30s budget test, background `spawn_blocking`, never on startup path.
- R3 Regex import parsing misses exotic syntax → unresolved imports surface as
  `external`, never silently; per-language fixture tests; tree-sitter follow-up.
- R4 Rename false positives → preview via blast_radius first, temp-copy swap,
  per-file status, typecheck gate, user-confirmed revert.
- R5 Scope creep (model routing, LSP fleet, desktop) → explicit non-goals in
  section 0; any bead adding them is rejected in polish.
- R6 jcode Registry conventions (tool lifecycle, approval, telemetry) →
  Phase 1 bead 1 includes reading `tool/mod.rs` execute/guard paths and mirroring.
- R7 No cross-model review yet → convergence report must flag single-model
  authorship; schedule one `claude-fable-5` review pass before bead conversion.

**Refine re-pass (simplicity lens, 85/100 — converged):** checked the plan
against oversimplification (no features lost — all 4 phases, both tools, all 5
open questions intact) and remaining fat. One cut adopted: `move_symbol`'s
import-line insertion duplicated `rename_symbol`'s batch machinery — merged into
a single `symbol-relocate` core with a `dst` option (`dst: none` = rename),
saving one tool surface while keeping both behaviors; P4a bead text updated at
implementation time. Deferred, not cut: trigram index (Empryo has it; FTS5
prefix matching covers v1 — add only if `find` precision bead fails), call-site
edges (dependents at file granularity suffice for blast radius; symbol-level
call graph is the tree-sitter follow-up's problem). No expansion detected
(450 → ~490 lines, all rubric-driven). Fresh pass surfaced no new structural
gaps — only known nits (R7 cross-model absence, unanswered elicitation).
Rubric: all 11 boxes satisfied or justifiably deferred.

**Security lens additions (pass 5):** graph tools are read-only but their
INPUTS are attack surface. `file`/`pattern`/`query` args: path traversal
rejected (canonicalize + prefix-check against root — symlinks resolved, not
string-compared). `pattern` passed to rg as literal (`-F`) unless explicit
`regex: true` flag; regex args capped at 200 chars + 5s timeout (ReDoS via
catastrophic backtracking is real — `^(a+)+$` class inputs must not hang the
daemon; Rust `regex` crate is linear-time, but rg PCRE fallback is not —
force `--pcre2=false`... bead must verify the flag). `git -C <root> log`
args fully constructed (no shell, no user interpolation — `Command`, never
`sh -c`). DB path: root-joined `.jcode/codegraph.db`, never from user input.
FTS5 queries: parameterized `MATCH ?`, never string-concatenated. Prompt map
block: file paths only, never file CONTENTS (no secret exfil into prompt cache
or logs). Advisory edit line is display-only — it MUST NOT auto-block or
auto-allow edits (approval policy unchanged; destructive-action gates in
`jcode-command-risk` untouched). Rename/move_symbol: same approval tier as
`edit` today, plus mandatory blast-radius preview in the result — no silent
multi-file writes. Telemetry: log counts + latency + source, never file lists
or symbol names.

## 8. Verification

- Unit: per-language import/symbol fixtures; PageRank on synthetic graphs;
  co-change counting on fixture git repos; atomicity tests (inject failure,
  assert restore).
- Perf: reindex budget test; query latency assertions (live < 2s, indexed
  < 200ms on fixture repo).
- Benchmark: 5 fixed tasks, measure input tokens + steps before/after P2;
  gate P4 on ≥30% token reduction or documented why not.
- Regression: full existing tool test suite; kill-switch test
  (`JCODE_CODEGRAPH=0` → all tools `degraded`, zero sqlite opens).

## 9. Open questions (assumptions flagged for refine loop)

1. DB location `.jcode/` vs global cache keyed by repo hash — assumed repo-local.
2. Exact forbidden-path API name in jcode `read` tool — assumed mirrorable.
3. Whether `agentgrep` backends are callable as library functions or only via
   tool execute — Phase 2 bead 1 must verify.

**Subtraction log (pass 6 — what was REMOVED, with justification):**
- Dropped `soul_grep`/`soul_find`/`soul_analyze` as separate tools: agentgrep
  grep/find/outline/trace already cover them; `code_query` composes rather
  than duplicates. (3 fewer tools to register, document, and maintain.)
- Dropped per-tab model routing + 10-role mixture of experts: orthogonal to the
  code graph, doubles the plan size, zero shared code. Follow-up initiative.
- Dropped LSP backend management: Mason-scale server fleet is a product, not a
  feature; regex + (later) tree-sitter covers v1 precision needs per Empryo's
  own fallback chain proving regex is a viable tier.
- Dropped file watcher (`notify`): mtime check per call is simpler, no daemon
  thread, no platform watcher bugs; 60s staleness is acceptable for an advisory
  system (blast radius is consultative, not a lock).
- Dropped DB migrations: `user_version` wipe-and-rebuild — the index is a pure
  cache, rebuildable from source in minutes; migration code would outlive its
  usefulness.
- Dropped auto-revert on typecheck failure: destructive-action policy says the
  user decides; the tool offers one-command revert instead.
- Merged `code_impact` + `code_query` shared logic into `codegraph/live.rs`
  from Phase 1 (not Phase 2): prevents the two-tools-diverge-then-refactor
  rework the draft allowed.
- Deferred tree-sitter to follow-up initiative (was Phase 4+ optional): keeps
  all 15 beads dependency-free except rusqlite-already-present; precision gap
  must be MEASURED before paying grammar-vendoring cost.

4. PageRank damping/iterations tuning — assumed 0.85/20, verify against jcode
   repo shape.
5. TUI surface for blast radius (inline hint vs command) — assumed tool-output
   only in v1.
