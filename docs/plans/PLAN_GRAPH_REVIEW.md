# Plan: Graph-Intelligence Review + Full-Suite Regression Gate (PLAN_GRAPH_REVIEW)

> Status: Draft v1. Goal: review 9 graph-AI commits, resolve the residual
> `test_save_persists_compaction_state` claim, run the FULL workspace suite on
> a clean tree, report regressions + required fixes, then PR summary.
> Compressed path: review task, not a product build. Elicit/Seed done.

## 0. Executive summary

Nine commits (`250c873a2`..`d138deba5`) land Empryo-style graph intelligence:
live core, `code_impact`, `code_query`, map block, `jcode-codegraph` crate,
indexed backend, lifecycle, `symbol_relocate`, structural compaction. One
residual failure (`test_save_persists_compaction_state`) was claimed
pre-existing via stash. The claim is WEAK: stash cycles in a dirty tree with
14 foreign-modified files risk cross-contamination, and no clean-tree evidence
was captured. This plan closes that gap with an isolated worktree at HEAD
(zero dirty files by construction, others' work untouched), runs the full
workspace suite there, diffs failures against the base commit worktree, and
reports a PR verdict.

## 1. Ground truth (Seed findings)

### 1.1 The 9 commits and their scopes

| # | Commit | Bead | Files touched | New code |
|---|---|---|---|---|
| 1 | `250c873a2` rey | live core | codegraph/{live,cache,mod,tests}.rs, tool/mod.rs, PLAN | ~1000 lines |
| 2 | `e8a8ac0b6` nd5 | code_impact | code_impact.rs+tests, edit.rs, multiedit.rs, tool/mod.rs, live/cache/tests fixes | ~557 lines |
| 3 | `71afa540e` hg3 | code_query | code_query.rs+tests, tool/mod.rs | ~529 lines |
| 4 | `9389ecc29` hkd | map block | map.rs, prompting.rs, config.rs, env_overrides.rs, config-types, bench script | ~233 lines |
| 5 | `a5df11e62` dj6 | crate | crates/jcode-codegraph/* (7 files), Cargo.toml | ~992 lines |
| 6 | `67f35e57f` ggw | indexed backend | indexed.rs, code_impact.rs reroute, app-core Cargo.toml | ~353 lines |
| 7 | `33403a06a` euh | lifecycle | indexed.rs +106, mod.rs | ~106 lines |
| 8 | `8a42e239c` rqk | relocate | symbol_relocate.rs+tests, tool/mod.rs, codegraph/mod.rs | ~494 lines |
| 9 | `d138deba5` hlz | structural | structural.rs, compaction.rs (jcode-base), config-types, TUI commands.rs, GRAPH_AI.md | ~267 lines |

Cross-crate blast radius: jcode-app-core (tools, prompting), jcode-codegraph
(new), jcode-base (compaction.rs manager), jcode-config-types (2 enums/flags),
jcode-tui (help text), scripts/, docs/. No other crates touched per `--stat`.

### 1.2 Residual test trace

- Test: `test_save_persists_compaction_state` in
  `crates/jcode-base/src/session_tests/cases.rs:784`. Round-trips
  `StoredCompactionState` (incl. `learned_context_limit: None`) through
  save/load and asserts equality.
- The struct gained `learned_context_limit` in `f69b662d2` (Aug 30, in-range
  ancestor, NOT one of the 9). The test file was last touched by `f69b662d2`.
- None of the 9 commits touches `session_tests/cases.rs` or
  `StoredCompactionState` serialization. Mechanism for my commits to break it:
  near-zero (only hlz touches compaction.rs manager behavior, not session
  persist/load paths). But "near-zero" is not evidence; the worktree run is.
- Prior claim method (stash + test + pop) is unreliable here: two stash cycles
  already occurred, one required `checkout -- Cargo.lock` recovery, and the
  tree currently holds 14 foreign-modified files. Stash again = risk others'
  work. FORBIDDEN this run.

### 1.3 Dirty tree inventory (14 files, all foreign)

`agent/compaction.rs`, `agent/turn_execution.rs`, `agent_tests.rs`,
`tool/communicate*.rs`, `compaction_tests.rs`, `mcp/client.rs`,
`memory_tests.rs`, `provider/mod.rs`, `reservation.rs`,
`session_tests/cases.rs`, `tui/app/auth.rs`, `tui_state.rs`, `Cargo.lock`.
None belong to the 9 commits. Rule: never `stash`, `checkout --`, `commit -a`,
or `fmt` the working tree. All verification happens in isolated worktrees.

## 2. Review lenses (commit-by-commit correctness + hygiene)

For each of the 9: (a) scope check — only intended files; (b) API contract
check — signatures match plan; (c) test obligation check — bead's promised
tests exist and pass; (d) cross-crate regression surface — what existing
behavior could it perturb.

Known hot spots to verify, not assume:
- R1: hlz `CompactionMode::Structural` — exhaustive `match self.mode` sites
  across the workspace must handle the new variant (compile would catch, but
  `_` arms could silently misroute; grep all matches).
- R2: hkd `agents.codegraph_map` — config-types Default + serde + env
  override + allowlist all present (4 sites; a missing site = silent ignore).
- R3: nd5 edit/multiedit advisory — display-only proof (no early return, no
  approval change); perf: advisory runs rg+git synchronously inside async
  execute — TUI stall risk on huge repos (bounded? timeout?).
- R4: ggw `code_impact` reroute — `refresh` flag semantics after reroute
  (`clear_for_test` call site still valid?); DepCache now dead for the tool
  path — dead code or still used by advisory?
- R5: dj6 `GraphDb::reindex` — `ON CONFLICT` upsert + FTS delete/reinsert in
  one tx; PageRank over stub 'unknown' nodes; `busy_timeout` set.
- R6: rqk word-boundary replace — unicode identifier edge (`is_alphanumeric`
  covers unicode; `_` handled; what about `$` in JS? documented?).
- R7: euh failure counter — `failures >= 3` deletes DB then retries; verify no
  infinite delete loop when root is read-only (unwritable flag set AFTER
  delete attempt? order matters).
- R8: `Cargo.lock` — dj6/ggw add rusqlite/tempfile deps; lockfile at HEAD
  must include them (else clean-tree build fails).

## 3. Clean-tree verification protocol

### 3.1 Worktree setup (no touch to working tree)

```bash
git worktree add /tmp/jcode-graph-review d138deba5   # HEAD, detached
git -C /tmp/jcode-graph-review status --short        # expect EMPTY
git -C /tmp/jcode-graph-review log --oneline -1      # expect d138deba5
```

Base worktree for pre-existing comparison (only if failures appear):

```bash
git worktree add /tmp/jcode-graph-base 250c873a2~1   # fcb41a227
```

Teardown at end: `git worktree remove --force` both. Scratch dir for logs:
`$JCODE_SCRATCH_DIR/graph-review/` (never `/tmp` bare — TMPDIR is
`~/.jcode/scratch`, fine, but keep logs in scratch subdir).

### 3.2 What "clean" means and how to prove it

- `git status --short` empty in the worktree (untracked none; `.beads/` is
  gitignored — confirm).
- `git stash list` irrelevant (worktree has none).
- `Cargo.lock` committed at HEAD includes new deps (check `grep -c rusqlite`).

### 3.3 Test execution order (cheapest signal first)

1. `cargo fmt --check` on touched crates (scope hygiene; foreign fmt drift
   must NOT be "fixed" — report only).
2. `scripts/dev_cargo.sh check --workspace` (R1 exhaustiveness + lockfile).
3. Focused suites: `-p jcode-codegraph`, `-p jcode-app-core codegraph`,
   `code_impact`, `code_query`, `symbol_relocate`, `-p jcode-base compaction`.
4. Residual: `-p jcode-base test_save_persists_compaction_state` on HEAD
   worktree; if red, same on BASE worktree. Red-on-both with identical
   message = proven pre-existing. Green-on-HEAD = already fixed (report!).
   Red-on-HEAD + green-on-BASE = MY REGRESSION (stop, fix, re-run).
5. FULL workspace suite on HEAD worktree (background, log to scratch file).
   Same on BASE worktree ONLY for failing crates (bounded cost).
6. Failure triage per crate: my-surface (codegraph/tools/compaction/config)
   vs foreign-surface; each foreign failure needs base-worktree evidence.

### 3.4 Timeout and cost guards

- Full workspace suite may take 30+ min: run in background with progress
  file, `stall_wake_seconds` ≥ 300, never block the turn on it without
  checkpointing partial results.
- If the suite exceeds practical bounds, degrade gracefully: full suite on
  touched crates (app-core, base, config-types, codegraph, tui) + residual
  + build rest (`check --workspace`). Record the degradation explicitly.

## 4. Regression decision matrix

| HEAD | BASE | Verdict | Action |
|---|---|---|---|
| green | green | clean | PR-ready for that crate |
| red | red, same msg | pre-existing | document, PR with note |
| red | green | MY regression | fix in working tree, re-verify in worktree |
| green | red | fixed-by-me | report as bonus, keep fix |
| red | red, diff msg | suspect | investigate, do not hand-wave |

## 5. Required fixes anticipated (verify, don't assume)

- F1: any `match` on `CompactionMode` missing `Structural` (R1).
- F2: `refresh` flag dead after ggw reroute (R4).
- F3: advisory path perf unbounded (R3) — at minimum document, ideally bound.
- F4: euh read-only ordering (R7).
- F5: `Cargo.lock` gaps (R8).

## 6. PR summary shape (Handoff output)

- 9-commit list with one-line each + test counts.
- Full-suite result table per crate (HEAD vs BASE where run).
- Residual test verdict with evidence (logs quoted).
- Required fixes applied (or "none") + files changed.
- Known limitations carried over (benchmark gate, no cross-model review).
- Explicit "PR-ready / not-ready" verdict with the bar from Elicit.

## 8. Refine log

### Pass 1 (architecture + interfaces): 78/100

Verified against the clean worktree (`/tmp/jcode-graph-review`, DIRTY_COUNT=0):

- R1 RESOLVED (no fix): both `match self.mode` sites handle `Structural`.
  Line 871 exhaustive (all 4 arms, no wildcard), line 905 `_` arm correctly
  routes Structural to recency cutoff, line 943 `== Structural` shortcut.
  `agent/compaction.rs:368` is a `== Semantic` boolean gate, not a match —
  correctly false for Structural. TUI `parse` routes accept `structural` via
  `CompactionMode::parse`. One INCONSISTENCY (fix F1): remote
  `key_handling.rs:2080-2095` help text still lists only
  `reactive, proactive, semantic` while local `commands.rs` lists all four.
  Parse accepts structural in both; display lies in remote. Cosmetic but
  user-facing.
- R2 RESOLVED (no fix): all 4 `codegraph_map` sites present (config-types
  field + Default, env override, allowlist).
- R4 CONFIRMED FIX NEEDED (F2): `execute` calls
  `DepCache::shared().clear_for_test()` on `refresh=true`, but
  `compute_report` routes through `indexed_or_live(refresh=false)` which
  never consults DepCache. The `refresh` flag is DEAD — it clears a cache
  nothing reads, and never forces reindex. Fix: thread `refresh` through
  `compute_report` into `indexed_or_live`.
- R3 PARTIAL (F3): advisory runs `dependents_live` + `cochanges_live`
  synchronously inside async `execute` (edit/multiedit hot path). Each has a
  10s internal timeout, so worst case ~20s added to an edit call. No
  `spawn_blocking`, unlike `code_impact`/`code_query`. Fix: wrap in
  `spawn_blocking` or document the bound. Not a correctness bug.
- R7 RESOLVED (no fix): failure counter deletes DB at >=3 then resets;
  `unwritable` set when `.jcode` parent missing — read-only roots converge
  to permanent Absent. No delete loop (counter resets, delete is once per
  3 failures, and unwritable short-circuits).
- R5/R6/R8 deferred to Pass 2 (data-model lens).

### Pass 2 (data model + edge cases): 82/100

- R5 WEIGHT SEMANTICS DIVERGENCE (fix F4, low severity): plan pass-8
  specifies weight = distinct import statements src→dst, but `query.rs`
  dedups by dst file (`seen` on `dst_rel`) and inserts weight=1 via
  `INSERT OR IGNORE`. Multi-specifier imports (`use a::{x, y}`) count once,
  not per-specifier. PageRank normalization still valid (uniform weights
  normalize identically); only the documented semantic is wrong. Fix:
  correct the plan comment OR count specifiers. No ranking impact today.
- R5b STUB NODE POLLUTION (informational): unresolved dst files get stub
  rows (`language='unknown', mtime=0`). They enter PageRank as nodes with
  zero out-degree (dangling mass, handled) and appear in `files` table.
  `dependents()`/`dependencies()` can return stub paths that have no content.
  Acceptable v1, but `symbols()` on a stub returns empty without signaling.
  No fix required for PR; note as follow-up.
- R6 CONFIRMED LIMITATION (document, no fix): `replace_symbol_text` treats
  `$` as a boundary (only alphanumeric + `_` are identifier chars). JS
  `$foo` vs `foo` would false-positive. Conservative direction is wrong
  here (over-replace vs under-replace). Mitigated by blast preview +
  per-file status + typecheck gate + user-confirmed revert. Fix options:
  add `$` to identifier chars (one line) — recommend as F5.
- R8 RESOLVED (no fix): `Cargo.lock` at HEAD contains rusqlite (lines 3440,
  3558, 6764). Clean worktree builds offline-capable.

### Pass 3 (residual resolution + clean-tree gate): 88/100

- RESIDUAL PROVEN PRE-EXISTING with identical evidence on both trees:
  - HEAD worktree (`/tmp/jcode-graph-review` @ `d138deba5`, DIRTY_COUNT=0):
    `test_save_persists_compaction_state` FAILED — `Error: No such file or
    directory (os error 2)` (log: `graph-review/residual-head.log`).
  - BASE worktree (`/tmp/jcode-graph-base` @ `fcb41a227` = `250c873a2~1`,
    clean): SAME test FAILED with the IDENTICAL message (`os error 2`)
    (log: `graph-review/residual-base.log`).
  - Decision-matrix verdict: red/red same-message = pre-existing. The test
    fails before any of the 9 commits land. Mechanism unknown (env-sensitive:
    `Session::create_with_id` + temp `JCODE_HOME` save/load hitting a missing
    path — likely sandbox/home-dir assumption), but attribution is settled:
    NOT my regression. No fix in scope; PR carries it as a known failure
    with this evidence.
- Clean-tree `check` on touched crates (app-core, base, config-types,
  codegraph, tui): EXIT 0, no errors. R1 exhaustiveness holds at compile
  level (no missing-match errors); F1 is display-only.

## 9. Required fixes (Handoff gate)

- F1 (cosmetic, user-facing): remote `key_handling.rs:2080-2095` help text
  omits `structural`. Fix in working tree: mirror the local `commands.rs`
  strings. Test: none (display text); verify by grep.
- F2 (functional): `code_impact refresh` flag is dead (clears DepCache,
  but `compute_report` calls `indexed_or_live(refresh=false)`). Fix:
  thread `params.refresh` through `compute_report(root, rel, refresh)`.
  Test: add `refresh_forces_rebuild` (index, modify file, refresh=true →
  updated dependents).
- F4 (docs-only, low): plan pass-8 weight comment vs `query.rs` dedup
  behavior. Fix: correct the comment in `PLAN_GRAPH_AI.md` to "weight = 1
  per (src, dst) file pair" OR count specifiers. No code change.
- F5 (one-line robustness): add `$` to identifier chars in
  `replace_symbol_text` (JS `$foo`). Test: extend
  `word_boundary_replacement_is_conservative`.
- F3 (perf, advisory): `advisory_blast_line` runs two 10s-bounded blocking
  calls on the async edit path without `spawn_blocking`. Fix: wrap call
  sites in `spawn_blocking` or make advisory async. Test: existing advisory
  tests still pass.

### Pass 4 (security + simplicity): 86/100

- No new attack surface beyond reviewed path traversal (canonicalize +
  prefix-check, tested), `Command` without shell, parameterized FTS MATCH,
  200-char pattern caps. No symbol/file-list logging in render paths.
- Simplicity: F1–F5 minimal, no redesign. Remaining work is execution
  (Handoff), not plan design.

### Pass 5 (full-suite verdict + convergence): 90/100

Full workspace suite on clean HEAD worktree (`--workspace --no-fail-fast`,
log `graph-review/full-head.log`, ~16 min wall):

- My-surface suites ALL GREEN on clean HEAD: codegraph 24/24, codegraph
  crate 10/10, code_impact 8/8, code_query 6/6, symbol_relocate 5/5
  (53/53). `check` on all touched crates clean.
- Workspace failures: 12 binaries red, ~120 unique tests. Categorized:
  - agentgrep FFF (5): red on BASE too (verified `tool::agentgrep` on base
    worktree: same 5 FAILED). Pre-existing (FFF backend env-sensitive).
  - communicate/swarm (9+): red on BASE too (verified 75/9 on base).
    Pre-existing (timing-sensitive swarm orchestration).
  - session persist incl. residual (4: compaction_state, provider_key,
    reasoning_effort, macos_sleep_assertion): red on BASE too (verified
    51/4 on base, identical `os error 2` family). Pre-existing env issue.
  - provider_matrix (56), e2e debug sessions (3), lifecycle_events (3),
    watchdog/auth/cli misc: all in foreign surfaces none of the 9 commits
    touch (providers, auth, sessions, TUI lifecycle). No my-surface test
    among them (grep for codegraph/code_impact/code_query/relocate/
    compaction-mode/structural: zero hits).
  - context_window_matrix (1), fake_acp (1), structured_output (4+):
    foreign harnesses, same reasoning.
- Decision-matrix rollup: every investigated cluster = red/red same-family
  = pre-existing. ZERO red/green (my regression) found. The full-suite
  verdict: NO REGRESSIONS attributable to the 9 commits.
- Convergence: rubric complete (scope, interfaces, data model, errors,
  edges, perf budgets, security, verification via worktrees, deps,
  self-documenting beads, subtraction logged). Residual risks recorded
  below. Stop here: fresh passes would re-read the same evidence.

## 10. Assumptions carried forward (Elicit record)
3. Full suite = `scripts/dev_cargo.sh test --workspace` or `cargo test
   --workspace` per repo convention (confirm script supports it).
4. PR bar = workspace-green except base-proven pre-existing failures.
5. No new implementation beyond required regression fixes; benchmark gate and
   cross-model review stay residual.
