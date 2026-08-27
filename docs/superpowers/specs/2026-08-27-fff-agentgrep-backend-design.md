# FFF-backed AgentGrep search

Status: Phase 1A implemented, rollout default off
Date: 2026-08-27

## Implementation evidence

Phase 1A is implemented behind the `fff-search` Cargo feature and the typed
`[search].fff_backend = off | shadow | prefer` setting. The public tool keeps its
existing name and schema. Production defaults to `off`; ready eligible requests
can be compared in `shadow` or routed in `prefer`, while cold and unsupported
requests use linked AgentGrep with machine-readable fallback metadata.

Public-tool tests against the published `fff-search 0.10.5` crate prove ready
backend selection, byte-for-byte parity, zero matches, ordering, case and literal
whitespace behavior, shadow parity, unsupported-request fallback, and create,
modify, rename, and delete watcher updates. The existing AgentGrep test module,
formatting, config tests, and `jcode-app-core --no-default-features` check pass.

Isolated real-daemon acceptance with disposable `JCODE_HOME`, runtime directory,
socket, and Git fixtures observed cold linked fallback followed by ready FFF
routing with the same output and generation. The live watcher reflected create,
modify, rename, and delete operations. Regex, excerpt, path, glob, type, hidden,
and no-ignore requests stayed on linked AgentGrep with their expected fallback
reasons. A second repository replaced the inactive index and advanced the
generation, shadow mode reported `shadow_parity = true`, and
`max_fff_indexes = 0` forced the disabled fallback. Focused registry tests also
prove the two-search process cap, active-index replacement protection, and
filesystem-root and home-root rejection.

On the Jcode repository on macOS arm64, 50 measured calls after 5 warmups through
the same `AgentGrepTool::execute` boundary observed linked p50/p95 of
36,911/65,796 microseconds and FFF p50/p95 of 31,539/62,648 microseconds. A local
self-dev binary comparison measured 337,556,368 bytes with FFF and 326,804,144
bytes without it, a 10,752,224-byte increase. `cargo tree -e features` confirms
the stable crate's default `ripgrep` feature and no `zlob` feature. Cross-platform
release linking and release-profile size remain CI acceptance items because this
machine has only the macOS arm64 and wasm targets installed.

## Summary

Jcode should keep the `agentgrep` tool and its agent-facing contract while replacing repeated file walking and content-search process work with FFF's warm in-process Rust index. FFF becomes the preferred backend for compatible `grep` and `find` requests. AgentGrep's existing linked implementation remains responsible for `outline`, `smart`, and `trace`, and remains a correctness fallback until parity is proven.

This is a backend migration, not a tool rename. Models keep one search tool, current prompts keep working, output remains parse-compatible with Jcode's context-exposure logic, and unsupported requests degrade to the existing implementation rather than failing.

## Problem

Jcode's `agentgrep` tool currently delegates all modes to the linked `agentgrep` crate. Its ordinary `grep` path shells out to ripgrep, while `find` walks the repository again. The tool correctly moves this blocking work off Tokio workers and promotes searches to background after five seconds, but repeated calls still pay cold filesystem and process costs.

Jcode is a long-lived process that searches the same repository many times. FFF is designed for that workload:

- one background scan instead of one walk per query
- a live filesystem watcher
- in-process plain, regex, and fuzzy content search
- frecency-aware file ranking
- typed pagination and match metadata
- cached Git status
- Rust library integration with no external service

The migration must not discard AgentGrep's higher-level value. `outline`, `smart`, `trace`, context exposure, exact-file scoping, bounded output, background promotion, and stable tool schemas are part of Jcode's behavior.

## Goals

- Make FFF the default backend for repeated compatible `grep` and `find` calls.
- Preserve the public `agentgrep` tool name, input schema, output grammar for each migrated mode, and current exposure behavior.
- Keep `outline`, `smart`, and `trace` behavior unchanged initially.
- Initialize indexes lazily and reuse them across sessions sharing a repository root.
- Fall back safely during cold start, unsupported option combinations, watcher failure, index failure, or resource pressure.
- Bound memory, index count, scan time, output size, and background work.
- Prove correctness parity before claiming performance improvement.
- Use a stable crates.io release with the pure-Rust default walker. Do not add the optional Zig-powered `zlob` build requirement.

## Non-goals

- Replacing AgentGrep's semantic ranking or trace DSL in the first migration.
- Renaming the tool to `fff`, `ffgrep`, or `fffind`.
- Exposing FFF's MCP server or spawning a separate FFF process.
- Persisting frecency or query history in the first slice.
- Indexing the user's home directory or filesystem root.
- Removing the linked `agentgrep` dependency before all retained modes have an owner.
- Claiming FFF is faster in Jcode before repository-level acceptance benchmarks pass.
- Enabling fuzzy content fallback for literal `grep` calls without an explicit product decision.

## Approaches considered

### A. Replace AgentGrep completely with FFF

Use `fff-search` for file and content search, delete the linked `agentgrep` dependency, and rebuild outline and semantic features locally.

Advantages:

- one search engine
- no legacy backend
- maximum long-term control

Costs:

- rewrites `outline`, `smart`, `trace`, rendering, and exposure parsing at once
- large behavior and prompt-compatibility risk
- difficult to distinguish migration regressions from product redesign

Decision: reject for the first migration.

### B. Add FFF as a mode-level backend inside AgentGrep

Keep the current tool and route compatible `grep` and `find` requests through a shared FFF index. Route all other modes and unsupported option combinations through linked AgentGrep. Preserve one rendering contract.

Advantages:

- smallest reversible path
- immediate benefit on the highest-frequency modes
- semantic modes remain stable
- parity and fallback can be measured per request

Costs:

- two backends coexist temporarily
- requires explicit routing and compatibility tests
- FFF result types must not leak through the tool boundary

Decision: recommended.

### C. Run FFF through MCP or a sidecar process

Install `fff-mcp` and proxy AgentGrep requests to it.

Advantages:

- less Rust API coupling
- upstream owns process lifecycle

Costs:

- adds installation and configuration failure modes
- duplicates Jcode's tool and process boundaries
- loses the simplest in-process warm-index advantage
- complicates packaging, remote sessions, cancellation, and upgrades

Decision: reject.

## Architecture

### Public tool boundary

`AgentGrepTool` remains the only model-facing search tool. Its schema and default mode remain unchanged. Existing aliases such as `pattern`, `include`, and `file_path` continue to work.

The implementation gains a local dispatcher:

```rust
enum SearchBackendDecision {
    Fff,
    LinkedAgentGrep { reason: FallbackReason },
}

fn choose_backend(request: &NormalizedSearchRequest, index: &IndexState)
    -> SearchBackendDecision;
```

Do not introduce a public plugin trait in the first slice. Two private functions and a decision enum are enough. A reusable backend trait is justified only if a third backend or external extension point appears.

### Normalized request and result

Backend-specific arguments must not drive routing. Jcode owns a normalized request contract:

```rust
struct NormalizedSearchRequest {
    mode: SearchMode,
    index_root: PathBuf,
    scope_root: PathBuf,
    exact_file: Option<PathBuf>,
    query: String,
    regex: bool,
    glob: Option<String>,
    file_type: Option<String>,
    paths_only: bool,
    hidden: bool,
    no_ignore: bool,
    max_files: usize,
    max_regions: usize,
}

struct NormalizedPathResult {
    paths: Vec<PathBuf>,
}
```

Phase 1A needs only `NormalizedPathResult`. Both backends deduplicate and sort it before the existing paths-only grammar is rendered.

A flat content-match type is explicitly insufficient for excerpt-mode compatibility. Current AgentGrep output enriches raw matches with structural groups, language and role labels, symbol summaries, non-code caps, compacted lines, and exact total counts. Phase 1B must either gain an upstream AgentGrep enrichment API or define and prove a richer internal result. It cannot silently replace that output with flat FFF excerpts.

The renderer preserves the grammar of the specific migrated surface. Current `grep` and `find` results do not contribute known files or regions in `collect_agentgrep_exposure`; only `outline`, `smart`, and `trace` do. The migration must preserve that absence of grep/find exposure side effects. New optional metadata such as backend and index state belongs in `ToolOutput.metadata`.

### Index and query roots

Jcode distinguishes the index root from the request scope. The index root is the canonical Git top level containing the session working directory. If no Git root exists, it is the canonical session working directory. The query root is the normalized `path` scope from the tool request and must remain inside the index root. Exact files, globs, and type filters further narrow the query root.

The root algorithm is explicit:

1. resolve the session working directory without using the process working directory
2. find and canonicalize the containing Git worktree root; otherwise canonicalize the session root
3. resolve the requested `path` relative to the session root
4. preserve current exact-file behavior: an existing file becomes the exact target and its parent is the scope root
5. reject `..`, symlink, Windows case-folding, or separator normalization that escapes the index root
6. convert accepted scopes into index-relative constraints
7. sort output paths lexicographically and matches by line to preserve current order

A nonexistent exact-file path, worktree boundary, or unprovable containment forces linked-backend fallback. Jcode never silently widens a subdirectory request to the whole index.

### Index registry

An `FffIndexRegistry` is keyed by canonical repository root. `AgentGrepTool` owns it behind an `Arc`. Jcode's existing process-wide base-tool cache shares that tool instance across sessions, while tests that call `AgentGrepTool::new()` receive isolated registries without global reset hooks.

```rust
struct FffIndexRegistry {
    entries: Mutex<HashMap<CanonicalRoot, Arc<IndexEntry>>>,
    limits: IndexLimits,
}

struct IndexEntry {
    generation: u64,
    picker: SharedFilePicker,
    state: AtomicIndexState,
    created_at: Instant,
    last_used_at: Mutex<Instant>,
}
```

The first slice disables persistent frecency and query-history databases. FFF runs in AI mode with its watcher and content index, scoped to the canonical repository root. This avoids cross-session private ranking state and LMDB lifecycle work while preserving the primary warm-index benefit.

### Lazy initialization

The first compatible request for a root creates one entry under the registry lock and starts FFF's background scan after releasing that lock. Request behavior depends only on states Jcode can observe:

- `warming`: scan active, watcher not ready, or readiness deadline not reached; use linked AgentGrep
- `ready`: scan inactive, watcher ready, and the entry generation still current; use FFF for eligible requests
- `ineligible`: root or request cannot use FFF; use linked AgentGrep
- `evicting`: entry was removed from the registry; existing guards may finish, new requests create or find another entry

FFF 0.10.5 does not expose background scan or watcher errors reliably. The first slice therefore does not claim `failed` or `degraded` states. If readiness is not reached within the configured deadline, Jcode keeps falling back and may retry initialization with a new generation after a cooldown.

A request never waits several seconds solely for index warm-up. A short configurable readiness budget may be measured later, but the first slice uses immediate fallback when not ready.

### Backend eligibility

FFF is a migration target only for requests meeting all of these conditions:

- mode `grep`, plus `find` after Phase 2 is enabled
- a canonical index root that passes root-safety checks
- default ignore behavior
- `hidden != true`
- `no_ignore != true`
- supported UTF-8 query and path inputs
- glob and file-type constraints that translate without semantic loss
- an index in `ready` state

The initial FFF production route is narrower: literal `grep` with `paths_only = true`, default ignore behavior, no hidden files, and no unproven translated constraint. Excerpt-mode grep, regex, exact-file scope, path scope, glob, type, and `find` become eligible only in later phases after their parity requirements pass.

- `outline`, `smart`, and `trace` always use linked AgentGrep in this design.

### Query semantics

`grep` preserves caller intent:

- `regex = false`: construct an `FFFQuery` directly from the raw query and Jcode-generated constraints; use FFF plain mode with `smart_case = false`
- `regex = true`: deferred until after Phase 1A; precompile with Jcode's `regex` crate before FFF and use `smart_case = false`
- never pass public query text through FFF's query parser
- no automatic fuzzy, literal, regex, or constraint fallback
- any FFF `regex_fallback_error` or `literal_fallback` is an invariant violation and triggers linked-backend fallback
- `paths_only = true`: request at most one match per file, paginate files until exhaustion, deduplicate, and sort normalized paths lexicographically. Phase 1A needs the complete matching path set but does not claim exact content-match totals.

`find` uses FFF path search in AI mode with FFF's native fuzzy scoring. Persistent frecency and query-history boosts remain disabled in the initial migration. Existing `type` values translate to explicit extension constraints. Existing globs translate only when FFF can represent them exactly.

Jcode normalizes all returned paths relative to the request root and rejects results outside it.

### Resource limits and eviction

The registry uses single-flight initialization per canonical root. It never waits, searches, cancels, or drops an entry while holding the registry map lock. Warm-ups and FFF searches use separate global semaphores.

An entry in an active search is not synchronously destroyed. Eviction removes it from the map, marks its generation `evicting`, requests cancellation, and lets existing `Arc` references and read guards finish before final drop. Jcode does not claim watcher threads have stopped until all entry references are gone. Reinitialization creates a new generation, and stale generations cannot publish readiness or serve new calls.

Defaults for the first slice:

- maximum 1 live repository index in Phase 1A; a later multi-root phase raises the cap after eviction behavior is measured
- replacing the root evicts only an inactive entry; if the current entry is active, the new request falls back
- never index the filesystem root
- index a home directory only when it is the explicit session working directory and an explicit setting permits it
- retain FFF's documented content-size exclusions and report coverage differences during shadow validation
- existing AgentGrep output caps retained
- no persistent frecency or query-history database

The registry reports index state and fallback information in local logs and each search's `ToolOutput.metadata`. A later debug-socket surface may aggregate registry state after Jcode has a clean shared-service access path. The first slice does not refactor tool construction solely to expose that view.

A later phase adds a multi-root least-recently-used registry, initially capped at four, and may use process-memory pressure signals to evict earlier. Phase 1A uses one entry because FFF does not expose an exact per-index memory cost and multi-root eviction is not needed to prove the search hypothesis.

## Data flow

```text
agentgrep tool call
  -> parse aliases and normalize scope
  -> choose repository root
  -> consult FffIndexRegistry
     -> ready + Phase-enabled eligibility: FFF
     -> otherwise: linked AgentGrep
  -> normalize the mode-specific result
  -> compatibility render
  -> ToolOutput text + backend metadata
  -> existing exposure/context collector
```

Tool metadata includes:

```json
{
  "search_backend": "fff",
  "index_state": "ready",
  "fallback_reason": null,
  "search_root_hash": "...",
  "elapsed_ms": 8,
  "total_matches": 17,
  "returned_matches": 17
}
```

Fallback is normal behavior, not a tool warning. Because FFF does not expose reliable background error details, metadata reports observable states and fallback categories without inventing a scan or watcher error.

## Packaging

- Add stable `fff-search = "0.10.5"` behind Jcode's FFF search feature.
- Keep all direct FFF imports in `tool/agentgrep/fff_backend.rs`; FFF types do not cross into generic tool or protocol crates.
- Use its default `ripgrep` walker feature, which avoids Zig but still brings native dependencies elsewhere in the crate.
- Do not enable `zlob`; it requires Zig 0.16 and would expand release tooling.
- Keep the existing linked `agentgrep` dependency.
- Record FFF's MIT license in existing third-party license output if Jcode generates one.
- Verify macOS arm64, Linux x86_64, Linux arm64, and Windows builds.
- Pin through `Cargo.lock`; do not use nightly FFF releases.

The dependency brings new non-optional components including vendored libgit2, LMDB through `heed`, filesystem notification crates, and tracing support. Jcode does not currently depend on those crates directly. Phase 0 must record clean build time, incremental build time, release binary size, dependency count, and platform linker behavior. If the footprint is disproportionate to the observed warm-search benefit, the rollout stops before production routing rather than hiding the cost.

FFF uses Rust edition 2024, matching Jcode. The crate's published metadata does not declare an MSRV, so Jcode's pinned CI toolchain is the compatibility authority. Its default feature is named `ripgrep`, but it uses Rust libraries for traversal and glob behavior and does not require an `rg` executable at runtime.

## Failure handling

- Index scan or watcher does not become observably ready: remain `warming`, fall back, and retry after cooldown. Do not invent an unavailable upstream error reason.
- Watcher stops reporting ready: stop production routing for that generation and fall back.
- File changes during a query: results are advisory search output; normalize missing-file races by dropping vanished matches and recording a count.
- Invalid regex, once regex routing is enabled: precompile with Jcode's regex crate and return the current actionable input error before calling FFF. Never accept FFF's silent literal fallback.
- Unsupported glob or type mapping: fall back rather than broaden scope.
- Hidden or ignored-file request: fall back in the first slice.
- Non-Git directory: allow the canonical session working directory unless it is a prohibited root.
- Very large repository: allow initialization because this is FFF's intended workload, but record warm-up duration and process-memory deltas. Operators can disable FFF or lower the live-index cap if observed memory is unacceptable.
- Memory pressure: evict idle indexes; current request may fall back.
- Panics returned through Jcode-owned calls: catch them at the blocking-task boundary, stop routing to that generation, and fall back. A panic in an upstream detached watcher cannot be assumed recoverable.
- Binary, oversized, or invalid UTF-8 content: preserve FFF's documented exclusions and expose coverage counts. If current AgentGrep would search a case FFF excludes, fall back when detectable.

## Observability

Add counters and durations without source content:

- requests by mode and backend
- fallback reasons
- index warm-up duration
- warming, ready, ineligible, and evicting index counts
- FFF and linked-backend latency histograms
- result-count mismatches during shadow validation
- watcher reinitializations

No query text, file content, or absolute private path is emitted to telemetry.

## Migration phases

### Phase 0: benchmark and parity harness

Before routing production requests, add a developer-only comparison path that runs both backends for eligible fixture and real-repository requests. It compares normalized paths, line numbers, scope, totals, and bounded output. Ranking differences for `find` are expected and evaluated separately from missing results.

Shadow execution is globally bounded: one comparison per root and query class at a time, a hard timeout, separate warm-up and search semaphores, cancellation on daemon shutdown or eviction, and load shedding when blocking or Rayon work is saturated. Skipped and timed-out comparisons are counted.

This phase also performs a packaging go/no-go check before repository integration. It compiles the published stable crate under Jcode's toolchain and release targets, audits licenses and duplicate native dependencies, and measures binary and build-time growth.

This phase proves or rejects the performance hypothesis on:

- cold index initialization
- warm repeated case-sensitive literal `paths_only` grep
- lowercase and mixed-case parity
- repository mutation followed by watcher update
- concurrent searches from multiple sessions
- later, each additional request class before it becomes eligible

### Phase 1A: smallest production slice

- Add one single-root-capable registry with single-flight initialization, generation checks, and hard warm-up and search concurrency caps.
- Start indexes lazily.
- Route only ready, case-sensitive, literal `grep` requests with `paths_only = true`, default ignore behavior, and no additional scope translation through FFF.
- Sort returned paths lexicographically to match current output.
- Preserve linked AgentGrep for cold start and every unsupported case.
- Preserve the exact paths-only output grammar.
- Add backend metadata and bounded local diagnostics.

This slice avoids structural enrichment, excerpt rendering, and exact total-match counts. It directly tests the warm-index hypothesis without rewriting AgentGrep's product-visible renderer.

### Phase 1B: excerpt-mode grep

Excerpt mode requires an owner for current AgentGrep enrichment and rendering. Prefer adding an upstream AgentGrep API that accepts precomputed raw matches and returns the existing enriched result. Before routing, prove exact total and omission counts, structural grouping, language and role labels, symbol summaries, non-code caps, and long-line compaction. Collecting all FFF pages is acceptable only if the benchmark still shows a useful improvement.

### Phase 1C: regex and constraints

Add case-sensitive regex, exact-file, path, glob, and type routing one at a time. Each addition requires direct `FFFQuery` construction, prevalidation, containment proof, deterministic sorting, and dedicated parity tests. Any inferred or silent FFF fallback disables that route.

### Phase 2: FFF file find

Route compatible `find` calls through FFF and validate native fuzzy ranking, pagination, and weak-match handling. Keep exact path and unsupported scopes on fallback. Persistent frecency remains deferred.

### Phase 3: simplify

After production evidence shows parity and reliability:

- remove dead linked grep/find argument and rendering paths
- decide whether AgentGrep's semantic engine should consume FFF candidates
- consider persistent frecency only with explicit privacy and lifecycle design
- revisit measured memory data before enabling the multi-root LRU cap of four

Do not remove linked AgentGrep while it still owns `outline`, `smart`, or `trace`.

## Acceptance criteria

### Correctness

- Existing public AgentGrep grep tests pass unchanged, and new tests inject a deterministically ready index and assert `search_backend = "fff"` so fallback cannot create a false pass.
- Phase 1A preserves byte-for-byte `paths_only` output for case-sensitive literal queries, including zero matches and lexicographic path order.
- Lowercase and mixed-case queries return the same path set as linked AgentGrep.
- Current grep/find exposure behavior remains unchanged: neither backend adds known-file or known-region side effects.
- A request outside the Phase-enabled exact compatibility surface uses linked AgentGrep and reports a machine-readable fallback reason.
- Files created, modified, renamed, and deleted after warm-up appear correctly within two seconds after filesystem-event quiescence in the runtime acceptance environment.
- Later routes for excerpt, regex, exact file, path, type, and glob must each pass dedicated parity criteria before eligibility.

### Performance

On the Jcode repository with a warm index, measure at least 50 iterations after 5 unmeasured warm-up calls on an otherwise idle machine. Report p50, p95, sample count, index state, filesystem-cache state, hardware, and backend for each request class.

For Phase 1A:

- p50 repeated literal `paths_only` grep latency improves over linked AgentGrep
- p95 does not regress by more than 10 percent
- no routed query exceeds the current five-second foreground budget in normal repository conditions
- index initialization and shadow work never block tool responsiveness

No absolute speedup is promised until these measurements are observed on supported platforms.

### Resource behavior

- Opening a second repository replaces the inactive Phase 1A index. If the first index is active, the second request falls back without disrupting it.
- Active entries are not synchronously destroyed. After cancellation and all in-flight guards and `Arc` references finish, tests observe watcher/thread termination and memory becoming reclaimable.
- An index that never becomes ready does not make search unavailable and retries only after cooldown.
- Tool metadata and local diagnostics use only observable states: warming, ready, ineligible, or evicting.

### Packaging

- `cargo tree -e features -p jcode-app-core` is reviewed for native and duplicate dependency growth.
- `cargo check -p jcode-app-core` passes with the stable crate.
- clean and incremental build-time deltas are recorded.
- release binary-size growth is recorded and explicitly accepted.
- vendored libgit2 and LMDB build and link successfully on every release target, including portable Linux.
- TUI self-dev build passes on macOS arm64.
- CI build matrix passes on Linux, macOS, and Windows targets.
- Release packaging requires no external FFF binary, Node package, MCP server, or Zig installation.

### Public acceptance path

Using a self-dev Jcode binary and isolated daemon socket:

1. enable `shadow` on an isolated daemon and use a test-only readiness barrier to wait for one specific registry generation
2. run a case-sensitive literal `paths_only` AgentGrep query and confirm linked output plus a completed shadow comparison
3. enable `prefer` and rerun; assert `search_backend = "fff"` and byte-for-byte path parity
4. create, modify, rename, and delete matching files, waiting for event quiescence, and observe the updated path set through FFF
5. run lowercase and mixed-case variants and compare with linked AgentGrep
6. run excerpt, regex, path, glob, type, exact-file, hidden, and `no_ignore` requests and observe linked-backend fallback during Phase 1A
7. run `outline` and `trace` and observe unchanged linked behavior
8. inspect tool metadata for generation, observable index state, and fallback reason

## Testing strategy

- Unit tests for request normalization, routing, direct `FFFQuery` construction, case behavior, path containment, and fallback reasons.
- Byte-for-byte golden tests for Phase 1A paths-only output, zero matches, ordering, CRLF files, Unicode, and backend metadata.
- Phase 1B goldens for headers, structural groups, non-code caps, long-line compaction, dense-file totals, many-file totals, and omission counts before excerpt routing.
- Registry tests for single-flight initialization, generation safety, readiness timeout, cooldown retry, semaphores, eviction during search, reinitialization, cancellation, and final drop behavior.
- Fixture tests for ignore files, hidden files, untracked files, symlinks, Unicode paths, invalid UTF-8, binary files, large files, rename/delete races, and watcher updates.
- Property tests that every Phase-enabled FFF scope remains inside the requested root and exact constraints.
- Real-crate integration tests using stable `fff-search`, not copied FFF source or a fake index.
- Runtime acceptance through the public AgentGrep tool and isolated Jcode daemon.

## Rollout and rollback

Start behind a typed setting:

```toml
[search]
fff_backend = "off" # off | shadow | prefer
max_fff_indexes = 1
```

The typed setting belongs to `jcode-config-types`; runtime resolution remains in Jcode's normal config path. `AgentGrepTool::new()` owns an isolated registry, and Jcode's existing process-wide base-tool cache shares the production instance across sessions.

- `off`: current behavior
- `shadow`: linked AgentGrep answers; eligible requests also run FFF for parity and timing without affecting output
- `prefer`: ready eligible requests use FFF; all others fall back

Default to `off` for the first merged build, then `shadow` for self-dev and canary use. Shadow comparisons are sampled and run outside the response critical path. Move to `prefer` only after parity, watcher, memory, and packaging acceptance pass. Rollback is one configuration change and requires no data migration.

## Success criteria

- Models continue using one stable `agentgrep` tool.
- Warm repeated content searches become measurably faster in Jcode.
- Search remains available whenever FFF is cold, unsupported, warming, ineligible, or evicting.
- No scope broadening, output grammar regression, or context-exposure regression ships.
- The first production slice adds one private registry and one mode-level backend path, not a new plugin system.
- Higher-level AgentGrep modes retain their existing behavior.
- Packaging remains a single Jcode binary with no new runtime installation step.
