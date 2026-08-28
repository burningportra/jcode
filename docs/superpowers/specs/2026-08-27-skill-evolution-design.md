# Skill Evolution Design

## Goal

Jcode should learn whether installed global skills help in real work and propose safe improvements without changing any skill automatically. The first production slice records explicit skill use, asks the active model for a bounded outcome classification, aggregates only repeated high-confidence evidence, surfaces refine, merge, or retire suggestions in Learning Inbox, and requires persisted user approval before any filesystem mutation.

## Scope

Included:

- Canonical global skills stored under `~/.jcode/skills/<name>/SKILL.md`.
- Explicit `skill_manage load` calls from agent turns.
- Model-scored outcomes recorded through a constrained `skill_manage record_skill_outcome` action.
- Evidence-backed refine, merge, and retire suggestions.
- Learning Inbox Review, Dismiss, and Never controls.
- Immutable evolution proposals and persisted approval evidence.
- Atomic, retry-safe skill mutation and transactional registry replacement.

Excluded:

- Project-local, plugin, `~/.agents`, or external compatibility skills.
- Background provider calls outside an active agent turn.
- Automatic mutations.
- Free-form model confidence below the production threshold.
- Ranking or optimizing skills from token counts, latency, or subjective prose quality alone.

## User workflow

1. The model explicitly loads a canonical global skill.
2. The load output includes a bounded usage ID and instructs the model to report one outcome after applying the skill.
3. The model calls `record_skill_outcome` with `helped`, `corrected`, `replaced`, or `unused`, a confidence score, a concise rationale, and an optional related skill.
4. Jcode verifies the usage record, session identity, original load tool call, skill fingerprint, and outcome tool call before persisting the outcome.
5. Automatic Learning Inbox refresh aggregates recent verified outcomes.
6. Repeated high-confidence evidence creates one immutable suggestion:
   - **Refine**: at least three `corrected` outcomes from distinct sessions for one skill.
   - **Merge**: at least three `replaced` outcomes from distinct sessions naming the same canonical related skill.
   - **Retire**: at least five `replaced` or `unused` outcomes from distinct sessions, no stable replacement candidate, and no recent high-confidence `helped` outcome.
7. `/learning` shows the suggestion and evidence count.
8. Review queues an agent turn that reads the verified evidence and drafts an exact evolution proposal. It does not mutate a skill.
9. The user explicitly approves the immutable proposal in a persisted user message.
10. `approve_skill_evolution` revalidates the proposal, evidence, current skill fingerprints, and approval message, then performs the mutation atomically and reloads the injected registry transactionally.

## Evidence model

### Usage record

A usage record contains:

- schema version
- content-addressed usage ID
- session ID
- load tool-call ID
- skill name
- canonical skill path
- complete normalized skill-content fingerprint
- creation timestamp

A usage record is accepted only when:

- session and tool-call identifiers are bounded safe components
- the persisted session ID matches the requested session
- the session contains a `skill_manage load` tool use with the recorded tool-call ID and skill name
- the current canonical skill fingerprint matches the record at creation

### Outcome record

An outcome contains:

- content-addressed outcome ID
- usage ID
- outcome class: `helped`, `corrected`, `replaced`, or `unused`
- confidence in `[0, 1]`
- rationale bounded to 500 characters
- optional related canonical skill name
- outcome tool-call ID
- digests of the bounded persisted conversation messages between load and outcome calls
- creation timestamp

Only confidence `>= 0.80` contributes to suggestions. Lower-confidence outcomes remain available for diagnostics but never trigger proposals. Outcome recording is best effort and must never fail the user turn merely because the ledger is unavailable.

The evaluator is the active model itself. The load result gives it a narrow reporting contract. Jcode does not start a second provider request, which avoids hidden cost, provider routing ambiguity, and lifecycle coupling.

### Tamper resistance

Review and approval reload the referenced session, verify its identity, locate the recorded tool calls, recompute every message digest, and recompute current skill fingerprints. Changed, missing, copied, renamed, oversized, malformed, or unsafe evidence fails closed.

## Suggestion model

Evolution suggestions are stored separately from repeated-workflow suggestions under `~/.jcode/skill-evolution/suggestions/`. Each contains:

- content-addressed suggestion ID
- action kind
- target skill names and fingerprints
- verified outcome references and digests
- concise summary
- creation timestamp

The Learning Inbox application boundary returns a tagged item so the TUI remains independent of storage details. It selects the newest pending item across workflow and evolution sources. Dismiss suppresses only the exact suggestion. Never suppresses the stable action-and-target pattern until the evidence fingerprint changes materially.

## Review and proposal flow

Review does not ask the TUI to edit files. It queues an agent instruction specific to the suggestion kind:

- Refine: produce a replacement `SKILL.md` for the existing skill.
- Merge: produce one canonical destination skill and identify both source skills.
- Retire: explain why removal is justified and propose archival.

The agent calls `propose_skill_evolution`. The proposal is immutable and content-addressed. It includes exact before fingerprints, exact proposed content where applicable, all verified evidence, and the requested mutation.

`approve_skill_evolution` requires `confirmed=true` and persisted approval evidence whose message explicitly contains the proposal ID. The approval action is the only public operation that mutates skill files.

## Mutation semantics

All mutation operations share the existing cross-process crystallization lock and the injected `Arc<RwLock<SkillRegistry>>`.

- **Refine**: write candidate content to a private temporary file, archive the old skill, atomically persist the new file, load a fresh global registry, verify the new fingerprint, then swap the injected registry.
- **Merge**: validate both source fingerprints and destination non-conflict, stage the destination, archive both sources, persist the destination, load and verify a fresh registry, then swap.
- **Retire**: archive the source, load a fresh registry, verify absence, then swap.

Every operation is retry-safe. Existing identical artifacts are accepted. Conflicting artifacts fail closed. If registry verification fails after filesystem installation, the output reports an explicit incomplete state with recovery paths rather than claiming success.

## Failure behavior

- Skill load succeeds even if usage recording fails.
- Outcome recording returns a visible advisory error but does not invalidate work already completed.
- Automatic discovery logs failures and emits no suggestion.
- Malformed evaluator output cannot create a suggestion.
- One corrupt ledger or suggestion file is skipped or reported without erasing valid state.
- Explicit remote TUI workspaces remain fail closed until the server owns the evidence protocol.

## Testing and acceptance

Requirements map to checks:

1. Explicit canonical load records usage; project and external skills do not.
2. Valid high-confidence model outcomes persist; invalid classes, confidence, related skills, identifiers, and stale fingerprints fail.
3. Session/tool-call/message tampering is detected.
4. Thresholds create refine, merge, and retire suggestions only across distinct sessions.
5. Helped evidence and low-confidence evidence prevent false-positive retirement.
6. Learning Inbox lists, reviews, dismisses, and suppresses evolution suggestions without regressing workflow suggestions.
7. Review queues the correct drafting instruction and stops before mutation.
8. Proposal creation is immutable and deduplicated.
9. Approval requires persisted user evidence and revalidates all inputs.
10. Refine, merge, and retire mutations are atomic, retry-safe, registry-transactional, and recoverable.
11. No-default builds and packaged TUI builds pass.
12. The active packaged TUI visibly shows an evolution suggestion in an isolated acceptance home and exercises Review, Dismiss, and Never without mutating the real user home.

## Complexity limits

The implementation adds one focused evolution module and extends the Learning Inbox with a tagged source enum. It does not add a second provider client, scheduler, database, configuration surface, or generic event framework. Storage remains bounded JSON with content-addressed records and existing atomic-file patterns.
