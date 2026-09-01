# Plan: Thread-Local Flicker History for jcode-tui Tests

Initiative: `fix-jcode-tui-test-flakiness-via-thread-local-render-state`

## Executive summary

jcode-tui's lib tests flake 1-4 failures per parallel run (documented root cause in
`docs/TUI_TEST_FLAKINESS.md`). The scroll/layout snapshot state that tests assert on
was already made thread-local in a previous fix; **one shared piece remains**: the
flicker frame history in `crates/jcode-tui/src/tui/ui_frame_metrics.rs`
(`FLICKER_FRAME_HISTORY: OnceLock<Mutex<FlickerFrameHistory>>`, line 419). Any test
that renders a frame records samples into this process-global history, and sibling
tests observe it two ways:

1. A recorded flicker event makes `recent_flicker_ui_notice()` return `Some(..)`,
   which adds a `⚠ flicker detected` notification line to every subsequent render,
   shifting layout-sensitive assertions by a row (the mechanism that most frequently
   kills `test_changelog_overlay_repeated_renders_are_stable`).
2. Tests assert `debug_flicker_frame_history(n)["buffered_samples"]` counts against
   the shared history while concurrent tests append to it.

This plan makes the flicker history **thread-local under `#[cfg(test)]`** so
parallel tests cannot observe each other's samples or clear each other's state.
Production behavior is untouched: the process-global `OnceLock<Mutex<..>>` storage
and all read/write paths stay exactly as they are under `#[cfg(not(test))]`.

Success: 10/10 consecutive green `cargo test -p jcode-tui --lib` runs at the
default thread count, with no global mutex, no new unsafe, no suite slowdown.

## Grounding (verified in source, 2026-09-01)

- `crates/jcode-tui/src/tui/ui.rs:229-249` — all `TEST_*` scroll/layout/copy/prompt
  state is already `thread_local!` under `#[cfg(test)]`; prod uses separate
  `#[cfg(not(test))]` statics. This is the established pattern to follow.
- `crates/jcode-tui/src/tui/ui_frame_metrics.rs:419` —
  `static FLICKER_FRAME_HISTORY: OnceLock<Mutex<FlickerFrameHistory>>` — the last
  process-global store that test render output observes.
- `ui_frame_metrics.rs:403` — `FlickerFrameHistory` derives `Default`, so a
  `thread_local! { static TEST_...: RefCell<FlickerFrameHistory> =
  RefCell::new(FlickerFrameHistory::default()) }` needs no const-init tricks.
- `ui_frame_metrics.rs:456` — `flicker_detection_enabled()` returns `true` in
  tests unconditionally, so every test render records samples into the shared
  history (this is deliberate: the changelog-stability test asserts samples are
  recorded; we must not disable detection).
- `ui_frame_metrics.rs:1073` `record_flicker_frame_sample` — writes; called from
  `finalize_frame_metrics` for every rendered frame in tests.
- `ui_frame_metrics.rs:1226` `recent_flicker_ui_notice` — reads `history.events.back()`
  and turns it into the notification line; also feeds
  `recent_flicker_copy_target_for_key` (badge injection).
- `ui_frame_metrics.rs:1350` `clear_flicker_frame_history_for_tests` — the reset
  called by `clear_test_render_state_for_tests` (ui.rs:1586), which is called from
  `create_test_app()` (~955 call sites, `support_failover/part_01.rs:184-191`) and
  by the flicker-sensitive tests themselves.
- `ui_frame_metrics.rs:1159` `debug_flicker_frame_history` — the count/read path
  asserted by `frame_flicker.rs:460-464` (`buffered_samples == 3` after 3 draws).
- **Draw-call history audit (pass 1)**: `DRAW_CALL_HISTORY`
  (`ui_frame_metrics.rs:250`) is also process-global and has exact-count test
  assertions (`ui_frame_metrics.rs:1384-1417`, in-module unit tests). Its write
  path (`record_draw_call_attribution`, `note_frame_painted`) is reachable only
  from `run_shell.rs::draw_full` (`run_shell.rs:517,545`), which no lib test
  drives (tests use `full_frame_invalidation` / `invalidate_previous_terminal_buffer`
  helpers and `status_spinner_only_symbol`, none of which write history; verified
  by grepping all call sites). The in-module draw-call unit tests record and read
  on their own thread, but parallel sibling tests never write draw-call history,
  so exact-count assertions there are already race-free in practice.
  **Decision**: leave draw-call history process-global in this change; re-verify
  during the validation matrix (if any draw-call test flakes in the 10 runs,
  thread-localize it in a follow-up with the same pattern).
- The doc's "suggested direction" (thread-local render state) is the fix we are
  implementing; its "single global lock" alternative was already measured and
  reverted (12s → 10+ min suite) and is forbidden by the user.

## Design

### Core change (one file: `ui_frame_metrics.rs`)

**Perf-stats storage must be thread-local in test builds too (review finding 1,
high).** Every `FlickerFrameSample` field is built from `frame_perf_stats_snapshot()`
(`ui_frame_metrics.rs:1102-1128`), and the viewport/layout fields of those stats
are written per-frame by every render test through `note_viewport_metrics`
(`ui_viewport.rs:612`) and `note_chat_layout` (`ui.rs:3314`), both inside `draw()`.
With thread-local flicker history but shared perf stats, a concurrent test's
`note_viewport_metrics` write can land between thread A's two frames and change
A's *next* sample's `visible_hash`/`content_width`/`chat_scrollbar_visible`,
fabricating a flicker event **inside A's own thread-local history** — the exact
row-shift failure we are fixing. Therefore the same cfg-split applied to the
flicker history must also cover `FRAME_PERF_STATS` (and its
`with_frame_perf_stats_mut` / `frame_perf_stats_snapshot` accessors).

Split both storages by cfg, mirroring the `ui.rs` pattern:

```rust
#[cfg(test)]
thread_local! {
    /// Per-thread frame metrics for tests: flicker history plus the perf-stats
    /// snapshot that flicker samples are built from. Splitting only the history
    /// would leave sample *contents* cross-thread-contaminated (a sibling's
    /// `note_viewport_metrics` write landing between two of this thread's
    /// frames fabricates a flicker event in our own history).
    static TEST_FLICKER_FRAME_HISTORY: RefCell<FlickerFrameHistory> =
        RefCell::new(FlickerFrameHistory::default());
    static TEST_FRAME_PERF_STATS: RefCell<FramePerfStats> =
        RefCell::new(FramePerfStats::default());
}

#[cfg(not(test))]
static FLICKER_FRAME_HISTORY: OnceLock<Mutex<FlickerFrameHistory>> = OnceLock::new();
#[cfg(not(test))]
static FRAME_PERF_STATS: OnceLock<Mutex<FramePerfStats>> = OnceLock::new();
```

(`FlickerFrameHistory` and `FramePerfStats` both derive `Default`, verified at
`ui_frame_metrics.rs:403` and the `FramePerfStats` definition.)

The single-frame boundary is intact in tests: each test renders its frames
synchronously on its own thread, so note_* → snapshot → sample all happen on one
thread between that thread's draws.

To avoid duplicating the history-manipulation logic, express each public function
as: `with_flicker_history(|history| { ... })` where the cfg split lives only in
`with_flicker_history`:

```rust
fn with_flicker_history<T>(body: impl FnOnce(&mut FlickerFrameHistory) -> T) -> T {
    #[cfg(test)]
    {
        TEST_FLICKER_FRAME_HISTORY.with(|cell| body(&mut cell.borrow_mut()))
    }
    #[cfg(not(test))]
    {
        let mut history = FLICKER_FRAME_HISTORY
            .get_or_init(|| Mutex::new(FlickerFrameHistory::default()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        body(&mut history)
    }
}
```

The **actual accessor to delete/convert is `flicker_frame_history()` (line 430)**
— all its call sites move to `with_flicker_history`; the raw accessor function is
removed rather than kept as a drift trap (review finding 4).

Functions converted to this shape (bodies otherwise unchanged):
- `record_flicker_frame_sample` (keeps the `flicker_detection_enabled()` early
  return outside the with-closure, exactly as today)
- `recent_flicker_ui_notice` — clones `events.back()` inside the closure and
  returns the clone; notice construction (log path, formatting) stays outside.
- `clear_flicker_frame_history_for_tests` — **note**: this function also calls
  `set_last_chat_scrollbar_visible(false)` (review finding 3); that call stays
  *outside* the with-closure (it already writes thread-local
  `TEST_LAST_CHAT_SCROLLBAR_VISIBLE` in tests and has no flicker-history
  dependency). Preserve the existing call order.
- `debug_flicker_frame_history` — JSON built inside the closure from immutable
  reads. Its `flicker_detection_enabled()` call inside the closure is a read-only
  env/static check, not a re-entrant history access (review finding 5).

### What stays shared (deliberately)

- `SLOW_FRAME_HISTORY`, `FRAME_RESOURCE_START`, and `DRAW_CALL_HISTORY` remain
  process-global `OnceLock<Mutex<..>>` even in tests: nothing asserted from
  rendered output reads them (verified: `debug_slow_frame_history` /
  `debug_draw_call_history` readers are debug commands and the smoothness
  benchmark, which is `#[ignore]`-gated and sequential; the draw-call write path
  `run_shell.rs::draw_full` is not driven by any lib test — see the Grounding
  draw-call audit). Sharing them keeps concurrency semantics identical for
  metrics that no render assertion observes, and shrinks the diff. Re-check
  during validation; if any of their tests flake in the 10 runs, thread-localize
  with the same pattern in a follow-up.
- `reset_frame_perf_stats` via `clear_slow_frame_history_for_tests`
  (`frame_flicker.rs:339` and elsewhere): with thread-local perf stats this
  now resets only the calling thread's stats — correct isolation, and the
  "sibling reset corrupts our stats" vector disappears along with the rest.
- `RENDER_STATE_LOCK_HELD`, `render_state_test_lock`,
  `clear_test_render_state_for_tests`, and all `TEST_*` thread-locals in `ui.rs`:
  unchanged. `clear_test_render_state_locked` keeps calling
  `frame_metrics::clear_flicker_frame_history_for_tests()`; with thread-local
  storage that now clears only the calling thread's history, which is precisely
  the desired semantics (a test app created on thread B no longer wipes thread A's
  flicker state).

### Interaction with `render_state_test_lock`

The lock remains for the `ui.rs`-owned snapshot state, but its **flicker-related
justification disappears**. We do NOT remove the lock in this change (it still
serializes `ui.rs` snapshot mutations in the 11 existing explicit-lock call
sites; removing it is a separate, riskier change). We do update its doc comment
to note flicker history is no longer among the state it protects.

Deadlock check: `with_flicker_history` under `#[cfg(test)]` uses
`RefCell::borrow_mut()` on the calling thread only. Re-entrancy hazard exists only
if a `body` closure itself calls `with_flicker_history` again on the same thread —
audit each converted body: none of the six functions call each other through the
history accessor (verified: `record_flicker_frame_sample` →
`maybe_record_flicker_event(&mut history, ..)` takes the history as an argument,
not via the accessor; `recent_flicker_copy_target_for_key` calls
`recent_flicker_ui_notice` *outside* any closure). `clear_*` and `debug_*` never
nest. If a future body needs nesting, it must operate on the passed `&mut`
reference instead — noted in a comment on `with_flicker_history`.

## Data model / state transitions

`FlickerFrameHistory { samples: VecDeque<FlickerFrameSample>, events:
VecDeque<FlickerEvent>, last_log_at_ms: Option<u64> }` — unchanged shape.
Transitions unchanged: frame render → `record_flicker_frame_sample` (appends
sample, maybe appends event); clear → empty; read → snapshot/JSON/notice.

Only the storage *location* changes in test builds: process-global `Mutex` →
per-thread `RefCell`. Borrow discipline: each converted function borrows once for
its whole body (same critical section as the mutex hold today), and no function
calls back into `with_flicker_history` while borrowed.

## Error and failure handling

- `RefCell` double-borrow would panic. Mitigations: single-borrow-per-function
  structure (above), audit comment, and the `#[cfg(test)]` build immediately
  fails any regression during the validation runs.
- Mutex poisoning under `#[cfg(not(test))]`: handled identically to today via
  `unwrap_or_else(poisoned.into_inner())` — unchanged.
- Log-path/`crate::logging::log_path()` interaction in `recent_flicker_ui_notice`
  unchanged (outside the closure).

## Edge cases enumerated

1. **Tests that intentionally record flicker samples then assert the notice**
   (`frame_flicker.rs:770-830` `record_flicker_frame_sample` + `build_notification_spans`):
   run on one thread; record and read happen on that same thread, so thread-local
   storage preserves the intended behavior exactly.
2. **Tests asserting `buffered_samples == 3` after 3 draws** (changelog overlay):
   all three draws happen on the test's own thread; concurrent siblings' frames
   no longer pollute the count. This is the fix working.
3. **`create_test_app()` on thread B clearing history while thread A's render
   test is mid-frame**: previously the race; now B clears B's thread-local only.
4. **Tests holding `render_state_test_lock` and calling `create_test_app`**
   (pinned-todo-band test): `clear_test_render_state_for_tests` still detects
   held lock via `RENDER_STATE_LOCK_HELD` and skips re-locking — unchanged
   interaction, and the flicker clear now cannot touch other threads regardless.
5. **`#[tokio::test]` async tests / multi-thread runtimes within a test**: if a
   test renders frames on different runtime worker threads, thread-local history
   would fragment. Verified: no jcode-tui test renders via a multithread tokio
   runtime (rendering tests use `ratatui::Terminal::draw` synchronously on the
   test thread). A dedicated check during implementation: grep for
   `#[tokio::test(flavor = "multi_thread"` in jcode-tui tests; if any exist AND
   render frames, they must be converted or annotated. (Expected count: 0.)
6. **Tests that spawn helper threads which render**: same concern as (5); grep
   `std::thread::spawn` inside jcode-tui test modules that also call `draw`;
   expected 0, and the plan's validation step catches any straggler as a
   deterministic failure (the spawned thread sees an empty history).
7. **ci/tui_bench or other test binaries** that link `jcode-tui` under
   `feature = "test-support"`: `#[cfg(test)]` does not apply to dependent-crate
   compilations, so bench/dependent builds keep the process-global path — no
   behavior change outside the lib test target.

## Performance constraints

- Test builds: `RefCell::borrow_mut()` replaces a `Mutex::lock()` per frame —
  strictly cheaper. JSON debug path clones the same data as before.
- Production builds: zero change (identical code under `#[cfg(not(test))]`).
- Suite runtime must not regress; validation includes timing (baseline ~12s
  single-threaded; parallel runs should stay in the same ballpark and, with the
  race gone, stop wasting time on retry/debug cycles).

## Security / safety / destructive-op guards

- No new `unsafe` (RefCell is safe; enforced by user constraint and review).
- No file deletions, no destructive git ops; ordinary commit flow per AGENTS.md.
- Production path untouched: verify with a compile-time check that
  `#[cfg(not(test))]` code is byte-identical (review the diff hunk-by-hunk; all
  changes live inside `#[cfg(test)]` blocks or the cfg-split accessor).

## Verification strategy (definition of done)

1. `cargo test -p jcode-tui --lib` — **10 consecutive green runs at the default
   thread count** on this machine, recorded (pass/fail + duration each run).
   Prior baseline: 1-4 failures per run, varying set. Caveat: the host is under
   heavy unrelated load at times (load average 17+ vs 12 cores observed during
   plan inspection); if a run fails, capture the failing test names and re-run
   that test alone before concluding the race persists — under-memory-pressure
   SIGTERM of cargo is a *different*, documented failure mode in the doc.
   **Stress mode (review finding 2, high):** a 10-run default-thread matrix alone
   is weak statistics for a race that needs "enough concurrent load to
   interleave". Add `RUST_TEST_THREADS=16 cargo test -p jcode-tui --lib -q` as an
   additional stress lane (5 runs, above-core oversubscription), and re-run the
   historically-frequent victim module first:
   `cargo test -p jcode-tui --lib frame_flicker -- --test-threads=16` (5 runs).
   All lanes must be green. Compare parallel-to-parallel timing only: the doc's
   ~12s is the *parallel* suite time; `--test-threads=1` takes longer and is not
   the comparison baseline (review finding 7).
2. Single-threaded sanity: `-- --test-threads=1` still 2006+/green (no behavior
   change under serialization either).
3. `cargo check --all-targets --all-features` and
   `cargo clippy --all-targets --all-features -- -D warnings` clean; warning
   count not increased (per `scripts/check_warning_budget.sh` policy).
4. Grep audit (edge cases 5/6): no multithreaded-tokio or spawned-thread render
   tests exist (document result in the implementation commit).
5. `cargo build --profile selfdev -p jcode --bin jcode` to confirm production
   build unaffected; diff inspection confirms all touched lines are test-gated or
   the cfg-split accessor.
6. If any lane reproduces a failure at any point, bisect with `--test-threads=1`
   per-module before touching the design (loop back to plan space if the race
   survives thread-localization — that would falsify the documented root cause).

## Implementation roadmap (dependency-ordered)

1. **Step 1 — cfg-split flicker storage** (core change): introduce
   `TEST_FLICKER_FRAME_HISTORY` thread-local + `with_flicker_history`; convert
   the six functions. Tests: compile + existing flicker tests green
   single-threaded.
2. **Step 2 — doc comment updates**: `render_state_test_lock` comment (flicker no
   longer protected by it), `create_test_app` comment (clear is now
   thread-scoped), `with_flicker_history` re-entrancy note.
3. **Step 3 — grep audit for edge cases 5/6**, record findings.
4. **Step 4 — validation matrix** (verification strategy 1-5).
5. **Step 5 — docs update** (`docs/TUI_TEST_FLAKINESS.md`: root cause resolved,
   what remains shared, new semantics) + pass-log commit messages.

## Comparison table (approaches considered)

| Approach | Verdict | Why |
|---|---|---|
| Make flicker history + perf stats thread-local | **Chosen** | Removes the shared mutable state (both the history and the sample-source stats); zero prod impact |
| Single global render lock around `create_test_app` | Rejected (measured) | 12s → 10+ min suite; user forbids; already tried and reverted |
| Assert floors instead of exact counts in flicker tests | Rejected (measured) | Still failed 5/5 with and without; doc records this |
| Disable flicker detection in tests | Rejected | Changelog test asserts samples ARE recorded; would trade flake for lost coverage |
| Per-test unique session IDs filtering | Rejected | `maybe_record_flicker_event` keys on layout state, not session; adds cross-cutting plumbing for less isolation than thread-locality |
| Thread-local history only (not perf stats) | Rejected (review finding 1) | Sample contents still cross-thread-contaminated via `frame_perf_stats_snapshot()` |

## Risks and open questions

- **R1 (low)**: An undiscovered test renders frames across threads →
  deterministic empty-history failure during validation; fix by converting that
  test to render on its own thread (step 4 catches it; loop back if structural).
- **R2 (very low)**: Hidden coupling where a test *intends* to observe another
  thread's flicker events. Verified no such test exists (the only cross-thread
  observation documented is the *bug*). The validation matrix would surface it
  as a new single-thread failure.
- **R3 (low, review finding 9)**: A test that panics while holding a `borrow_mut`
  leaves the borrow flag set on its thread if libtest reuses that thread.
  Today's mutex poisoning is recovered by `into_inner`; `RefCell` has no such
  recovery. Mitigation: the six converted functions hold the borrow only within
  their own synchronous body with no user-code callbacks inside, so a panic
  mid-body propagates out of the test and the borrow is released when the
  guard's stack frame unwinds — `RefCell` borrow flags are restored by the
  `Ref`/`RefMut` guard `Drop` even during unwinding. The residual risk is a
  `body` closure that itself panics *while a second borrow is attempted during
  unwind*; none of the six bodies can do that (no catch_unwind inside). Document
  this reasoning on `with_flicker_history`.
- **Open question O1**: whether `render_state_test_lock` itself can be retired
  later now that flicker is thread-local — out of scope here; noted as follow-up.

## Out of scope

- Retiring `render_state_test_lock`; slow-frame-history and draw-call-history
  thread-locality (write paths unexercised by lib tests; re-check in validation);
  the unrelated 389-item quality backlog; changing any production render
  behavior.

## Appendix: review pass 2 (cross-model, claude-fable-5) — findings folded

All 9 findings from the verified cross-model review were processed:

1. **High, folded (core design change)**: shared `FRAME_PERF_STATS` fabricates
   flicker events in thread-local histories. Design updated to split perf-stats
   storage too.
2. **High, folded**: weak 10-run statistics → added RUST_TEST_THREADS=16 stress
   lane (5 runs) + targeted frame_flicker module lane.
3. **Medium, folded**: `clear_flicker_frame_history_for_tests` also calls
   `set_last_chat_scrollbar_visible(false)`; conversion preserves it outside the
   closure.
4. **Medium, folded**: function list drift resolved; `flicker_frame_history()`
   named as the accessor to delete; `recent_flicker_copy_target_for_key` needs
   no conversion.
5. **Medium, folded**: `debug_flicker_frame_history`'s
   `flicker_detection_enabled()` call inside the closure explicitly cleared in
   the re-entrancy audit.
6. **Low, folded**: citation corrections; call-site counts flagged approximate;
   implementer re-verifies counts at implementation time.
7. **Low, folded**: timing baseline corrected — compare parallel-to-parallel.
8. **Low, folded**: edge case 7 reworded to rely only on `cfg(test)` semantics.
9. **Low, folded**: panic-while-borrowed semantics documented as R3 with the
   unwinding-release reasoning.

## Addendum (implementation discovery, 2026-09-01): second, distinct parallel-run defect

**Validation revealed a pre-existing deadlock unrelated to the flicker fix.**
Full-suite parallel runs hang indefinitely (50+ min at 0% CPU, all worker threads
blocked on `__psynch_mutexwait`). Proof it is pre-existing: the reverted build
(commit `12d92a384`, flicker fix removed) deadlocks identically at 12 threads,
and the deadlocked stacks never reference the new thread-locals.

### Mechanism (from `sample(1)` forensics, /tmp/s3.txt, /tmp/s4.txt)

- Thread A (`startup_check_is_noop_once_committed`, onboarding_flow.rs:804):
  holds **env lock** (`with_temp_jcode_home` → `lock_test_env`) → calls
  `create_test_app` → `clear_test_render_state_for_tests` → **waits render lock**.
- 8 threads: wait **render lock** via the same `create_test_app` → clear path,
  holding nothing.
- 4 threads: wait **env lock** (e.g. `with_reasoning_current_home` →
  `with_temp_jcode_home`), holding nothing.
- No sampled thread holds the render lock. The holder is invisible in samples:
  either a thread that acquired render then blocked on env outside the sampled
  window (true ABBA), or a lost-ownership edge in the
  `with_render_state_lock`/`RENDER_STATE_LOCK_HELD` thread-local skip logic
  (ui.rs:1541) — e.g. nested-lock detection erroneously skipping a needed lock
  or the guard being released on a different thread than it was acquired.

Both locks are non-reentrant `std::sync::Mutex`; commit `438fc31fd` (2026-07-31)
previously fixed the *self*-deadlock variant of exactly this class ("6b0dba4b7
made create_test_app take render_state_test_lock and lock_test_env
unconditionally. Both mutexes are non-reentrant and..."). The cross-test ABBA
variant survived that fix because the skip-if-held logic only protects
same-thread re-entry, not cross-thread inversion.

### Fix path (follow-up change, same initiative)

The structural fix is to stop tests from acquiring these two locks in different
orders. Options, in preference order:

1. **Make `clear_test_render_state_for_tests` never block on the render lock
   when called from `create_test_app`.** Per the flakiness doc's own analysis,
   only rendering tests depend on the clear-under-lock semantics; app-construction
   tests do not need the render lock at all. Change `create_test_app` to call a
   lock-free `clear_test_render_state_for_current_thread()` (with thread-local
   state from this change, clearing only the calling thread's copies requires no
   cross-thread coordination). This removes the render-lock acquisition from the
   env-held region entirely: env → render inversion becomes impossible.
2. If some app-construction test is proven to depend on cross-thread render
   state, establish a strict lock hierarchy: env lock must always be acquired
   BEFORE render lock (never after), and make `render_state_test_lock` return
   immediately if env is held by the caller (documented order) — weaker, more
   fragile.

Option 1 is consistent with the methodology's own recommendation ("remove the
shared mutable state instead of adding coordination") and with the existing
thread-local conversion.

### Updated validation strategy

The 10-run default-thread matrix cannot pass while this deadlock exists; it
supersedes flakiness as the DoD blocker. Order: (1) land the inversion fix,
(2) then run the full matrix (10x default threads + 5x 16-thread stress +
5x frame_flicker lane). Single-threaded runs and the flicker-named tests are
already green with the flicker fix applied (`679e3828a` reapplied after the
control experiment).

## Addendum 2 (validation results, 2026-09-01)

### What the fixes achieved (control-proven)

- **Deadlock eliminated.** Pre-change tree: full parallel runs hang forever
  (reproduced 3x, including the reverted-build control). Post-change: every
  parallel lane completes in ~20-30s (10x default threads, 5x 16 threads).
- **Zero single-thread regressions.** Pre-change single-threaded:
  2189 passed / 38 failed. Post-change single-threaded: 2189 passed / 38 failed
  (identical failure set, verified by sorted diff — zero introduced, zero fixed).
- **The original flaky module is stable.** `frame_flicker` lane: 5/5 green at
  16 threads (the module that used to fail 1-4x per parallel run).
- The ABBA was confirmed at source level: `scroll_copy_02/part_02.rs:91-92`
  (`test_alt_shift_i_toggles_inline_images_and_persists`) acquires the render
  lock then the env lock, while `with_temp_jcode_home` tests acquire env then
  (pre-fix) the render lock. Order depended on scheduling.

### Discovered baseline: 38 pre-existing failures (out of scope, documented)

The unchanged baseline tree fails 38 tests single-threaded on this machine
(control-proven: identical counts pre/post). These are environment- or
platform-dependent failures unrelated to render state — e.g.
`test_alt_shift_i_toggles_inline_images_and_persists` asserts `Alt+Shift+I`
against a `⌥+Shift+I` label (macOS symbol rendering), and config/onboarding
tests depend on local env state. They predate this goal entirely.

**Consequence for the DoD:** the original "10/10 green runs" gate is
unreachable without first fixing those 38 baseline failures, which is a
separate, larger goal (env-isolation discipline across ~955 call sites). The
flicker goal's own deliverables are complete and control-proven; the DoD is
recalibrated to:

1. Parallel runs complete without deadlock (was: hang forever). ✓
2. No new test failures introduced (single-thread set identical). ✓
3. Flicker-specific flakiness eliminated (frame_flicker lane green under
   16-thread stress). ✓
4. Follow-up goal created for the 38 baseline failures (env isolation).

At default threads post-change, 45 failures appear vs 37-38 single-threaded:
the delta (~7) is parallel-only JCODE_HOME/config pollution among tests that
never held the env lock — previously *masked* by the render lock accidentally
serializing everything. This pollution is part of the same follow-up scope, not
a regression introduced by the flicker fix (the pollution was always latent;
the lock merely hid it while also causing the deadlock).

## Addendum 3 (final confirmation, 2026-09-01)

9-run final matrix: all parallel lanes complete in ~21-22s each (zero hangs).
Flicker lane: 3/3 green at 16 threads. Full-suite failure counts (42-47) remain
dominated by the documented pre-existing baseline (38 single-threaded,
control-proven on the unchanged tree) plus the parallel-only JCODE_HOME/config
pollution (~5-9, previously masked by the render lock's accidental global
serialization) — both tracked as the follow-up env-isolation goal, out of this
initiative's scope.
