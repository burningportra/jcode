# tgrep spike results, 2026-09-08

## Decision for both requested surfaces

**Keep the current @ walker/cache. Keep the current agent-search routes as the default.** tgrep 1.0.4 is promising for an optional literal, filenames-only content-search accelerator, but this spike does not justify replacing either existing surface now.

This is a completed execution spike, not a shipped production tgrep adapter. The user selected both surfaces and delegated implementation choices. I chose a checksum-verified executable, private indexes and short-lived owned servers rather than global installation or a persistent service. No PATH, provider credentials, user search configuration or production search code changed.

Confidence in retaining the @ implementation for this release: **97%**. Confidence in retaining existing default agent-search routes until compatibility/freshness gates pass: **96%**. Confidence that a narrowly scoped tgrep backend could help selected larger workloads: **80%**, not a statistical estimate or a measured win over FFF.

## What actually ran

- tgrep **v1.0.4**, source commit `75894b124c4e53586032d7a41524168dfa02f480`, MIT license. Apple Silicon archive SHA256: `9ef13569d6725bb50497671506c914aaf6602fb0631810c8d100214498497ec8`. Hash verified before execution.
- macOS arm64, current Jcode repository read-only, plus a generated **5,000-file, 20,480,000-byte** corpus. This is not a 500,000-file monorepo benchmark.
- Five warmups and twenty measured calls per engine/case, alternating engine order. Full normalized result equality checked before and during timing. No speedup is reported for mismatching cases.
- CLI timing includes process startup and server IPC when used. Filename discovery uses an explicitly labeled ripgrep-listing proxy, not the private Rust TUI walker. Cached substring filtering is measured separately, not equated with subprocess discovery. No new TUI adapter or insertion behavior was tested or shipped.
- Final run finished in approximately **51 seconds**, with no execution errors. Negative parity results are findings, not ignored test failures. Raw timing samples and reduced structured observations are in [the evidence file](2026-09-08-tgrep-spike-evidence.json). Full command construction is in [the runnable harness](../../scripts/tgrep_spike.py).

## Representative timings

Numbers are **median / p95 milliseconds**, from the final run. All four rows passed exact parity under their stated policies.

| Workload | ripgrep | tgrep |
|---|---:|---:|
| repo: literal paths-only content | 24.97 / 26.38 | 10.68 / 11.60 |
| repo: @-compatible discovery proxy | 13.33 / 14.81 | 19.62 / 20.84 |
| generated: literal paths-only content | 96.62 / 97.74 | 25.41 / 25.99 |
| generated: @-compatible discovery proxy | 14.37 / 15.95 | 22.17 / 23.47 |

Warm literal paths-only content search was approximately **2.34x faster** than rg on Jcode and **3.80x faster** on the generated corpus. @-compatible discovery was slower in both datasets. Default indexed filename listing was somewhat faster, but omits hidden files required by the current @ policy, so it is not a valid replacement comparison.

The repository index built in approximately **1.80 seconds** and occupied **27.72 MB**. The generated index built in **142 ms** and occupied **2.87 MB**. Observed peak combined index storage was **30.60 MB** and peak monitored server RSS **69.16 MB**. These are periodic measurements, not guaranteed hard limits. Initial index construction is not an OS cold-cache benchmark.

### Existing FFF baseline

Current source already has an FFF backend. Its supported route is narrow: literal case-sensitive `grep`, `paths_only=true`, without path/glob/type/hidden/no-ignore overrides. Other requests retain the linked implementation. Feature availability does not imply every search is routed to FFF.

The existing `benchmark_warm_fff_against_linked_agentgrep` exercised the **real public Tool API**, asserted FFF readiness/backend metadata, and checked exact output equality for `ToolOutput`. Each run used five warmups and fifty measured calls:

| Run | FFF p50 / p95 ms | Linked p50 / p95 ms |
|---|---:|---:|
| First | 21.18 / 38.08 | 25.54 / 32.37 |
| Repeat | 8.97 / 10.36 | 22.42 / 23.53 |

The FFF public-tool harness used the selfdev/unoptimized profile; tgrep used its official release executable. Run-to-run variation is substantial. These public-tool measurements are not directly comparable to tgrep CLI measurements, and do **not** establish that tgrep beats FFF. No tgrep adapter was put through the public Tool API.

## Correctness findings

### 1. Preserving @ behavior defeats filename indexing

Pinned `search.rs:504` requires both `!hidden` and `!no_require_git` for indexed filename listing. Current @ includes hidden files and applies ignore rules outside Git repositories. Supplying those options takes tgrep's walking path. Default indexed listing omitted four required hidden paths in the controlled fixture.

With the appropriate traversal flags and explicit adapter exclusions, Git, genuine non-Git and nested-root fixture membership matched the expected @ set. Binary and empty files remained selectable. Ignored tracked paths, symlinks and control-character names were excluded by the intended adapter policy. Direct CLI output and adapter-filtered output are recorded separately, not silently conflated.

The `-g !.git` spelling does not prune `.git` descendants in this release's listing path. The fixture exposed five Git-internal paths. `-g !.git/**` excluded them. Repository benchmark commands used the latter spelling.

### 2. Server JSON loses byte offsets unless explicitly requested

The synthetic two-match fixture demonstrates:

| Output path | Absolute offsets | CRLF preserved |
|---|---|---|
| rg `--json` | 11, 35 | Yes |
| tgrep server `--json` | 0, 0 | No |
| tgrep server `--json -b` | 11, 35 | No |
| tgrep `--json --no-index` | 11, 35 | No |

Pinned `search.rs` requests position details for byte-offset/max-column options but not JSON alone. The server omits offsets without that request, and the client defaults absent offsets to zero. `-b` is a possible adapter workaround for offsets, but does not restore CRLF bytes. The harness retains strict mismatches rather than normalizing them into passes. General excerpt/regex replacement cannot be called drop-in compatible.

### 3. Glob semantics differ

The controlled `-g '*.txt'` content query returned nine rg matches versus four tgrep matches under the default policy, and nine versus five under the @-compatible policy. These are actual missing matches, not just formatting differences. Any optional adapter must reject or correctly translate unsupported membership semantics rather than treating glob filtering as universally equivalent.

### 4. Readiness is not immediate filesystem freshness

A completed offline index omitted a newly created file and still listed a deleted path. After the owned server stopped, a new file was likewise absent from indexed filename and content results. The existing sentinel still matched, so a successful probe does not prove completeness.

With a ready active watcher, create/edit/delete converged in approximately **241 / 236 / 81 ms** on the final fixture run. Both filename and content results were checked. This is eventual convergence, not a guarantee that an agent can write a file and immediately search it without missing results. Default “no matches” from a stale index cannot be authoritative.

## Verification ledger and remaining limits

| Obligation | Observed check/result |
|---|---|
| Cover both requested surfaces | Executed filename membership/discovery/cache proxy and content-search parity/timing cases, with separate adoption decisions |
| Git and nested workspace membership | Explicit expected sets, raw engine comparisons and adapter-filtered comparisons passed under compatible flags |
| True non-Git behavior | Detected ancestor Git contamination in scratch; final 421-byte fixture used real macOS user temp and verified no ancestor `.git`; adapted membership passed |
| Invalid UTF-8 filenames | macOS rejected fixture creation; **not exercised**, recorded in fixture notes |
| Literal/regex/case/no-match | Controlled cases passed; glob and JSON representation mismatches explicitly retained |
| Filesystem updates | Offline/stopped-index misses reproduced; active watcher create/edit/delete eventually matched both content and filenames |
| Fair latency claims | Raw samples retained, engine order alternated, parity-gated ratios; mismatching broad/offset cases excluded from speed claims |
| Current FFF behavior | Existing real Tool benchmark passed twice with backend/readiness and equality assertions |
| Resource/process safety | Time/output/index/RSS budgets monitored; forced timeout/output-budget checks aborted and reaped owned processes; final server/process and workspace checks passed; no production source writes |
| Reproducibility | Pinned release/hash, executable harness, structured evidence, exact commands below |

Not exercised: Windows/Linux integration, 64 MiB boundary files, crash-during-index-build recovery, lost watcher events, branch-switch storms, truly cold OS caches, a production tgrep adapter, or TUI integration. Indexed file membership and optional JSON fixes require further acceptance tests before adoption. These limits do not invalidate the negative default-adoption decision.

The first run's non-Git fixture was inside an ancestor Git repository and is **withdrawn as non-Git evidence**. A second run correctly marked it blocked because inherited TMPDIR also pointed into scratch. The final invocation explicitly selected the real macOS user-temp directory for the tiny synthetic fallback. All indexes, captured output and large generated data remained under JCODE_SCRATCH_DIR.

## Follow-through if tgrep adoption is pursued

Do not add another agent-facing search tool or replace outline/trace. A future backend should sit behind the existing search contract and own its lifecycle. The first eligible slice is literal paths-only content search, with explicit unavailable/rebuilding/stale states and current-backend fallback. It must compare against FFF through the same public Tool API, including initial indexing cost and immediate post-write queries. JSON/glob/hidden/large-file semantics need individual eligibility gates or proven translations. Do not add automatic installation or default service startup before those checks pass.

For @, retain the existing in-process catalog and cached filtering. Revisit only if a backend can satisfy hidden/non-Git membership without giving up its indexed path and demonstrate a real TUI improvement.

Independent reviewer clover reviewed the plan and compact measurements and supported retaining both defaults. No cross-model review was claimed. This bounded spike used executable todos instead of a speculative implementation bead tree.

## Reproduce

Obtain the pinned archive from the upstream v1.0.4 release and verify the hash above. The harness checks its version, does not install it, and refuses to overwrite an existing report.

```bash
# macOS: TMPDIR affects only the tiny synthetic non-Git fallback.
TMPDIR="$(/usr/bin/getconf DARWIN_USER_TEMP_DIR)" \
python3 scripts/tgrep_spike.py \
  --tgrep "$JCODE_SCRATCH_DIR/tgrep-v1.0.4-spike/tgrep" \
  --repo "$PWD" --scratch "$JCODE_SCRATCH_DIR" \
  --output "$JCODE_SCRATCH_DIR/tgrep-new-report.json" --samples 20

cargo test --profile selfdev -p jcode-app-core --lib \
  benchmark_warm_fff_against_linked_agentgrep \
  -- --ignored --nocapture --test-threads=1
```

The current harness is intentionally macOS-specific in its rg/ps paths. Source fixtures contain synthetic text only; repository contents are compared in memory and are not copied into evidence artifacts. The reduced evidence omits repetitive command records while retaining their count and the SHA256 of the full local report.
