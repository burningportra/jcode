# Skill Evolution Implementation Plan

## Objective

Implement the approved model-scored Skill Evolution design as one bounded extension of the existing skill and Learning Inbox systems. Preserve the current workflow-discovery and crystallization behavior.

## Dependencies

```text
A. Evidence primitives
   ↓
B. SkillTool usage and outcome actions
   ↓
C. Evolution discovery and Learning Inbox union
   ↓
D. Proposal and approval mutations
   ↓
E. Public tests and packaged acceptance
```

## A. Evidence primitives

**Files**

- Add `crates/jcode-app-core/src/tool/skill/evolution.rs`.
- Modify `crates/jcode-app-core/src/tool/skill.rs` only for action schema and dispatch.

**Work**

1. Define versioned `UsageRecord`, `OutcomeRecord`, `EvolutionKind`, `EvolutionSuggestion`, and `EvolutionProposal` types.
2. Store bounded content-addressed JSON under `~/.jcode/skill-evolution/{usage,outcomes,suggestions,proposals,archive}`.
3. Reuse the crystallization cross-process operation lock and private-directory helpers where ownership permits. Do not introduce another lock hierarchy.
4. Implement safe identifier validation before filesystem access.
4. Validate exact canonical global paths and fingerprints from the complete normalized raw `SKILL.md`, including frontmatter. Reject project, plugin, `~/.agents`, symlinked, or shadowed sources.
5. Verify persisted session identity and locate ordinary `skill_manage` tool uses by tool-call ID in durable assistant messages. Exclude batch subcalls, provider-native synthetic calls, direct/debug execution, slash-only activation, and same-assistant-message outcomes.
6. Fingerprint complete normalized raw skill files and the bounded evidence message window strictly after the load message through the later outcome message.
7. Add isolated tests for path safety, canonical and shadowed-source checks, stale full-file fingerprints, copied sessions, missing/batched/native/synthetic tool calls, same-message outcomes, symlinks, and oversized records.

**Why first**

All later decisions depend on durable, tamper-evident evidence. Centralizing it prevents proposal and approval paths from inventing weaker validators.

## B. SkillTool usage and model outcome actions

**Files**

- Modify `crates/jcode-app-core/src/tool/skill.rs`.
- Extend `evolution.rs`.

**Work**

1. On successful ordinary persisted agent-turn `load` of a canonical, unshadowed global skill, persist a usage record best effort. Unsupported execution paths may return the skill normally but must not create eligible evidence.
2. Append a concise reporting contract and usage ID to the load output. Direct/debug executions may report that tracking is unavailable rather than inventing evidence.
3. Add `record_skill_outcome` with required `usage_id`, `outcome`, `confidence`, and `rationale`; allow `related_skill` only for `replaced`.
4. Validate enum values, confidence bounds, rationale length, related canonical skill, session identity, exact loaded path, full raw-file fingerprint, distinct ordered load and outcome assistant messages, and evidence-window digests.
5. Keep load behavior successful when tracking storage fails. Return explicit advisory metadata for tracking status.
6. Add public schema and action-field tests.

**Why active-model scoring**

The active model already sees the work, tools, corrections, and loaded instructions. A constrained reporting action adds no hidden provider request and produces structured evidence that can be validated later.

## C. Evolution discovery and Learning Inbox union

**Files**

- Extend `evolution.rs`.
- Modify `crates/jcode-app-core/src/learning_inbox.rs`.
- Modify Learning Inbox bus payloads only if tagged review instructions require it.
- Modify `crates/jcode-tui/src/tui/app/learning_inbox.rs`.

**Work**

1. Scan a bounded number of recent outcome records and verify them before aggregation.
2. Apply exact thresholds from the design:
   - refine: three corrected outcomes across distinct sessions
   - merge: three replaced outcomes naming the same related skill
   - retire: five replaced/unused outcomes, no stable replacement, and no recent helped outcome
3. Ignore confidence below 0.80 and deduplicate multiple outcomes from one session.
4. Persist immutable suggestions and separate exact-dismiss from stable-pattern suppression.
5. Introduce a tagged Learning Inbox item/source and command result so workflow and evolution suggestions coexist and the backend-resolved suggestion ID always reaches the TUI, including commands invoked without an explicit ID.
6. Select the newest pending item deterministically.
7. Route inbox, Review, Dismiss, and Never to the owning backend under one outer operation-lock owner; backend helpers must not reacquire the flock.
8. Return a kind-specific Review prompt to the TUI. Review must queue proposal drafting and stop before mutation.
9. Add regression tests proving workflow suggestions still behave unchanged.

## D. Proposal and approval mutations

**Files**

- Extend `evolution.rs`.
- Modify `skill.rs` action schema and dispatch.

**Work**

1. Add `propose_skill_evolution` accepting a suggestion ID, evolution kind, source names, optional destination name, and proposed content where required.
2. Revalidate the suggestion and all evidence before writing an immutable proposal.
3. Normalize and validate proposed skill frontmatter/content by staging the exact raw `SKILL.md` and loading it through the real registry parser before a proposal is accepted.
4. Add `approve_skill_evolution` accepting proposal ID, `confirmed`, and persisted approval evidence.
5. Require approval evidence to be a durable user message whose normalized text exactly equals `I approve skill evolution proposal <proposal-id>.`
6. Revalidate evidence and current source fingerprints immediately before mutation.
7. Persist a transaction record before the first rename. Advance it through staged, sources-archived, destination-installed, registry-verified, and finalized phases. Stage mutations privately, archive prior files, persist replacements atomically, load a fresh global registry, verify expected presence/absence/full-file fingerprints, then swap the injected registry.
8. Recover or roll back any incomplete transaction before new evolution work. Make retries accept identical completed artifacts and reject conflicts.
9. If a pre-verification step fails, roll back renamed sources and reload the prior registry. Return explicit incomplete/recovery metadata only if both forward progress and rollback fail.
10. Never mutate project-local or externally sourced skills.

## E. Verification and acceptance

**Focused tests**

- Canonical versus shadowed global load tests, plus batch/native/direct/slash exclusions and same-message outcome rejection.
- `skill_manage` public schema, load, outcome, propose, and approve tests.
- Discovery threshold and false-positive tests.
- Learning Inbox union and control tests.
- Full-frontmatter tamper, concurrent discovery/dismiss/approve, corrupt-one-source union, omitted-ID Review routing, and transaction recovery after every phase.

**Integration checks**

```bash
cargo fmt --all -- --check
cargo test -p jcode-app-core skill_evolution --lib
cargo test -p jcode-app-core learning_inbox --lib
cargo test -p jcode-app-core tool::skill::tests --lib
cargo test -p jcode-tui learning_inbox --lib
cargo check -p jcode-app-core --no-default-features
cargo check -p jcode-tui
cargo check -p jcode --bin jcode
git diff --check
```

**Packaged acceptance**

1. Build and reload the TUI using `selfdev build-reload`.
2. Use an isolated `JCODE_HOME` and persisted fixture sessions to create verified model outcomes through public `skill_manage` actions.
3. Launch the packaged TUI through a PTY and verify `/learning` renders the evolution kind and controls.
4. Exercise Review and observe the exact proposal-drafting turn without approving it.
5. Exercise Dismiss and Never in the isolated home and verify persistence across restart.
6. Run one isolated approved mutation per kind through the public tool path, verify resulting files and actual slash/load registry behavior, and confirm the real user home remains untouched.
7. Inject interruption at each transaction phase and verify restart recovery or rollback, including symlink and platform rename constraints.
7. Rerun the full suite over the final committed result and require a clean worktree.

## Commit sequence

1. `docs: design skill evolution`
2. `feat: track skill evolution evidence`
3. `feat: surface skill evolution in learning inbox`
4. `feat: approve skill evolution mutations`
5. `test: validate skill evolution feedback loop` if acceptance fixtures require a separate commit

Each implementation commit must compile and keep existing workflow discovery usable.
