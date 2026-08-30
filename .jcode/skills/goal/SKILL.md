---
name: goal
description: "Set a goal as a durable initiative and let its plan converge through adversarial review passes with typed underspecification checks before implementing. Use for /goal, 'set a goal', 'plan this', 'refine this plan', 'operate in plan space', or APR-style plan review/hardening prior to writing code."
---

# Goal (plan-space refinement)

You are setting a **goal** (a durable initiative) and converging its plan before
building. Revising a plan is ~10x cheaper than debugging an implementation, so
drive the plan through repeated adversarial review passes with typed checks until
quality converges, then hand off to implementation. Adapted from the Agent
Flywheel / APR (Automated Plan Reviser) methodology.

## Core principle

> "It's a lot easier and faster to operate in 'plan space' before we start
> implementing these things."

Front-load rigor into the plan. Each pass applies a different critical lens.
Early passes fix architecture, middle passes refine structure and edge cases,
later passes polish and subtract. Stop when the plan converges.

## Start

Open a todolist with one entry per phase so the loop is visible in autonomous
runs:

1. Elicit
2. Seed
3. Draft
4. Refine loop
5. Converge
6. Handoff
7. Distill

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
  for input.

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

## Phase 3: Draft

Capture the plan as a durable `initiative` (jcode's plan artifact):

- `initiative create` with `title`, `why`, `description`, `success_criteria`,
  and `milestones` (each with `steps`).
- Make each unit **self-documenting for "future self"**: include background,
  reasoning, and how it serves the over-arching goal. A stranger with no context
  should be able to act on it.

**Optional: multi-model synthesis for high-stakes plans.** The cass-memory case
study (a 5,600-line plan, zero-to-85% in one day) started not from a single
draft but from *competing proposals by different frontier models* synthesized
into one hybrid ("take the best parts of each"). For a large or high-risk goal,
reproduce this: spawn 2-3 `swarm` agents on **different models** with the same
prompt to each propose an approach, then synthesize the strongest ideas into the
initiative and note in the plan which idea came from which proposal (a
comparison table is ideal). For small goals a single draft is fine; do not
over-invest.

**What a strong plan contains** (structural checklist, drawn from the case
study's plan anatomy; use to judge completeness, not every plan needs all):

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

## Phase 4: Refine loop

Run up to 6 passes. Each pass: (a) review with fresh eyes through one lens,
(b) score against the rubric, (c) revise the initiative via `initiative update`,
and (d) record the pass durably with `initiative review` (id, lens, score, and
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
> analysis, then revise the plan to fix what you find.

**Cross-model review (APR's key move).** APR's real leverage is that a
*different, heavyweight* model reviews the plan than the one that will implement
it: a fresh reviewer with no attachment to the draft catches what the author is
blind to. Reproduce that with `swarm` rather than reviewing only in your own
head:

- At least once during the loop (ideally the architecture pass, and again near
  convergence), spawn a `swarm` reviewer on a **different model than this
  session**. Run `swarm list_models` to see routes; for design/review work
  prefer a strong reasoning model (e.g. the `claude-fable-5` review route per the
  swarm routing guidance) distinct from your implementation model.
- Hand the reviewer the current initiative text plus the rubric, and ask for
  concrete gaps, wrong assumptions, and risks, not a rewrite. Prompt shape:
  > You are an adversarial plan reviewer. Here is a plan and a rubric. With fresh
  > eyes and first-principles analysis, list concrete underspecifications, wrong
  > assumptions, missing edge/failure cases, and risks. Be specific; cite the
  > part of the plan. Do not rewrite it; find what is wrong.
- Fold the reviewer's findings back into the initiative via `initiative update`,
  and note in the pass log which findings came from the cross-model reviewer
  (e.g. `pass 1 (architecture) [+swarm review claude-fable-5]: ...`).
- **Verify the reviewer actually ran a different model before claiming it did.**
  When `agents.swarm_model` is unpinned, workers inherit the coordinator's model
  and the per-spawn `model` parameter is silently ignored, so a pass can look
  cross-model while being same-model. Read the worker's actual model from its
  session (`~/.jcode/sessions/<id>.json`, field `model`) and compare it to yours.
  If they match, record the pass as a same-model fresh-eyes review and say the
  plan still lacks a cross-model check. A same-model reviewer still finds real
  defects; it just cannot supply the independence the convergence signal claims.
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

APR measures convergence mechanically (shrinking review output, slowing change
velocity, rising round-to-round similarity). You are approximating the same
signal by hand: when a fresh pass (including the cross-model reviewer) surfaces
only minor, already-known nits rather than new structural gaps, the plan has
converged. A cross-model reviewer that returns "no significant issues" is the
strongest convergence signal available here.

Do not stop early just because one pass found nothing; confirm with the next
lens. Do not keep looping past rubric completion just to chase a round number.

## Phase 5: Converge

**First, rewrite the plan body.** `initiative update` APPENDS to the update log;
it does not revise `description`, `success_criteria`, or `milestones`. If you
only log passes, the reader-facing artifact still describes the draft you spent
the loop disproving, while the review history makes it look rigorous. An
implementer reads top-down and would build the rejected design.

So before reporting convergence, call `initiative update` with rewritten
`description`, `success_criteria`, and `milestones` reflecting the converged
design, then read it back with `initiative show` and check that each correction
actually survived. Reading it back from an independent session is stronger,
because it cannot be fooled by what you remember writing.

Then emit a short convergence report:
- Final score and the quality progression across passes.
- Residual risks and explicit open questions.
- Any rubric items intentionally deferred, with justification.

## Phase 6: Handoff

Turn the converged plan into executable work. The case study's flow is
plan -> structured tasks -> parallel swarm ("planning is 80% of the work; a
detailed plan makes agent execution predictable and fast"):

- Seed a `todo` list from the milestones/steps for a single-session build, or
- Seed a `swarm` task-graph (explore/implement/verify/fix nodes with
  dependencies) for parallel execution. Break large plans into many small,
  dependency-ordered tasks so multiple agents can work without collisions, and
  start from the highest-ROI / unblocking tasks first.

## Phase 7: Distill

Compound the learning: `memory remember` any new reusable rules or anti-patterns
discovered during refinement (category: architecture/testing/security/etc.),
so the next plan starts smarter.

## Notes

- This is plan-space work: do not write implementation code during the loop.
- Prefer revising the existing initiative over creating new ones; the update log
  is the diff trail that shows convergence.
- If the user gave a goal sentence, draft first, then refine. If they gave a full
  plan, load it as the draft and go straight to the refine loop.
