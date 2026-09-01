---
name: goal
description: "Set a goal as a durable initiative, produce an exhaustive markdown plan, converge it through adversarial multi-model review passes, then convert it into polished beads before implementing. Use for /goal, 'set a goal', 'plan this', 'refine this plan', 'operate in plan space', 'convert plan to beads', or Flywheel/APR-style plan review and hardening prior to writing code."
---

# Goal (Flywheel plan-space refinement)

You are setting a **goal** (a durable initiative) and converging its plan before
building. Planning is ~80% of the work. Revising a plan is ~10x cheaper than
debugging an implementation (the Law of Rework Escalation: a mistake costs 1x in
plan space, 5x in bead space, 25x in code space), so front-load rigor. Drive the
plan through repeated adversarial review passes with typed checks until quality
converges, translate it into beads, polish the beads, and only then hand off to
implementation. Adapted from the Agent Flywheel methodology
(https://agent-flywheel.com/complete-guide) and APR (Automated Plan Reviser).

## Core principle

> "It's a lot easier and faster to operate in 'plan space' before we start
> implementing these things."

Three reasoning spaces, three artifacts, three questions:

| Space | Artifact | You decide there |
|---|---|---|
| Plan space | Large markdown plan (`docs/plans/`) | What the system *is*: architecture, workflows, tradeoffs |
| Bead space | `br` issues + dependency graph | How the work is *packaged*: task boundaries, order, embedded context |
| Code space | Source files + tests | Implementation and verification |

The whole point of the markdown plan is that it stays small enough to fit a
model's context window while the codebase never does. Global reasoning while
global reasoning is still possible. If you are still redesigning the product,
stay in plan space. If you are mainly packaging work for execution, move to bead
space. Once you are in bead space you never look back at the plan, so the beads
must carry all the details forward. The exception: if implementation reveals a
plan-level flaw, return to plan space, fix the plan, and re-translate the
affected beads. This loop back is deliberate, not a failure.

Every phase transition (plan -> bead, bead -> code) is a validation gate: do
not advance while the previous artifact still has unresolved structural gaps.
Drop back a phase instead of pushing forward optimistically.

## Start

Open a todolist with one entry per phase so the loop is visible in autonomous
runs:

1. Elicit
2. Seed
3. Draft (markdown plan)
4. Refine loop (plan space)
5. Converge (rewrite plan body)
6. Bead space (convert + polish)
7. Handoff
8. Distill

Scale the pipeline to the goal. Full ceremony is for new systems or major
features; for bounded single-session work, compress to Elicit -> short plan ->
todo list. Never skip Elicit, Converge, or Distill. See Notes for the exit
ramp.

## Phase 1: Elicit

**A plan can only be as good as your understanding of what the user wants.** The
most expensive failure in this whole loop is converging a beautiful plan for the
wrong goal, and no number of adversarial passes fixes a misread intent. So before
drafting, interview the user grill-me style: ask the questions whose answers would
most change the plan, not a generic questionnaire.

Judge first whether you need this, and how much:

- **Underspecified goal + user is present** (the normal `/goal` case): run a short
  grilling before drafting. Draw the technique from the bundled `grill-me` skill
  (a relentless interview to sharpen a plan): surface the assumptions, the
  ambiguities, and the decisions that fork the design, and make the user commit to
  answers. Prefer a small batch of high-leverage questions over a long list.
- **Already well-specified** (a detailed spec, a pasted plan, or a precise
  request): skip or shrink this. Confirm the one or two riskiest assumptions in a
  single question rather than interrogating what is already clear.
- **Autonomous / unattended run** (no human to answer): do NOT block on the human.
  Write down the open questions and the assumption you are proceeding under for
  each, put them in the initiative's `why`/description, and mark them as
  assumptions the refine loop must stress-test. Never stall a headless run waiting
  for input. Escalation path for headless runs: if the ambiguity is
  goal-forking (you cannot even pick a direction), record the fork explicitly in
  the initiative, pick the interpretation with the best value/risk ratio, proceed,
  and surface the fork prominently in every convergence report. If NO
  interpretation is defensible, stop and report the blocker instead of
  fabricating a goal.

Aim your questions at what actually forks the plan:

- **Outcome and definition of done.** What does success look like concretely, and
  how will the user know it worked? This becomes `success_criteria`.
- **Scope boundaries.** What is explicitly out of scope? What is the smallest
  version that would still be worth shipping?
- **Constraints.** Deadlines, compatibility, performance, security, things that
  must not change or break.
- **Users and workflow.** Who uses this and how; what is the real acceptance path.
- **Tradeoffs.** When two goals conflict (speed vs. safety, scope vs. time), which
  wins? Make the user rank, do not guess.
- **Prior context.** Has this been attempted before, and what went wrong?

Batch the questions, get answers, then reflect them back in one or two lines the
user can correct ("So the goal is X, done when Y, explicitly not Z") before you
draft. If the interview reveals the real goal is different from the opening
request, that is the interview working. Update your understanding via the `todo`
tool's `user_intention` and proceed from the corrected goal.

## Phase 2: Seed

Load prior knowledge so you do not re-derive known rules or repeat past mistakes.

- `memory recall` relevant rules and anti-patterns for this goal (scope: all).
  If recall returns nothing relevant, note that in one line and move on. Do not
  manufacture rules to fill the section; an empty seed is a valid outcome.
- If a plan/spec already exists (an initiative, a `docs/plans/*.md`, or pasted
  text), load it as the starting draft. Otherwise draft fresh in Phase 3.
- **Ground in the real artifact.** Before reviewing, read the actual code,
  interfaces, and data model the plan touches (not the draft's mental model of
  them). Most high-value gaps surface only when the plan is checked against how
  the system truly works. Re-ground whenever a pass reaches into a new area.
- **Foundation bundle check.** Confirm AGENTS.md, best-practice references, and
  the tech-stack decision exist and are coherent before planning. Weak foundations
  leak uncertainty into every later stage.

## Phase 3: Draft (the markdown plan)

The primary artifact is an **exhaustive markdown plan file** at
`docs/plans/PLAN_<SLUG>.md`, committed to the repo. It, not the initiative, is
what the refine loop and the reviewers work against. The initiative
(`initiative create` with `title`, `why`, `description`, `success_criteria`,
`milestones`) is the durable index that points at it and tracks convergence.

- Make each unit **self-documenting for "future self"**: include background,
  reasoning, and how it serves the over-arching goal. A stranger with no context
  should be able to act on it.
- Plans in this methodology are large (3,000-6,000+ lines for major projects) and
  that is fine: planning tokens are far cheaper than implementation tokens. Depth
  is the point, not bloat. Every line must carry information, not padding.

**Multi-model synthesis (the standard opening move).** Do not draft alone.
Reproduce the flywheel's competing-plans pattern:

1. Spawn 2-3 `swarm` agents on **different models** (`swarm list_models` first)
   with the same elicitation-derived prompt; each independently proposes an
   approach. Competing proposals surface complementary strengths and blind spots.
2. Synthesize a "best of all worlds" hybrid: analyze all proposals with an open
   mind about what the others did better, then fold every good idea into one
   superior plan (note which idea came from which proposal in a comparison table).
3. For small or low-risk goals a single draft is acceptable; do not over-invest.
   Scale the ceremony to the stakes.

**Degradation ladder for multi-model drafting.** If `swarm` is unavailable,
spawned workers fail, or all workers come back same-model, fall back in order:
(a) one non-coordinator model via any available route, (b) a second independent
session on your own model, (c) a single self-draft. Record which rung you
landed on in the plan's comparison table; never claim competing-proposal
synthesis that did not happen.

The markdown plan is the primary artifact only for goals touching code or
product behavior in a repo. For non-code goals (a process, a campaign, an
analysis), write the same exhaustive plan at `docs/plans/PLAN_<SLUG>.md` and
adapt the artifact-specific checklist items to the domain.

**What a strong plan contains** (structural checklist; use to judge
completeness, not every plan needs all):

- Executive summary: problem + solution in one screen.
- Data models / core types: concrete schemas, not prose ("validate inputs" ->
  the actual type and constraints).
- Interface/CLI/API surface: every command or entry point with example I/O.
- Architecture sketch: how data flows between the pieces (an ASCII diagram is
  fine).
- Error handling and edge cases anticipated *before* implementation.
- Implementation roadmap: phased, dependency-ordered, with rough effort/impact
  so the highest-ROI work goes first.
- Comparison table: why this approach over the alternatives considered.
- Theory-first, concrete examples throughout: explain the why, then show a real
  example, then implementation notes.

## Phase 4: Refine loop (plan space)

Run up to 6 passes over the **markdown plan** (fresh eyes each pass). Each pass:
(a) review with fresh eyes through one lens, (b) score against the rubric,
(c) revise the plan file in place and log the change via `initiative update`, and
(d) record the pass durably with `initiative review` (id, lens, score, and
optional gaps/resolved/reviewer_model). `initiative update` keeps the free-text
diff trail; `initiative review` stores the structured convergence history
(quality progression) that renders in the goal's "Plan review" section. When a
cross-model reviewer produced the pass, pass its model as `reviewer_model`.

A pass need not force a revision. If a lens genuinely does not apply (e.g.
performance for a tiny internal helper), record it as `pass N (<lens>): score X
— not applicable: <one-line reason>` and move on. Do not pad the log with
invented changes. Each pass reviews the *previous* pass's output with fresh eyes,
so contradictions introduced earlier get caught later; that is the point.

**Pass ladder** (one lens per pass):

1. Architecture and interfaces
2. Edge cases and failure/error handling
3. Data model and state transitions
4. Performance and resource constraints
5. Security, safety, and destructive-op guards
6. Simplicity and subtraction, then final polish

**Fresh-eyes review prompt** to run each pass (paraphrase, do not just recite):

> Read the current plan with fresh eyes, as if reviewing a peer's work. Through
> the lens of <this pass>, super carefully and critically: are you sure each
> part makes sense? Is it optimal? What is underspecified, ambiguous, or missing?
> What would break in production? Diagnose root causes with first-principle
> analysis, then revise the plan to fix what you find. For each change, give the
> rationale plus a git-diff-style change against the plan.

**Anti-stall techniques (from the flywheel's session archive):**

- **"Lie to them" overshoot hunt.** Models stop hunting after finding ~20-25
  issues and declare satisfaction. If a review pass feels too short or
  self-satisfied, re-run it with: "Check again against the plan. I am positive
  you missed at least 80 issues from that feedback." The model keeps cranking
  instead of settling early.
- **Fresh conversations/sessions.** Run later passes in fresh `swarm` workers
  (or fresh sessions) so no reviewer anchors on its own prior output.
- **No oversimplification.** Each pass must obey: DO NOT oversimplify; DO NOT
  lose features or functionality while revising. Models tend to "improve"
  artifacts by deleting complexity they do not fully understand. Subtraction is
  allowed only when the removed thing is demonstrably redundant, and must be
  justified in the pass log.

**Cross-model review (the flywheel's key move).** The real leverage is that a
*different, heavyweight* model reviews the plan than the one that will implement
it: a fresh reviewer with no attachment to the draft catches what the author is
blind to. Reproduce that with `swarm` rather than reviewing only in your own
head:

- At least once during the loop (ideally the architecture pass, and again near
  convergence), spawn a `swarm` reviewer on a **different model than this
  session**. Run `swarm list_models` to see routes; for design/review work
  prefer a strong reasoning model (e.g. the `claude-fable-5` review route per the
  swarm routing guidance) distinct from your implementation model.
- Hand the reviewer the current plan file plus the rubric, and ask for concrete
  gaps, wrong assumptions, and risks, not a rewrite. Prompt shape:
  > You are an adversarial plan reviewer. Here is a plan and a rubric. With fresh
  > eyes and first-principles analysis, list concrete underspecifications, wrong
  > assumptions, missing edge/failure cases, and risks. Be specific; cite the
  > part of the plan. Do not rewrite it; find what is wrong.
- Fold the reviewer's findings back into the plan via `initiative update`, and
  note in the pass log which findings came from the cross-model reviewer
  (e.g. `pass 1 (architecture) [+swarm review claude-fable-5]: ...`).
- **Verify the reviewer actually ran a different model before claiming it did.**
  When `agents.swarm_model` is unpinned, workers inherit the coordinator's model
  and the per-spawn `model` parameter is silently ignored, so a pass can look
  cross-model while being same-model. Read the worker's actual model from its
  session (`~/.jcode/sessions/<id>.json`, field `model`) and compare it to yours.
  If that file is missing, unreadable, or lacks the `model` field, treat the
  pass as UNVERIFIED (same-model) rather than assuming; say so in the pass log.
  If they match, record the pass as a same-model fresh-eyes review and say the
  plan still lacks a cross-model check. A same-model reviewer still finds real
  defects; it just cannot supply the independence the convergence signal claims.
- **Convergence honesty guard.** Because self-scored convergence is
  inflation-prone, the plan is not "fully converged" unless either (a) at least
  one verified cross-model pass ran during the loop, or (b) the convergence
  report explicitly states "no cross-model review was possible" and lists that
  as a residual risk. Never let an unfalsifiable score be the only evidence of
  convergence.
- **Overshoot-hunt bound.** Run the overshoot hunt at most twice per artifact;
  findings from a third pass are increasingly confabulated. Require each
  finding to cite the specific plan text it corrects; discard findings that
  cannot.
- If `swarm` is unavailable, degrade gracefully: do the fresh-eyes pass yourself
  and note that no independent reviewer model was used. A single-model loop is
  still useful; it just lacks the cross-model check.

**Typed underspecification rubric** (check every pass; a plan is not done while
any relevant box is unchecked):

- [ ] Scope and explicit non-goals stated
- [ ] Interfaces/contracts defined (signatures, data shapes, CLI/API surface)
- [ ] Data model and state transitions specified
- [ ] Error and failure handling beyond "basic"
- [ ] Edge cases enumerated
- [ ] Performance / resource constraints where they matter
- [ ] Security / safety / destructive-op guards addressed
- [ ] Verification strategy present (how we will know it works)
- [ ] Dependencies and ordering captured (what blocks what)
- [ ] Each unit self-documenting for "future self"
- [ ] Simplicity: anything to subtract before adding?

**Scoring and convergence.** After each pass, score the plan 0-100 (roughly:
% of relevant rubric items satisfied, weighted by risk). The score is a judgment
signal, not a computed metric; use it to see direction of travel, not as the
authority on when to stop. Record the pass in the initiative update as
`pass N (<lens>): score X/100 — <gaps found> / <resolved>`.

Stop the loop when the PRIMARY condition holds:
- **Primary (rubric completion):** every rubric item relevant to this plan is
  satisfied or explicitly, justifiably deferred. This is the real convergence
  test.

Secondary conditions (use as backstops, not the main gate):
- all 6 passes done, or
- score delta < 3 across two consecutive passes with no new gaps found.

The flywheel measures convergence mechanically; approximate the same signals by
hand: review output shrinking round to round, change velocity decelerating,
successive rounds growing more similar (rising content similarity). When a fresh
pass (including the cross-model reviewer) surfaces only minor, already-known
nits rather than new structural gaps, the plan has converged. A cross-model
reviewer that returns "no significant issues" is the strongest convergence
signal available here.

Convergence red flags (rethink rather than iterate):
- **Oscillation** (alternating between two versions): reframe the problem; if
  the oscillation survives a reframe, surface both variants to the user (or, if
  headless, pick one, record why, and flag it as an open decision).
- **Expansion** (output growing instead of shrinking): an agent is adding
  complexity; step back.
- **Plateau at low quality**: kill the approach and restart fresh. When you
  restart, reset the pass counter; carried-over pass numbers create false
  convergence history.
- **Tooling failure mid-loop** (initiative or swarm unavailable): the plan file
  and its git history are the source of truth. Record pass results as commits
  with the `pass N (<lens>): score X/100` line in the commit message, and
  backfill `initiative review` entries when the tool recovers. Never lose pass
  history to a tool outage.

Do not stop early just because one pass found nothing; confirm with the next
lens. Do not keep looping past rubric completion just to chase a round number.

**When to leave plan space.** Stay in plan refinement if whole-workflow
questions are still moving, major architecture debates are open, or fresh models
keep finding substantial missing features, constraints, or tradeoffs. Move to
bead space when remaining improvements are about execution structure, testing
obligations, sequencing, and embedded context rather than what the system
fundamentally is.

## Phase 5: Converge (rewrite the plan body)

`initiative update` APPENDS to the update log; it does not revise `description`,
`success_criteria`, or `milestones`. If you only log passes, the reader-facing
artifact still describes the draft you spent the loop disproving, while the
review history makes it look rigorous. An implementer reads top-down and would
build the rejected design.

So before reporting convergence, call `initiative update` with rewritten
`description`, `success_criteria`, and `milestones` reflecting the converged
design, rewrite the markdown plan body in place to the converged version, then
read the initiative back with `initiative show` and check that each correction
actually survived. Reading it back from an independent session is stronger,
because it cannot be fooled by what you remember writing. On the compressed
single-session path (no initiative), the convergence report goes in the final
summary plus a commit; do not invent an initiative just to satisfy this phase.
If `initiative update` fails or the tool is unavailable, record the convergence
report as the body of a git commit on the plan file instead; the durable trail
matters more than the tool.

Then emit a short convergence report:
- Final score and the quality progression across passes.
- Residual risks and explicit open questions.
- Any rubric items intentionally deferred, with justification.

## Phase 6: Bead space (convert + polish)

Only run this phase when the work will outlive this session, involves multiple
actors, or needs dependency-aware ordering. A small single-session goal with a
converged plan and a rich todo list is already executable; forcing it through
bead creation is ceremony without leverage.

Treat plan-to-beads as a **translation problem, not task extraction**. Watch for
the **plan-bead gap**: plans that got refined but never became beads. Always end
convergence with an explicit transition into this phase.

**Convert:**
- If the repo uses beads (`br` available), create real beads via `br create` /
  `br dep add`, with the full dependency graph. Never write pseudo-beads in
  markdown.
- Beads must be **self-contained executable memory**: so detailed that no agent
  ever needs to reopen the plan. Each bead carries outcome, background/reasoning,
  failure modes, embedded markdown context, and explicit **test obligations**
  (unit + e2e expectations). Nothing from the plan may be lost in translation.
- If `br` is unavailable, seed a `todo` list or `swarm` task-graph with the same
  richness instead: every node self-contained, dependency-ordered, with
  verification steps. Richness matters more than the tool.
- **Compaction safety:** beads and todos must be writable without conversation
  memory. If a session compaction hits mid-conversion, the plan file plus the
  already-created beads are sufficient to resume; re-read the plan and `br
  list --json` before continuing rather than reconstructing from memory.
- **Concurrent edits:** if other agents may touch the same plan file, commit
  after each pass and rebase/pull before each new pass; the pass log in the
  commit messages is append-only.

**Polish ("check your beads N times, implement once"):** run 3-5+ polishing
passes until convergence, ideally mixing models (a fresh reviewer per pass):

> Check over each bead super carefully: are you sure it makes sense? Is it
> optimal? Could anything be changed to make the system work better? Revise.
> It is a lot easier and faster to operate in plan/bead space before
> implementing. DO NOT OVERSIMPLIFY. DO NOT LOSE ANY FEATURES OR FUNCTIONALITY.

Each pass should include:
- Duplicate detection and merging (keep the survivor with richer test specs and
  better dependency chains).
- Filling empty or thin descriptions (a thin bead makes the swarm improvisational).
- Correcting dependency links (this is what makes execution order deterministic).
- **Bidirectional coverage check**: cross-reference every plan element against
  beads AND every bead against the plan, both directions, so nothing was lost.

**Handoff:** once beads converge, execution becomes mechanical: seed a `todo`
list for a single-session build, or a `swarm` task-graph (explore/implement/
verify/fix nodes with dependencies) for parallel execution, starting from the
highest-ROI / unblocking tasks first. If heavy cognitive work shows up during
implementation, that is the signal planning or beads were insufficient: pause,
go back to bead space, add the missing detail.

## Phase 7: Distill

Compound the learning: `memory remember` any new reusable rules or anti-patterns
discovered during refinement (category: architecture/testing/security/etc.),
so the next plan starts smarter.

## Operator library (canonical flywheel moves)

The methodology names its recurring cognitive moves as operators. This skill
implements all of them; know the mapping so the vocabulary matches the source:

| Operator | Where implemented here |
|---|---|
| 1. Plan-first expansion | Phase 3 (draft before code; plan must cover testing, failure paths, sequencing) |
| 2. Competing-plan triangulation | Phase 3 (multi-model synthesis; integrate only the strongest elements, not every idea) |
| 3. Overshoot mismatch hunt | Phase 4 (lie-to-them re-pass when review output looks too short) |
| 4. Plan-to-beads transfer audit | Phase 6 (bidirectional coverage check; rationale/constraints/tests embedded in beads) |
| 5. Convergence polish loop | Phases 4 and 6 (stop when revisions are small, corrective, coverage checks pass) |
| 6. Fresh-eyes reset | Phase 4 (fresh sessions when passes get repetitive or shallow) |
| 7. Fungible swarm launch | Handoff (staggered, coordination in artifacts, no special-agent bottleneck) |
| 8. Feedback-to-infrastructure closure | Phase 7 (distill lessons into durable rules, skills, and AGENTS.md guidance) |

## Notes

- This is plan-space/bead-space work: do not write implementation code until
  Phase 6 completes.
- Prefer revising the existing initiative over creating new ones; the update log
  is the diff trail that shows convergence.
- If the user gave a goal sentence, draft first, then refine. If they gave a full
  plan, load it as the draft and go straight to the refine loop.
- Not every change deserves the full pipeline: for quick bounded work, a plain
  todo list is fine. If an ad-hoc change later grows important, retroactively
  formalize it.
