# Learning Inbox implementation plan

1. Extend discovery state with bounded automatic-scan and surfaced-suggestion fields plus latest-pending lookup.
2. Add a public `jcode_app_core::learning_inbox` boundary that serializes operations with the existing crystallization lock.
3. Add a small bus payload and TUI background refresh path after successful local turns. Remote TUI sessions fail closed until discovery has a server-owned protocol.
4. Add `/learning`, `/learning review`, `/learning dismiss`, and `/learning never` to the shared command dispatcher and command catalog. Run controls off the UI thread. Review queues an agent turn that drafts through the existing proposal path and stops before approval.
5. Test backend rate limiting, persistence, deduplication, failure isolation, TUI command outputs, and post-turn refresh dispatch.
6. Run full relevant suites, build the packaged TUI, reload, and validate the real command and automatic refresh workflow.
