# Dual-surface tgrep spike

## Elicitation and scope

The user explicitly selected BOTH @ file selection and agent code search, delegated implementation choices, and asked to resume this spike after the invalid-request fix. That fix is active at b60d5f50f and its provider suite passed129 tests with3 live tests ignored. The earlier evaluation-only document does not complete this clarified request.

Run a bounded, reproducible spike before production adoption. Choose isolated, Jcode-owned executable/index/process resources in scratch rather than global installation or persistent background services. This settles the immediate lifecycle choice without another user question. No production backend replacement is promised before parity evidence.

## Seed and current state

Current source has FFF0.10.5 integrated in agentgrep and enabled as a compiled feature. Its runtime routing is controlled separately by search configuration. The @ picker still uses an ignore-aware background walker. Thus compare against the real supported FFF paths as well as rg rather than pretending FFF is absent. Existing verification document has historical FFF timings, which are not measurements from this spike.

Pin the latest stable tgrep release v1.0.4 at commit75894b124c4e53586032d7a41524168dfa02f480. Apple Silicon release archive SHA256 is9ef13569d6725bb50497671506c914aaf6602fb0631810c8d100214498497ec8 as published by GitHub release metadata. Upstream main observed10c73887b6326f395afdf2853188dd398ca9bdfc is newer, so do not attribute untested main behavior to the release.

## Draft experiment

1. Download the pinned binary into scratch, verify hash before execution, extract only the executable, inspect release-specific help/source. No PATH changes.
2. Build a temporary repository with normal, hidden, ignored, untracked, binary, empty, spaced-path and nested files. Compare @ membership against the current ignore-walker policy and tgrep listing. Compare literal, regex, case, glob and no-match search against rg with normalized paths/results. Test ready versus stale local index, immediate create/edit/delete and watcher convergence.
3. Benchmark repeated filename and content requests on this repo with index outside repo and on a larger generated fixture. Record initial build, first invocation, median/p95 warm calls, index size and process memory where available. CLI-to-CLI is not public-tool latency: label it accordingly. Reuse current FFF public-tool benchmark for its supported scope if feasible.
4. Every subprocess gets a timeout. Own and terminate only this spike's server. Never trust empty results during build as a complete catalog. If membership flags bypass the index, record that rather than hiding it behind warm timings.
5. Commit harness and report with observed pass/fail cases, reproduction commands, limits and decision for EACH surface. A failing parity or negative performance result is a valid spike outcome, not permission to silently change user-visible behavior.

## Review and convergence gate

Require independent read-only review of scope, resource safety and honest baseline comparisons before executing downloaded code. Fold gaps into this plan. This is a compressed bounded goal, with explicit executable todos instead of beads. No multi-session implementation program until results justify one.

## Acceptance and handoff

Done means executed evidence for both filename listing and content search, plus concrete recommendation for integrating tgrep or retaining the existing path for each. Full production integration, UI changes and managed installer lifecycle are separate follow-through if supported by results. No invented speedup, no claim that CLI benchmarks validate a TUI adapter, and no blind replacement of outline/trace modes. Distill lessons in the final report and update stale earlier assessment links.

## Converged reviewer obligations

Independent reviewer clover identified resource, oracle, baseline and readiness gaps. They are incorporated here. Plan is approved for bounded execution, not production rollout. No cross-model review was claimed.

- Numeric budgets: per query30 seconds, initial index120 seconds, total run10 minutes, generated corpus5,000 files/32 MiB, captured stdout16 MiB per command, index disk2 GiB, server RSS1 GiB checked periodically. Use25% indexing CPU and256 MiB index-memory setting if supported. These are monitored budgets, not OS hard real-time guarantees. On timeout/budget violation terminate the owned process group, wait briefly then kill/reap; always clean up on exceptions.
- Fixtures define explicit @ expected membership: hidden/empty/binary files admitted, `.git`, ignored paths, symlinks, control-character and non-UTF8 paths omitted, subtree root respected, non-Git ignore rules applied. Tracked files matching ignore rules remain omitted under current @ behavior. Include case-insensitive substring filtering, prefix/suffix preservation and cached-list filtering semantics. CLI benchmarks do not validate UI insertion; existing real-TUI checks remain the UI baseline and no adapter is being shipped in this spike.
- Filename measurement separates initial discovery from cached filtering. Use explicitly labeled rg-listing proxy plus cached path filtering if invoking the private TUI walker directly is impractical. Do not compare each tgrep subprocess with cached in-process @ filtering as though both were the same operation. FFF is eligible only for literal case-sensitive paths-only grep without path/glob/type/hidden/no-ignore flags. Other content cases compare with linked/ripgrep behavior, not unsupported FFF routes.
- Ready means parsed server status `indexing=false` and active watcher for mutation tests, plus a known fixture sentinel visible. Server discovery alone is insufficient. Initial partial/empty results and missed writes are findings. Check post-index offline create/delete, live create/edit/delete convergence under a finite5-second budget, and stopped-server fallback behavior. Never promote empty/incomplete data into an authoritative no-match result.
- Use current repository read-only, with all index/output files outside it. Record same-query parity before reporting latency. Keep raw timing samples and selected commands, but do not store repository file contents or credentials in report artifacts.


## Execution outcome and distillation

Executed checksum-pinned release on both surfaces. Final run completed with no execution errors and preserved negative parity findings. See [the results and requirement ledger](../reviews/2026-09-08-tgrep-spike.md) and structured raw-sample evidence. Retain both existing defaults: @-compatible discovery is slower and bypasses the index; paths-only content acceleration is promising but not proven better than existing FFF, while JSON/glob and freshness mismatches block broader adoption. No production adapter was shipped.

NonGit validation required a 421-byte synthetic fixture under the real macOS user temp because the user's home and inherited TMPDIR were inside a Git tree. All large corpora, indexes and output stayed in scratch. The first contaminated observation was withdrawn, the next was marked blocked, and the final run verified no ancestor Git. Invalid UTF-8 filename creation was refused by macOS and remains explicitly untested.

Independent plan and compact-result review supported these decisions. Timeouts/output budget were separately exercised with owned subprocesses and cleanup assertions. The concrete lesson is to measure membership and freshness equivalence before interpreting indexed query speed. Negative adoption evidence is a completed spike, not an unfinished integration.
