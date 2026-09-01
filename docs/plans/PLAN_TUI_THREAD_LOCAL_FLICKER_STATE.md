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

Split the flicker history storage by cfg, mirroring the `ui.rs` pattern:

```rust
#[cfg(test)]
thread_local! {
    /// Per-thread flicker history for tests. Parallel test threads each get
    /// their own history, so `create_test_app()`'s render-state clear (or any
    /// sibling test's frames) can never inject a "⚠ flicker detected"
    /// notification into another test's render, and `buffered_samples`
    /// assertions observe only this thread's frames.
    static TEST_FLICKER_FRAME_HISTORY: RefCell<FlickerFrameHistory> =
        RefCell::new(FlickerFrameHistory::default());
}

#[cfg(not(test))]
static FLICKER_FRAME_HISTORY: OnceLock<Mutex<FlickerFrameHistory>> = OnceLock::new();

fn flicker_frame_history_slot() -> ... // cfg-split accessor:
  - #[cfg(test)]: operate directly on the thread-local RefCell via a closure API
  - #[cfg(not(test))]: lock the Mutex and operate
```

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

Functions converted to this shape (bodies unchanged):
- `record_flicker_frame_sample` (needs the `flicker_detection_enabled()` early
  return kept outside the with-closure, exactly as today)
- `recent_flicker_ui_notice` — reads need `&FlickerFrameHistory`, not `&mut`;
  the closure takes `&mut` which coerces. It clones `events.back()` and drops the
  borrow before the log-path work; preserve that ordering (clone inside closure,
  return the clone, do notice construction outside).
- `recent_flicker_copy_target_for_key` — calls `recent_flicker_ui_notice`; no
  direct history access.
- `clear_flicker_frame_history_for_tests`
- `debug_flicker_frame_history` — builds its JSON from immutable reads; build the
  summary/serialization inside the closure and return the `serde_json::Value`
  (clone cost is identical to today's per-item clones under the mutex).
- `debug_flicker_frame_history`'s siblings that read samples for other debug
  output paths (`debug_slow_frame_history` is out of scope: slow-frame history is
  not read by render-output assertions; see Scope).

### What stays shared (deliberately)

- `SLOW_FRAME_HISTORY`, `FRAME_PERF_STATS`, `FRAME_RESOURCE_START` remain
  process-global `OnceLock<Mutex<..>>` even in tests: nothing asserted from
  rendered output reads them (verified: only `debug_slow_frame_history` /
  perf stats readers are debug commands and the smoothness benchmark, which is
  `#[ignore]`-gated and sequential). Sharing them keeps concurrency semantics
  identical for metrics that no render assertion observes, and shrinks the diff.
- The single-frame `flicker_detection_enabled()` semantics: still `true` under
  `#[cfg(test)]`.
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
6. If (1) reproduces a failure at any point, bisect with `--test-threads=1`
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
| **Thread-local flicker history (test builds)** — this plan | **Chosen** | Matches the established `TEST_*` pattern in `ui.rs`; removes the shared mutable state instead of coordinating around it; zero prod impact; doc-recommended |
| Single global render lock around `create_test_app` | Rejected (measured) | 12s → 10+ min suite; user forbids; already tried and reverted |
| Assert floors instead of exact counts in flicker tests | Rejected (measured) | Still failed 5/5 with and without; doc records this |
| Disable flicker detection in tests | Rejected | Changelog test asserts samples ARE recorded; would trade flake for lost coverage |
| Per-test unique session IDs filtering | Rejected | `maybe_record_flicker_event` keys on layout state, not session; adds cross-cutting plumbing for less isolation than thread-locality |
| Make ALL frame metrics thread-local | Deferred | Larger blast radius; slow-frame/perf stats unobserved by render assertions; unnecessary for the DoD |

## Risks and open questions

- **R1 (low)**: An undiscovered test renders frames across threads →
  deterministic empty-history failure during validation; fix by converting that
  test to render on its own thread (step 4 catches it; loop back if structural).
- **R2 (very low)**: Hidden coupling where a test *intends* to observe another
  thread's flicker events. Verified no such test exists (the only cross-thread
  observation documented is the *bug*). The validation matrix would surface it
  as a new single-thread failure.
- **Open question O1**: whether `render_state_test_lock` itself can be retired
  later now that flicker is thread-local — out of scope here; noted as follow-up.

## Out of scope

- Retiring `render_state_test_lock`; slow-frame/perf-stats thread-locality;
  unrelated 389-item quality backlog; changing any production render behavior.
