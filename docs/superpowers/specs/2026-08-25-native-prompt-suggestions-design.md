# Native Prompt Suggestions Design

**Date:** 2026-08-25
**Status:** Approved, amended for default-on rollout
**Reference:** [`guwidoe/pi-prompt-suggester`](https://github.com/guwidoe/pi-prompt-suggester)

## Summary

Jcode will generate a likely next user prompt after every successfully completed interactive assistant turn and display it as ghost text in the empty TUI composer. Generation will run asynchronously in the daemon, while clients remain responsible for presentation and explicit acceptance.

The first version will reuse Jcode's existing session, repository, memory, todo, and project-guidance context rather than introducing a separate repository-seeding subsystem.

## Goals

- Generate one relevant next-prompt suggestion after each successful interactive turn.
- Render suggestions as non-editable ghost text when the composer is empty.
- Accept visible ghost text with `Tab` or `Right Arrow`.
- Preserve existing key behavior whenever no compatible ghost suggestion is visible.
- Provide consistent behavior for local and remote TUI sessions.
- Keep suggestion latency, failures, and cancellations outside the critical turn-completion path.
- Use a lightweight default model route with global and project-level overrides.
- Keep the feature easy to disable globally or per workspace for users who do not want the additional model request.

## Non-goals

- Automatically submit or execute suggested prompts.
- Suggest prompts during active streaming or internal tool turns.
- Generate suggestions for debug, scripted, or other non-interactive sessions.
- Add a general extension framework before shipping this feature.
- Extract a standalone plugin or public plugin repository before the integration boundary has stabilized.
- Add an agentic project-seeding pipeline in the first version.
- Persist raw suggestion prompts in observability logs.

## Architecture

The feature is split between daemon-owned generation and client-owned presentation.

### Daemon responsibilities

A new `PromptSuggestionService` will:

- observe finalized, successful interactive turns;
- build compact suggestion context from the active session;
- select the configured lightweight model route;
- run generation in a cancellable background task;
- validate and bound model output;
- discard stale results;
- publish session-scoped suggestion events;
- expose structured, privacy-safe diagnostics.

The service must not live in the TUI. Keeping generation in the daemon gives local and remote clients the same behavior and avoids coupling model execution to composer rendering.

### TUI responsibilities

The TUI will:

- store the latest suggestion separately from editable input;
- render compatible suggestions as dim ghost text;
- accept a visible suggestion with `Tab` or `Right Arrow`;
- preserve model switching and cursor movement when no suggestion is accepted;
- clear incompatible or obsolete suggestion state;
- request redraws when suggestion state changes.

Suggestion rendering should be factored into focused helpers rather than expanding the responsibilities of the existing `ui_input.rs` composer path unnecessarily.

### Protocol

The daemon-to-client protocol will add a session-scoped event equivalent to:

```rust
PromptSuggestionUpdated {
    session_id: SessionId,
    generation: u64,
    suggestion: Option<String>,
}
```

`None` explicitly clears a previous suggestion. The exact protocol type may follow existing server event naming and serialization conventions.

## Data flow

1. A successful interactive assistant turn reaches final completion in the daemon.
2. The daemon increments the session's suggestion generation number.
3. `PromptSuggestionService` snapshots the eligible context and starts background generation.
4. The configured model returns one suggestion or the no-suggestion sentinel.
5. The service validates session identity, generation number, length, and output shape.
6. The daemon emits a session-scoped suggestion update.
7. The active TUI stores the result and redraws.
8. The composer renders ghost text only while its compatibility rules remain true.
9. `Tab` or `Right Arrow` copies the full suggestion into editable input without submitting it.

## Eligibility and rendering rules

Generation is eligible only when all of the following are true:

- the turn completed successfully;
- the session is interactive;
- the session is not running in debug or scripted mode;
- prompt suggestions are enabled by effective configuration;
- the completed turn belongs to a live session that can receive suggestion events.

Ghost text is visible only when:

- a current suggestion exists for the active session;
- the editable composer is empty;
- no modal, picker, prompt-history search, command suggestion overlay, or incompatible input mode owns the composer;
- no newer user input or turn has invalidated it.

Multiline suggestions are allowed and use the normal composer wrapping rules. The configured maximum character count bounds both generation and rendering.

## Acceptance and invalidation

When compatible ghost text is visible:

- `Tab` accepts the suggestion into editable input;
- `Right Arrow` accepts the suggestion into editable input;
- acceptance does not submit the prompt;
- acceptance places the cursor at the end of the inserted text.

When no compatible suggestion is visible:

- `Tab` retains its current autocomplete behavior;
- `Right Arrow` retains normal cursor movement behavior.

A suggestion is cleared or invalidated by:

- any user text insertion or paste;
- starting or submitting another turn;
- switching sessions or branches;
- disconnecting from the owning session;
- receiving a newer suggestion generation;
- entering an incompatible composer mode;
- disabling the feature through configuration.

## Context and prompting

The first version will use compact context rather than a full transcript or separate seed database. Inputs should include, within a strict token budget:

- recent user and assistant turns;
- unresolved questions or explicit next steps from the latest completion;
- current todo or task state when available;
- working directory and concise repository identity;
- applicable project guidance already loaded by Jcode;
- recent accepted suggestion feedback if it can be added without introducing new persistence complexity.

The model instruction will require exactly one likely next user prompt as plain text, or a stable no-suggestion sentinel such as `[no suggestion]`. Generated text must not include explanation, quotation marks added only for presentation, or multiple alternatives.

## Model routing and configuration

The default route will be a fast, inexpensive model selected from routes already available to Jcode. Users can override it globally or per project.

Initial configuration surface:

- `prompt_suggestions.enabled`
- `prompt_suggestions.model`
- `prompt_suggestions.reasoning_effort`
- `prompt_suggestions.max_chars`
- `prompt_suggestions.acceptance_keys`

Defaults:

- enabled by default for compatible interactive clients;
- disabled for non-interactive, debug, and scripted sessions;
- lightweight model route;
- low or minimal reasoning effort;
- both `tab` and `right_arrow` acceptance keys;
- a conservative maximum length suitable for the TUI composer.

Project configuration overrides global configuration using Jcode's existing precedence conventions. Invalid routes or settings should fall back safely and surface through status or debug diagnostics rather than breaking a turn.

## Concurrency and cancellation

Each session owns a monotonically increasing generation number. Every request captures both the session ID and generation.

Pending work is cancelled when:

- a newer eligible turn completes;
- the user begins new input;
- the session changes or closes;
- the relevant connection is lost;
- the feature becomes disabled.

Cancellation is best effort. A completed result is published only if its session and generation still match current state. This final stale-result check is mandatory even when task cancellation succeeds.

Suggestion work must not hold locks needed by turn finalization, event dispatch, input handling, or rendering.

## Failure handling and observability

Suggestion failures are non-fatal and silent in normal interaction. They must never delay or change assistant turn completion.

Debug/status observability may record:

- request start and completion;
- selected route and reasoning effort;
- latency and token usage when available;
- cancellation reason;
- stale-result rejection;
- output validation rejection;
- configuration fallback.

Diagnostics must not persist the raw conversation context or generated suggestion text by default.

## Testing strategy

### Unit tests

- effective configuration and project override precedence;
- eligibility classification for interactive, scripted, debug, successful, failed, and aborted turns;
- output validation, sentinel handling, and maximum length;
- generation number ordering and stale-result rejection;
- cancellation state transitions;
- ghost compatibility rules;
- `Tab` and `Right Arrow` acceptance;
- fallback to existing autocomplete and cursor movement;
- multiline wrapping and cursor placement.

### Integration tests

- local turn completion triggers one background generation request;
- remote turn completion produces the same session-scoped event and rendering state;
- a newer turn supersedes an older pending request;
- input typed before completion prevents stale ghost text from appearing;
- model errors and unavailable routes do not affect turn completion;
- configuration changes clear or enable suggestions immediately where supported.

### Runtime verification

Build and run the changed binary against a dedicated daemon socket so verification cannot accidentally exercise the shared old daemon. Use debug-socket tester sessions and frame assertions to verify:

- ghost text appears after a completed turn;
- multiline ghost text renders and wraps correctly;
- `Tab` accepts when visible and retains autocomplete behavior otherwise;
- `Right Arrow` accepts when visible and moves the cursor otherwise;
- typing and session switching clear the ghost;
- remote and local sessions behave consistently.

## Architectural constraints discovered

- `crates/jcode-tui/src/tui/ui_input.rs` is already a large rendering module. Generation, lifecycle management, and model routing must not be added there.
- `Tab` already participates in autocomplete, so suggestion acceptance must be a narrow conditional that applies only to visible, compatible ghost text. Model switching remains on `Ctrl+Tab`.
- Provider `MessageEnd` can precede final bookkeeping. Suggestion generation must trigger from finalized turn completion, not the first message-end signal.
- Runtime verification must use the rebuilt binary and a dedicated socket or coordinated self-dev reload. A plain `cargo build` does not validate the active daemon behavior.

## Rollout

Ship the built-in integration enabled by default for compatible interactive clients. Users can opt out globally or per workspace with `prompt_suggestions.enabled = false`. Keep generation failures invisible to users, but expose sufficient debug metrics to evaluate latency, cost, cancellation frequency, and no-suggestion rate.

Treat the built-in implementation as the proving ground for a future plugin boundary. A standalone public plugin repository is deferred until the generation context, lifecycle hooks, protocol events, configuration contract, and client presentation API are stable enough to support third-party consumers without copying Jcode internals.
