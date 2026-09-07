# Plan: Upstream Merge (146 commits) — Safety Tag, Conservative Resolution, Test Gates

> Status: Draft v1. Goal: merge `upstream/master` (146 commits since base
> `fbbde778c`, Aug 31) into local `master` (150 local-only commits), preserving
> all codegraph work, with full-suite green as the gate for channel rebuild.
> Compressed path: bounded merge task, not a product build.

## 0. Key facts (Seed)

- Merge base: `fbbde778c` (Aug 31). Upstream-only: 146 commits (~22k lines,
  269 files: SSH harness, releases to v0.84.0, providers, swarm, memory, MCP).
- Local-only: 150 commits (graph-AI stack + review fixes + lockfile regen).
- Upstream LACKS `crates/jcode-codegraph/` and all `tool/codegraph/*`,
  `code_impact.rs`, `code_query.rs`, `symbol_relocate.rs` entirely. The scary
  "11,888 deletions" in the diff are local-only files shown from upstream's
  perspective. Git merge handles this correctly (add/add, no conflict) as long
  as nobody resolves by taking upstream's tree wholesale.
- Overlap zones (both sides touched): `tool/agentgrep.rs`,
  `tool/communicate*.rs`, `tool/ambient.rs`, `tool/batch*.rs`, possibly
  `prompting.rs`/compaction-adjacent files. These are where real conflicts
  will land.
- Strategy: merge commit (`git merge upstream/master --no-ff`), NEVER rebase
  (150 local commits, shared origin/master history).
- Safety: annotated tag `pre-upstream-merge-<date>` on `33ad44cb3` BEFORE
  merging. Rollback = `git reset --hard <tag>`.
- Tree is clean; no local changes to preserve.

## 1. Resolution policy (conservative, codegraph-preserving)

1. Our new files (codegraph/*, code_impact, code_query, symbol_relocate,
   jcode-codegraph crate, Cargo.toml/lock entries): ALWAYS keep ours.
   Upstream deletion of a path that exists only locally = keep local file.
2. Overlap files (agentgrep, communicate, ambient, batch): resolve hunk by
   hunk, keeping BOTH sides' logic where possible. Upstream refactors +
   our additive tools must coexist. Never `checkout --theirs` a whole file.
3. `CompactionMode::Structural` + `agents.codegraph_map`: re-verify every
   `match self.mode` site post-merge (upstream may add variants or matches).
4. `Cargo.lock`: accept merged union; rebuild regenerates if needed.
5. If a conflict is unresolvable conservatively (semantic clash, not textual):
   STOP, report the file + both sides, await user direction. Do not guess.

## 2. Test gates (in order, stop on red)

1. `cargo fmt --check` on merged tree (scope hygiene only).
2. `scripts/dev_cargo.sh check` on touched crates (compile + match
   exhaustiveness for CompactionMode).
3. Focused graph suites: codegraph crate, codegraph, impact, query, relocate
   (53/53 expected green — our code must survive the merge).
4. FULL workspace suite in an isolated worktree at the merge commit
   (background, log to scratch). Compare failures vs known pre-existing
   clusters (FFF, swarm timing, session persist, provider_matrix).
5. Decision: green except base-proven pre-existing → rebuild + repoint.
   Any red/green (new failure) → fix or report, NO channel update.

## 3. Rebuild + repoint (only if gates pass)

1. Commit lockfile regen if the merge dirties Cargo.lock (as before).
2. `install_release.sh --fast` (current + stable + launcher).
3. Repoint shared-server symlink manually (script does not cover it).
4. Verify `jcode --version` reports the merge commit hash exactly.

## 4. Residual risks

- 22k-line merge: conflict count unknown until attempted (overlap is narrow
  but agentgrep.rs changed 107 lines upstream).
- Full suite takes ~16+ min; upstream may bring new pre-existing failures
  needing base attribution (third worktree at upstream/master if needed).
- No cross-model review (same-model loop); merge conflicts get conservative
  treatment to compensate.
