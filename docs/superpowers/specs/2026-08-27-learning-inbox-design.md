# Learning Inbox

Date: 2026-08-27
Status: approved for implementation

## Outcome

Jcode should notice repeated workflows after successful turns without interrupting the conversation. A durable Learning Inbox exposes one pending suggestion at a time with **Review**, **Dismiss**, and **Never suggest this** controls. Review continues through the existing evidence-backed crystallization and explicit approval path.

## User interface

The TUI adds `/learning`:

- `/learning` shows the newest pending suggestion or an explicit empty state.
- `/learning review [suggestion-id]` revalidates evidence and starts an agent turn that drafts through the existing `crystallize` proposal path without approving or installing it.
- `/learning dismiss [suggestion-id]` dismisses only the current evidence snapshot.
- `/learning never [suggestion-id]` suppresses the normalized workflow pattern.

After a successful local turn, the TUI launches a background refresh. A newly surfaced suggestion adds one compact transcript notice and a status notice: `Learning Inbox: 1 suggestion · /learning`. It never modifies the composer, opens a modal, or starts approval automatically. Remote TUI sessions report that the inbox is unavailable rather than reading or mutating the client machine's unrelated session store.

## Backend behavior

- Reuse the existing exact-normalized, three-session proactive detector.
- Scan at most once per hour across the installation, regardless of how many sessions finish.
- Persist the last automatic scan and last surfaced suggestion in the existing bounded discovery state.
- Return only a suggestion not already dismissed or suppressed.
- Repeated refreshes for the same suggestion are silent.
- Corrupt state or scan failures are logged and do not affect turn completion.

## Architecture

`jcode-app-core::learning_inbox` is the public application boundary. It owns automatic refresh, latest pending lookup, and control actions while reusing the existing crystallization filesystem lock.

The TUI runs local refresh work off the UI thread and receives a small `BusEvent::LearningInboxUpdated` payload. A future remote implementation must run discovery where the server-owned persisted sessions live.

## Acceptance requirements

1. A successful local turn requests a nonblocking refresh.
2. A new suggestion produces one compact visible notice.
3. The same suggestion does not repeatedly notify.
4. Restarting preserves pending, dismissed, suppressed, and rate-limit state.
5. `/learning review`, `dismiss`, and `never` exercise the real application boundary.
6. Empty inboxes and corrupt state fail safely and visibly.
7. Existing `skill_manage` discovery and approval behavior remains unchanged.
