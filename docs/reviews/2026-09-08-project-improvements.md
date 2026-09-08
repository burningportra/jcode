# Jcode improvement review, 2026-09-08

## Scope and selection rule

Focus: the current CLI/TUI on this branch, starting at `08eb72bab`. Inspection covered shared storage, terminal initialization, clipboard helpers, provider diagnostics, web fetching, transport readers, and existing tests. This is a targeted review, not an exhaustive audit. The repository's exported bead inventory had no open items. No other branch was integrated.

An idea passes only if it addresses a concrete failure, matters to users, fits the existing architecture, and has a credible verification plan. Confidence below is an engineering judgment about net benefit, not a statistical measurement. Four improvements are selected for implementation now. The fifth needs a separate command-output compatibility change.

## First pass: 30 ideas

1. Refuse credential environment-file updates when reading existing contents fails.
2. Make JSON backup recovery read-only so readers cannot overwrite concurrent saves.
3. Install the terminal cleanup guard before any fallible initialization.
4. Include clipboard stdin delivery in the helper's timeout budget.
5. Return structured provider-doctor failures when `--json` is requested.
6. Reject parent traversal in sandbox-relative home paths.
7. Create temporary storage files exclusively rather than truncating any existing path.
8. Make Windows primary-file replacement atomic through the native replace API.
9. Add bounded line readers to daemon and debug socket protocols.
10. Report launcher, installed channel, and live daemon identities together.
11. Mark benchmark warm results unavailable when no warm samples exist.
12. Respect non-ANSI stderr in session resume hints.
13. Preserve picker highlights through Unicode lowercase expansion.
14. Reject duplicate fields in terminal exec-handoff metadata.
15. Update provider-doctor help to describe native providers as well as compatible APIs.
16. Add a size cap to HTTP response streaming.
17. Add atomic temporary-file writes for saved JSON.
18. Add backups for corrupt saved JSON.
19. Add configurable cancellation and timeout controls to shell jobs.
20. Add a new project-wide SQLite persistence layer.
21. Rewrite the daemon around event sourcing.
22. Split all remaining large crates before adding more behavior.
23. Add automatic provider switching after every failed request.
24. Add a new global command palette alongside existing slash commands.
25. Add a visual session timeline with tool replay.
26. Add session-level spending limits and preflight cost forecasts.
27. Automatically delete stale sessions and background-task artifacts.
28. Cache all tool results by argument hash.
29. Expand provider stream-contract tests without using live credentials.
30. Gate releases on startup, memory, and TUI latency budgets.

## Second pass: scrutiny of every idea

| # | Decision | Reason |
|---|---|---|
| 1 | Keep, implement | `jcode-storage::upsert_env_file_value` treats every read error as an empty file. Invalid UTF-8 can therefore erase unrelated credentials. `NotFound` is the only acceptable empty-file case. Small, deterministic fix. |
| 2 | Keep, implement | `read_json_with_recovery_handler` copies the backup over the primary during a read. A writer can publish newer data before that copy, then lose it. Removing implicit repair is simpler and safer than inventing cross-process repair locking. Parse bytes so invalid UTF-8 also reaches backup recovery. |
| 3 | Keep, implement | `src/cli/terminal.rs` enables terminal modes and uses fallible operations before constructing `TuiRuntimeGuard`. Initialization errors must unwind terminal state, just like runtime errors. Reuse the existing guard. |
| 4 | Keep, implement | Clipboard `stdin.write_all` runs before the 150 ms deadline. A non-reading child plus a large copy can freeze input indefinitely. Bound writes and polling together, with no blocked writer-thread leak. |
| 5 | Keep, plan | Provider-doctor setup and driver errors escape before JSON serialization. Automation needs a parseable failure on stdout and a nonzero status. Strong improvement, but define the output contract and tests before changing it. |
| 6 | Reject for this round | `user_home_path` allows lexical `..`, but inspected callers mostly use fixed relative paths. Rejecting traversal is sensible defense in depth, not yet a demonstrated user-facing failure. Lexical checks also do not provide symlink containment. |
| 7 | Reject for this round | Random PID/nonce names reduce collisions, but exclusive creation would strengthen the contract. A correct patch must also avoid deleting a pre-existing path on open failure. Useful follow-up, lower urgency than reproduced data loss and hangs. |
| 8 | Defer | The storage writer explicitly documents Windows' rename-away gap. Fixing it deserves Windows-native failure and sharing-mode tests. This macOS session cannot honestly validate that replacement contract. |
| 9 | Defer | Socket `read_line` paths are unbounded. Limits are worthwhile, but legitimate large messages and attachments need an explicit protocol budget before rejecting traffic. Do not pick an arbitrary cap. |
| 10 | Defer | Version reporting describes the executing binary and the repository warns about daemon mismatch. A combined view would help debugging, but channel identity is not necessarily live process identity. Inspect existing handshake/status data before adding an RPC. |
| 11 | Reject for this round | `tui_bench` can report zero warm timing for empty samples. A real benchmark correctness issue, but developer-only and less consequential than broken user workflows. |
| 12 | Reject for this round | Resume hints contain unconditional ANSI sequences. Useful polish for logs, but lower impact than state loss or a wedged terminal. |
| 13 | Defer | Unicode expansion can suppress picker highlights. Fix requires mapping folded search positions back to original characters, not merely removing a guard. Search correctness needs dedicated fixtures. |
| 14 | Reject for this round | Duplicate handoff fields are accepted, but this is private, generated exec metadata. Strictness has limited benefit without a malformed-input incident or caller requirement. |
| 15 | Reject as a standalone priority | Native provider routing exists while the help says compatible APIs. Fix alongside idea 5 so documentation and command behavior change together. |
| 16 | Reject, already present | `webfetch.rs` caps declared length and accumulated stream bytes, including unknown-length responses. Do not add a second cap implementation. |
| 17 | Reject, already present | Shared storage already uses temporary files, rename, and optional fsync. The real gap is recovery bypassing that discipline, addressed by idea 2. |
| 18 | Reject, already present | JSON backup recovery and recovery events already exist. Improve their semantics instead of adding another recovery system. |
| 19 | Reject as underspecified | Shell execution and background tasks already expose timeout, cancellation, and stall controls. Find a concrete propagation defect before adding more knobs. |
| 20 | Reject | Replacing file persistence would create migration and operational risks without evidence that storage throughput is the bottleneck. |
| 21 | Reject | A daemon rewrite would bury the identified bugs under new lifecycle complexity. No measured need justifies it. |
| 22 | Reject | The workspace already has many extracted domain crates. Blanket splitting is not an outcome. Extract a module only when a specific dependency or testing problem warrants it. |
| 23 | Reject | Blind fallback can change privacy, price, model behavior, or duplicate side effects. Retry only with explicit provider semantics and user policy. |
| 24 | Reject | A second command-discovery interface would duplicate an existing command vocabulary. First establish where slash-command discoverability fails. |
| 25 | Defer | Replay is appealing but cannot safely repeat arbitrary tool side effects. A read-only timeline needs separate interaction research and transcript-size measurements. |
| 26 | Defer | Spending control is valuable, but forecasts must handle subscription providers, unknown prices, and incomplete usage. Incorrect enforcement can stop work unexpectedly. |
| 27 | Reject | Automatic deletion risks destroying recoverable work. Prefer explicit retention rules, previews, and confirmation rather than opportunistic cleanup. |
| 28 | Reject | Identical tool arguments need not produce identical outputs. File changes, credentials, network state, and side effects make universal caching unsafe. |
| 29 | Defer | Offline stream-contract tests are valuable, but provider matrices and fixtures already exist. Identify missing contract cases before expanding tests broadly. |
| 30 | Defer | Benchmarks already exist. Stable machine-specific baselines and variance controls must precede hard CI gates, or the gate will be noisy and ignored. |

## Selected plans, ranked

### 1. Preserve credential files when reads fail

**Confidence: 99%.** Benefit is directly observable: an update must not erase information it could not read.

Location: `crates/jcode-storage/src/lib.rs`, `upsert_env_file_value`.

Change the read boundary, not the format or every auth caller:

```rust
let existing = match std::fs::read_to_string(path) {
    Ok(existing) => existing,
    Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
    Err(error) => {
        return Err(error).with_context(|| {
            format!("Failed to read environment file {}", path.display())
        });
    }
};
```

Preserve the existing key/newline validation and secret atomic writer. Add tests for invalid UTF-8 on both update and removal, a directory where a file is expected, first-time creation, replacement, removal, and unrelated-line preservation. Verify original bytes and backup remain untouched on failure. Never put credential values into diagnostics.

Downside: a corrupt file now blocks login/config update instead of silently replacing it. That is the intended tradeoff. The error must name the path and retain the underlying I/O error so users can fix the file.

This does not solve concurrent read-modify-write updates to different keys. That requires a separate cross-process serialization design.

### 2. Recover JSON without writing from the read path

**Confidence: 98%.** Readers should not destroy newer writes or truncate a primary file during repair.

Location: `crates/jcode-storage/src/lib.rs`, `read_json_with_recovery_handler`.

Use byte reads and `serde_json::from_slice` for the primary and backup. This also routes invalid UTF-8 JSON through the existing corruption path instead of returning an unrelated string-read failure. Keep the existing recovery-event API and backup preference. After decoding a backup, return the decoded value without `std::fs::copy`:

```rust
on_recovery(StorageRecoveryEvent::RecoveredFromBackup {
    backup_path: &bak_path,
});
Ok(value) // Recovery supplies data. It does not repair disk state.
```

Test successful primary precedence, corrupt primary/valid backup, invalid UTF-8, both corrupt, absent backup, and missing primary. Keep missing-primary semantics unchanged to avoid resurrecting deliberately deleted files. Use the existing recovery callback to deterministically publish a newer primary during recovery, then assert the read leaves it intact. Assert both source files remain byte-for-byte unchanged on ordinary fallback.

Downside: a corrupt primary remains corrupt until an explicit writer saves valid state. Repeated reads may repeat the warning. Automatic repair, if needed later, must have ownership/locking and a separate preservation policy for the known-good backup. A normal writer can still rotate a corrupt primary into `.bak`, which this change does not redesign.

### 3. Cover terminal initialization with the cleanup guard

**Confidence: 97%.** A setup error should not strand the shell in raw mode or leave mouse capture enabled.

Location: `src/cli/terminal.rs`, `init_tui_runtime`, `init_tui_terminal_resume`, and `TuiRuntimeGuard`.

Create the existing guard before terminal initialization and mode writes. Seed inherited mode state on exec handoff. For fresh initialization, arm keyboard cleanup before attempting the push. A failed push aborts initialization while keeping cleanup armed, because bytes may have been written before a flush error. Set idempotent mode cleanup flags before attempting their writes so partial writes also unwind. Return the same guard on successful initialization, preserving existing normal-exit and exec-handoff behavior.

Use a small initialization sequencing helper only if needed to inject errors into the exact production path. Tests must cover an early terminal-construction failure, a later mode-enable failure, successful initialization remaining armed until drop, and inherited keyboard state being cleaned without another stack push. Keep tests away from the developer's actual terminal. Run the existing handoff and guard suites plus an isolated runtime smoke test.

Downside: cleanup may run when setup changed only part of terminal state. Disabling idempotent modes is acceptable, but keyboard enhancement is stack-based and must be tracked precisely. Avoid a second independent cleanup state machine.

### 4. Bound clipboard pipe writes and helper polling together

**Confidence: 96%.** A stalled helper must not freeze the UI indefinitely.

Location: `crates/jcode-tui/src/tui/app/helpers/clipboard_helper.rs`.

Start one 150 ms deadline before sending text. Put the child stdin pipe into nonblocking mode on Unix. Write incrementally, handle `Interrupted` and `WouldBlock`, and check the deadline between attempts. Close stdin after successful delivery. Use the remaining time for early-exit polling. A delivered payload plus a still-live owner remains success. An undelivered payload is failure and must permit the existing fallback chain to continue.

On failure, kill the child and reap it. Do not spawn an unbounded writer thread that can remain blocked after the timeout. Preserve the existing long-lived-owner reaper.

Tests should run ordinary child processes, not real clipboard programs: large text into a consuming helper, text larger than pipe capacity into a finite-lived non-reader, missing executable, immediate failure, long-lived ownership, and Wayland/X11 fallback order. Compile this Unix path for tests on macOS without changing the macOS production clipboard route. Test latency with a generous margin while distinguishing a 150 ms deadline from the child's several-second lifetime.

Downside: a very slow but valid clipboard process can time out and fall through. The total fallback chain can still consume several deadlines. Spawning a process and OS scheduling are not hard real-time operations, so this is a bounded pipe/poll budget, not a guarantee that every copy completes within exactly 150 ms. Linux compositor integration remains separate from headless child-process tests.

### 5. Make provider-doctor failure output machine-readable

**Confidence: 94%. Planned, not part of this implementation.** Scripts should not lose JSON output on the failures where diagnostics matter most.

Locations: `src/cli/provider_doctor.rs` and `src/cli/args.rs`.

First define a command-failure JSON shape that does not invalidate the current successful `DoctorReport` output. For example, a separately tagged failure object:

```json
{
  "status": "error",
  "phase": "configuration",
  "provider": "example",
  "error": { "code": "missing_credentials", "message": "No credentials configured" }
}
```

Extract report construction from printing. Map missing-provider, missing-key, invalid-tier, and driver-execution errors at the command boundary. Print exactly one JSON object to stdout when `--json` is active, preserve nonzero exit status, and keep human-oriented diagnostics on stderr. Sanitize error messages so keys, authorization headers, and raw provider responses cannot leak. Preserve existing success JSON unless versioning it deliberately. Update help to include native provider routes.

Add process-level tests capturing stdout, stderr, and exit status for local configuration failures. Use an injected local driver failure for execution errors, never paid live provider calls. Parse stdout as one JSON document and assert secret sentinels are absent. Run existing native and compatible provider-report serialization tests.

Downside: failure output becomes a public interface. Its schema and exit behavior need compatibility tests and documentation. Some errors occur in argument parsing before command dispatch, which should be explicitly documented rather than promised as covered without implementation.

## Validation and delivery

### Storage checks

- Existing baseline: `cargo test -p jcode-storage` passed 4 tests before edits.
- New public-API regression suite against unchanged storage: 4 passed, 4 failed. The failures reproduced unreadable-file replacement, directory permission mutation, read-side primary replacement, and loss of a newer atomic save.
- After the fix: `scripts/dev_cargo.sh test -p jcode-storage --target-dir "$JCODE_SCRATCH_DIR/storage-safety-target"` passed all 12 tests, including the 8 new regressions. An isolated target avoided contention with the TUI compile. No mocks replaced storage or the filesystem.
- Commit: `fd7f8f549`, `fix(storage): preserve files on failed reads and backup recovery`.

### Terminal and file-picker checks

- TUI build passed with `scripts/dev_cargo.sh build --profile selfdev -p jcode --bin jcode`.
- `cargo test --profile selfdev -p jcode --lib cli::terminal::tests -- --test-threads=1` passed all 17 tests. The CLI belongs to the library target, not the binary's small test harness.
- `cargo test --profile selfdev -p jcode-tui --lib file_mention -- --test-threads=1` passed all 13 tests. Coverage includes connected and disconnected input routing, asynchronous loading and redraw, directory changes, partial listings, pending credential prompts, cursor cache invalidation, UTF-8, undo, quoting, and symlink exclusion.
- `python3 -u tests/test_file_mentions_tui.py "$PWD/target/selfdev/jcode"` passed the isolated real-TUI workflow. The test uses real typed input and PTY Enter/Tab/Esc keys, verifies popover text in full terminal renders, and uses debug frames for suggestion-state checks. It covers insertion without sending a message, draft-preserving dismissal, ignored-file exclusion, empty-result Enter handling, email addresses, and spaced filenames.
- The runtime fixture uses a private home, temporary daemon, unique socket, and a localhost-only provider endpoint without credentials. No real provider request or user-daemon mutation is needed. The fixture explicitly skips first-run onboarding. Debug-frame JSON does not contain late popover text, so assertions use actual PTY output rather than pretending composer metadata is the rendered list.
- Six existing autocomplete tests passed. A broader 51-test suggestion run initially had 49 passes, one ignored benchmark, and one macOS-specific assertion failure: the existing test expected `Alt+M`, while the formatter correctly emitted `⌥+M`. The assertion now uses the existing platform key formatter. Rerun results are recorded below.
- Commits: `b90972533` for terminal initialization cleanup and `54424904e` for the file picker and real-TUI acceptance test.

### Clipboard checks

- `cargo test --profile selfdev -p jcode-tui --lib clipboard_helper -- --test-threads=1` passed all 10 tests after correcting the macOS test fixture.
- The first run passed eight tests but failed two fallback-order tests. Fresh executable shell scripts took 184–307 ms to launch on this macOS host, compared with 11–20 ms when passed to `/bin/sh`. The fixture now invokes scripts through `/bin/sh`, retains missing-executable cases, and no longer mutates process-wide PATH. The production 150 ms deadline was not increased.
- The consuming-child test waits for its completion marker before inspecting the output file. A live clipboard owner can return success before all accepted pipe bytes have reached that file, so an immediate read was a race in the test.
- Commit: `4e295c40a`. The unrelated platform-label assertion correction is `e3d662378`.

The final compatibility rerun passed: 17 terminal tests, 10 clipboard tests, and 50 suggestion tests, with one benchmark ignored. The tested binary also passed the isolated real-TUI acceptance workflow. A source build alone does not update the running daemon; activation is a separate reload step.

## User-directed addition: `@` repository file selection

During implementation the user identified a missing daily workflow: typing `@` should allow selecting files in the active repository. Inspection confirmed this was absent, not hidden. Before this change, `get_suggestions_for` rejected every input that did not start with `/`.

This explicit user need outranks speculative new features in the original list. Implement file-path suggestions using the existing popover, without a second modal or the inline model picker that clears drafts. `@` at the cursor opens a cached, background-loaded list from the active session working directory. Typing filters it. Arrows navigate, Enter/Tab insert only the active reference, and Esc dismisses without altering the draft. Preserve UTF-8, text on either side of the cursor, undo, and paths containing spaces. Respect ignore rules and bound enumeration. Ordinary local daemon clients use their session cwd. Native SSH must not accidentally enumerate the client machine's files. This first version inserts references, not file contents.

Verify with temporary repository fixtures, input-routing tests and an isolated TUI frame. Check email addresses do not trigger it, ignored files stay absent, session cwd switches invalidate the list, and selecting a path does not submit the prompt.


Implementation is committed as `54424904e` and verified by the checks above. Confidence: **96%**. The main remaining limitations are deliberate: case-insensitive substring matching rather than fuzzy matching, nonignored workspace files rather than every tracked Git file, and no native-SSH remote catalog. Large or inaccessible catalogs show an explicit partial/unavailable notice. See [the user guide](../FILE_REFERENCES.md).
