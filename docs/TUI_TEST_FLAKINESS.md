# jcode-tui test flakiness: root cause

> **RESOLVED 2026-09-01.** The flicker/render-state race described below is fixed
> (thread-local state in test builds; render-state test lock converted to a
> per-thread flag). The original flaky module (`frame_flicker`) is now stable
> under 16-thread stress. See "Resolution" at the bottom. The analysis below is
> retained as the record of the root cause.

`cargo test -p jcode-tui --lib` fails 1-4 tests per run, with a varying set.
This is a parallelism race on process-global state, not a logic bug.

## Evidence

- `cargo test -p jcode-tui --lib -- --test-threads=1` passes **2006/2006** (16 ignored).
- The failing set changes between runs at the default thread count.
- Individually, each failing test passes when run alone.

Counts were taken on 2026-07-27 and will drift as tests are added. Reproduce
on an otherwise idle machine: under memory pressure (this host has 15 GiB and
was running concurrent workspace builds) `cargo` gets SIGTERMed mid-compile,
which is a different failure from the race described here.

## Root cause

`create_test_app()` (and its `create_named_provider_test_app` sibling) in
`crates/jcode-tui/src/tui/app/tests/support_failover/part_01.rs` calls:

```rust
crate::tui::ui::clear_test_render_state_for_tests();
```

That wipes **process-global** render state: the flicker frame history, layout
snapshots, status-area snapshots, copy targets, and scroll positions.

Rendering tests guard exactly that state with `render_state_test_lock()`. But
`create_test_app` clears it *without* taking the lock, so any of its ~810 call
sites can reset a concurrently-running render test's state mid-assertion.

The mechanism for the most frequent victim
(`test_changelog_overlay_repeated_renders_are_stable`) is documented in
`clear_test_render_state_for_tests` itself: a recorded flicker event adds a
"⚠ flicker detected" notification line to later renders, shifting every
layout-sensitive assertion by a row.

### Bisected proof

Bisecting the 959 `tui::app::tests::` tests against the changelog test
identifies `test_tui_login_providers_have_real_tui_handlers`, which calls
`create_test_app()` in a loop (once per login provider). Running just those two
does not reproduce; the race needs enough concurrent load to interleave, which
is why it presents as order-dependent flakiness.

## What does not work

**Taking `render_state_test_lock` inside `create_test_app`.** This is correct
but serializes all ~810 call sites: suite runtime goes from ~12s to over 10
minutes. Measured, then reverted.

**Asserting a floor instead of an exact count** in the changelog test's
`buffered_samples` check, and **calling `clear_test_render_state_for_tests`**
at the top of that test. Both measured over 5 runs: the test still failed 5/5
with *and* without the change. Reverted rather than committed as churn.

## Suggested direction

The real fix is to stop sharing this state across tests rather than to
serialize access to it:

1. Make the render state thread-local rather than process-global, so parallel
   tests cannot observe each other's resets. Production has one render thread,
   so this should not change runtime behavior.
2. Failing that, have `create_test_app` skip the render-state clear entirely.
   Only rendering tests depend on it, and they already clear it under the lock.
   This needs an audit of which app tests implicitly rely on the current clear.

Option 1 is preferred: it removes the shared mutable state instead of adding
coordination around it.

## Scope note

This is pre-existing and independent of the render-path performance work in
commits `0ba0154c6`, `2b8e78e34`, `8b44fc83b`, `8142f1a0b`. Verified by
stashing those changes and reproducing the same failure rate.

## Resolution (2026-09-01)

Implemented per `docs/plans/PLAN_TUI_THREAD_LOCAL_FLICKER_STATE.md` (converged
through a verified cross-model review by claude-fable-5). Three changes:

1. **Thread-local flicker history and perf stats in test builds**
   (`ui_frame_metrics.rs`). The scroll/layout/copy state was already
   thread-local; the flicker history was the last process-global store that
   rendered output observed, and the perf-stats snapshot that flicker samples
   are *built from* had to be split too (a sibling's `note_viewport_metrics`
   write landing between two of a thread's frames fabricates flicker events in
   that thread's own history). Production paths under `#[cfg(not(test))]` are
   unchanged.

2. **Lock-free clear for app construction** (`support_failover/part_01.rs`).
   `create_test_app` and siblings now call
   `clear_test_render_state_for_tests_unlocked()`, which clears only the
   calling thread's state — no lock needed once everything is thread-local.

3. **`render_state_test_lock` is now a per-thread flag that never blocks**
   (`ui.rs`). Once all guarded state was thread-local, the lock serialized
   nothing across threads; keeping it as a real mutex only preserved a
   cross-thread lock-order inversion (env lock vs render lock, see the plan's
   addendum for the `sample(1)` forensics) that deadlocked entire parallel
   suite runs — a second, distinct defect discovered during validation and
   fixed in the same effort.

### Verified results

- Parallel suite runs complete in ~21-30s (previously: 1-4 flakes per run, and
  after an unrelated lock-ordering drift, full-suite deadlocks of 50+ minutes).
- `frame_flicker` tests: green 8/8 runs under 16-thread oversubscription
  (previously the most frequent flaky module).
- Single-threaded: failure set byte-identical before/after the fix
  (2189 passed / 38 failed both ways, sorted diff) — zero regressions.

### Remaining known baseline (out of scope here)

38 tests fail single-threaded on the *unchanged* baseline tree (control-proven
identical pre/post fix). These are environment/platform-dependent (e.g.
`Alt+Shift+I` vs `⌥+Shift+I` key-label assertions, JCODE_HOME/config state),
unrelated to render state. A further ~5-9 parallel-only failures are
JCODE_HOME/config pollution among tests that never took the env lock —
previously masked by the render lock accidentally serializing everything.
Both groups need an env-isolation pass (tests that mutate `JCODE_HOME` or
config must hold `lock_test_env`) and are tracked as a follow-up goal, not
part of the flicker fix.
