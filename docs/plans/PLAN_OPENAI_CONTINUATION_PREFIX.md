# OpenAI continuation prefix integrity

## Goal and elicitation

Debug and fix the user's current invalid-request failure, preserving tool history and working continuation performance. This is a bounded repair with a compressed goal pipeline: evidence, short reviewed plan, executable todos, implementation, verification, distillation. Tgrep integration is paused. No credential changes, transcript surgery or unrelated provider redesign.

## Evidence

At 2026-09-08 08:18:41 local time the session's todo result was successfully persisted. A queued user message was injected immediately afterward. The canonical request then had 70 items, versus 68 previously, but its unchanged prefix ended at item 66. Despite `prefix_matches=false`, the persistent WebSocket path sliced at 68 and sent only two message items and zero tool outputs. OpenAI rejected the missing todo callback. Saved transcript independently contains both tool call and result. This rules out a failed todo execution and establishes an unsafe incremental request. Source follow-up identifies the trigger: messages_with_dynamic_system_context relocates its reminder after the newest fresh user prompt, while per-request ephemeral memory can disappear. The old canonical suffix [reminder, memory] became [todo call, todo result, new user, reminder]. This explains growth by two while the prior prefix changes.

`PersistentWsState` stores only `last_input_item_count`. `persistent_ws_incremental_items` blindly slices by that count. A longer request is not proof that the prior input is unchanged. A subsequent previous-response-not-found error was also logged; do not conflate it with the first rejection.

## Proposed contract

Continuation may slice prior input only when the entire previously sent canonical input is an unchanged prefix. Store a compact canonical input fingerprint alongside the count, using the project's existing fingerprint machinery if suitable. Compare the corresponding prefix before generating a delta. On mismatch clear persistent state and return the existing NotAvailable result so the established full-request path sends the complete call/result history. No partial callback-only repair and no silent retry of completed tools.

Maintain this state at fresh successful response and every successful continuation. A generate:false prewarm has empty prior input and must retain existing first-generation semantics. Failed/dropped responses must not advance count or fingerprint. Do not log input bodies or credentials.

## Alternatives and review gate

Count-only reuse is disproven by incident evidence. Disabling all WebSockets is broader and sacrifices valid reuse. Fingerprint validation with conservative full replay is the proposed minimum. Independent read-only reviewer goat checked the current implementation and found no blocking design gap. This compressed repair used one independent session, not a verified cross-model review. Architecture/failure review and subsequent state/simplicity review found no structural gaps after these requirements were incorporated. No cross-model review was performed, which remains a review limitation.

## Execution and test obligations

1. Review all PersistentWsState constructors and existing fingerprint behavior. Confirm mismatch reset precedes delta send.
2. Regression uses the actual messages_with_dynamic_system_context and build_responses_input builders with old ephemeral memory and new user input, reproducing 68->70 items and divergence at66. Growing input whose earlier prefix is replaced/reordered must decline reuse, including tool output moved before old cutoff. Append-only input must still reuse. Test unchanged-count/shrinking input and empty prewarm semantics.
3. Exercise real loopback transport: changed prefix must not send previous_response_id delta and must retain the callback in the fresh request. Existing continuation/prewarm tests must pass. Do not call a paid provider merely to reproduce deterministic request construction.
4. Run focused crate tests and coordinated TUI build/reload. Verify installed identity and report exact tested boundaries.

## Convergence and handoff

No beads required for this bounded single-session repair. Todo ordering is evidence -> reviewed plan -> implementation/regression -> build and verification -> distillation. Reviewer requirements are incorporated: fingerprint FULL canonical input, never the filtered delta; update fingerprint/count/response ID together only after completed success in both paths; initialize all prewarm/test constructors; bounds-check before slicing; validate before any response.create send. The existing stable_hash_json serializes the same Value deterministically and is suitable as a conservative internal fingerprint. Loopback acceptance must inspect the actual fallback payload for both call and result. Implementation gate passed. The source transformation is identified above. Its dynamic placement is legitimate and must not be treated as an append-only transport history. Broader history normalization changes are deferred unless the prefix guard is insufficient.

## Observed verification

- Incident log assertions passed: completed callback, changed prefix at66 before old cutoff68, two-message delta with zero callbacks, matching provider rejection.
- Real loopback `persistent_ws_changed_prefix_sends_no_delta_and_replays_tool_pair` passed. Actual context/canonical builders reproduce68->70; old socket receives no delta, provider.complete fallback carries the full matching pair, and a subsequent append-only request reuses the new response successfully.
- `persistent_ws_prefix_fingerprint_checks_full_input_and_bounds` and `persistent_ws_unchanged_or_shrinking_input_clears_state` passed. Existing cancellation, identity and prewarm checks passed in the final suite.
- Final command: `env -u OPENAI_BASE_URL -u OPENAI_API_BASE -u JCODE_OPENAI_API_BASE cargo test --profile selfdev -p jcode-provider-openai-runtime --lib -- --test-threads=1`: **129 passed, zero failed, three ignored**. Ignored tests require live credentials, not exercised here.
- Initial run was127 passed/two failed/three ignored. One existing identity-backpressure fixture had a prior-input fingerprint inconsistent with its request and was corrected to reach its intended guard. The endpoint default test inherited OPENAI_BASE_URL; only the test subprocess environment was cleared. No user configuration or credentials changed.
- Independent read-only production/test diff review found no concrete defects. Live installation is a separate coordinated build/reload step, not inferred from unit tests.

Distillation: ephemeral prompt context can move or disappear between otherwise-growing turns. Transport deltas must validate their canonical prefix rather than infer append-only history from counts. Valid history transformations should cause safe full replay, not callback loss.


Activation verified: running/current/shared-server revision is b60d5f50f after coordinated build/reload. Live logs subsequently recorded `persistent_state_reset reason=input_prefix_changed` while this conversation continued successfully. This confirms the new guard executed outside the loopback tests.
