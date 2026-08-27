# Jcode product roadmap

Date: 2026-08-27
Status: approved direction
Priority order: cockpit, trusted autopilot, compounding engineering partner

## North star

Jcode should be the place where engineering work becomes understandable, steerable, trustworthy, and increasingly self-propelling.

The product loop is:

1. See active work across projects and agents.
2. Direct outcomes rather than repeatedly authoring prompts.
3. Execute safely with explicit autonomy and policy.
4. Report evidence instead of status theater.
5. Accumulate reusable judgment, memory, and workflows.

## Completed foundations

- Corrected model context limits and context-meter behavior.
- Added report-only sourced complexity and anti-slop quality evidence.
- Added the safe FFF indexed AgentGrep route with linked fallback.
- Enabled FFF `prefer` mode by default after parity, mutation, resource, packaging, and performance acceptance.

These are foundations, not substitutes for the product roadmap below.

## Next implementation slice

Build the smallest shared product kernel that can support the cockpit without creating a second orchestration system.

### Contracts

Formalize six first-class concepts:

- `Project`: repository, goals, policies, memory, work history, sessions, and health.
- `Mission`: durable desired outcome with success criteria, constraints, policy, and work DAG.
- `WorkNode`: typed investigation, decision, implementation, verification, review, integration, release, monitoring, or approval unit.
- `Artifact`: durable patch, commit, test result, benchmark, decision, finding, design, screenshot, reproduction, handoff, or release candidate.
- `AttentionItem`: a decision that genuinely requires human judgment.
- `Policy`: scoped autonomy, risk, validation, cost, provider, and approval rules.

Map existing sessions, todos, swarms, background tasks, schedules, and memory onto these contracts. Do not introduce parallel replacements.

### First vertical feature

Deliver a minimal Attention Inbox projection backed by those contracts:

- requested decision
- reason Jcode cannot decide safely
- recommendation and alternatives
- consequences and evidence
- urgency
- whether other work can continue
- accept, choose alternative, delegate, defer, inspect, and open-session actions

This slice proves that the contracts serve a real end-user workflow before expanding the TUI.

## Ranked product bets

1. Project, Mission, WorkNode, Artifact, AttentionItem, and Policy contracts.
2. Attention Inbox.
3. Mission Control.
4. Multi-surface TUI workspace.
5. Proof-carrying completion artifacts.
6. Native DAG navigator.
7. Scoped autonomy policy engine.
8. Portable context capsules.
9. Semantic activity ledger and replay.
10. Ambient stewardship domains.

## Phases

### Phase 0: Product kernel and contracts

- Map existing runtime concepts onto the shared contracts.
- Add versioned semantic domain events for meaningful state transitions.
- Add active-work and attention read projections.
- Add schema migration and replay fixtures.
- Add architecture fitness tests.

Acceptance: the Attention Inbox vertical slice uses existing work state through the new contracts without duplicating orchestration or persistence.

### Phase 1: Command center

- Attention Inbox.
- Mission Control answering what is running, changed, blocked, needs attention, and likely to finish next.
- Multi-surface workspace for sessions, diffs, tests, artifacts, evidence, and health.
- Native DAG navigator.
- Live evidence panel.
- Change topology for overlapping agent work.
- Persistent workspace layouts.
- Model and budget summary.

Acceptance: a user supervising ten agents can understand the situation and make the most valuable decision within thirty seconds.

### Phase 2: Trust and orchestration

- Editable outcome contracts.
- Reusable verification gates.
- Proof-carrying changes with requirement-to-check traceability.
- Observe, Draft, Execute, Integrate, Deliver, and Steward autonomy levels.
- Structured checkpoint negotiation.
- Working-set context capsules.
- Rehearsal and shadow-policy modes.
- Typed competing-agent deliberation.
- Replayable mission timeline.

Acceptance: “done” has a consistent, inspectable meaning across models and providers.

### Phase 3: Trusted autopilot

- Stewardship domains for dependencies, flaky tests, documentation, performance, provider catalogs, releases, security, memory, architecture drift, and coverage gaps.
- Opportunity scoring and budget-aware scheduling.
- Automatic rollback points.
- Outcome measurement.
- Attention batching and daily briefings.
- Continuous engineering radar.
- Cross-project scheduling.

Acceptance: Jcode advances projects for days without surprise, noise, or unexplained changes.

### Phase 4: Compounding engineering partner

- Context compiler producing the smallest sufficient provenance-linked capsule.
- Inspectable personal engineering model.
- Skill and workflow crystallization.
- Incremental project digital twin.
- Confidence and uncertainty map.
- Expectation-gap detector.
- Repository scent trails.
- Maintenance packets and decision-pattern learning.

Acceptance: the hundredth mission is materially easier, faster, and better aligned than the first.

### Phase 5: Ecosystem and portability

- Stable capability and extension APIs.
- Shareable workflows, proof packets, and mission capsules.
- Cross-device continuation and optional encrypted synchronization.
- Team policy overlays and provenance-aware shared memory.
- Remote execution pools.
- External clients using the same projections.

This phase begins only after the solo experience is exceptional.

## Scalable architecture rules

- Keep the shared server authoritative for projects, missions, sessions, DAGs, artifacts, policies, schedules, memory, events, and provider resources.
- Persist semantic domain events, not keystrokes or raw UI state.
- Use typed artifacts where orchestration depends on structure. Keep chat for human communication.
- Feed the TUI small projection-specific read models rather than mirroring the server.
- Centralize approval, validation, budget, provider, destructive-action, and ambient permissions in a policy engine.
- Publish machine-readable capabilities for tools, providers, surfaces, and workflow handlers.
- Use one engine with presets instead of separate interactive, ambient, local, remote, light, and deep implementations.
- Give durable records stable IDs, schema versions, migrations, replay fixtures, and forward-compatible handling.
- Enforce dependency, protocol, projection, event-compatibility, deterministic-scheduling, compile-time, and binary-size boundaries in CI.

## Explicit non-goals for the next slice

- No generated project wiki or full digital twin.
- No new plugin framework.
- No replacement for sessions, todos, swarms, schedules, or memory.
- No large TUI redesign before the contracts support a working Attention Inbox.
- No autonomous publishing or deployment policy.
- No generic AI-generated cleanup backlog.

## Product metrics

- Time to understand active work.
- Time to identify the highest-value intervention.
- Interruptions per completed mission.
- Percentage of attention items resolved in one action.
- Agents safely supervised per user.
- Unattended completion and rollback rates.
- Surprise-action rate.
- Useful work per unit of human attention.
- Requirement-to-evidence coverage.
- Percentage of repeated workflows crystallized into reusable structure.
