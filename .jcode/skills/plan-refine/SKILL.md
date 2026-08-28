---
name: plan-refine
description: "Drive a plan through adversarial review passes with typed underspecification checks until quality converges, before implementing. Use for /plan-refine, 'refine this plan', 'operate in plan space', APR-style plan review, or hardening a spec/initiative prior to writing code."
---

# Plan Refine (APR-style)

Operate in **plan space**. Revising a plan is ~10x cheaper than debugging an
implementation, so drive the plan through repeated adversarial review passes with
typed checks until quality converges, then hand off to implementation. Adapted
from the Agent Flywheel / APR (Automated Plan Reviser) methodology.

## Core principle

> "It's a lot easier and faster to operate in 'plan space' before we start
> implementing these things."

Front-load rigor into the plan. Each pass applies a different critical lens.
Early passes fix architecture, middle passes refine structure and edge cases,
later passes polish and subtract. Stop when the plan converges.

## Start

Open a todolist with one entry per phase so the loop is visible in autonomous
runs:

1. Seed
2. Draft
3. Refine loop
4. Converge
5. Handoff
6. Distill

## Phase 1: Seed

Load prior knowledge so you do not re-derive known rules or repeat past mistakes.

- `memory recall` relevant rules and anti-patterns for this goal (scope: all).
  If recall returns nothing relevant, note that in one line and move on. Do not
  manufacture rules to fill the section; an empty seed is a valid outcome.
- If a plan/spec already exists (an initiative, a `docs/plans/*.md`, or pasted
  text), load it as the starting draft. Otherwise draft fresh in Phase 2.
- **Ground in the real artifact.** Before reviewing, read the actual code,
  interfaces, and data model the plan touches (not the draft's mental model of
  them). Most high-value gaps surface only when the plan is checked against how
  the system truly works. Re-ground whenever a pass reaches into a new area.

## Phase 2: Draft

Capture the plan as a durable `initiative` (jcode's plan artifact):

- `initiative create` with `title`, `why`, `description`, `success_criteria`,
  and `milestones` (each with `steps`).
- Make each unit **self-documenting for "future self"**: include background,
  reasoning, and how it serves the over-arching goal. A stranger with no context
  should be able to act on it.

## Phase 3: Refine loop

Run up to 6 passes. Each pass: (a) review with fresh eyes through one lens,
(b) score against the rubric, (c) revise the initiative via `initiative update`
(this appends to the initiative's update log, giving you a diff trail).

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

Do not stop early just because one pass found nothing; confirm with the next
lens. Do not keep looping past rubric completion just to chase a round number.

## Phase 4: Converge

Emit a short convergence report:
- Final score and the quality progression across passes.
- Residual risks and explicit open questions.
- Any rubric items intentionally deferred, with justification.

## Phase 5: Handoff

Turn the converged plan into executable work:
- Seed a `todo` list from the milestones/steps for a single-session build, or
- Seed a `swarm` task-graph (explore/implement/verify/fix nodes with
  dependencies) for parallel execution.

## Phase 6: Distill

Compound the learning: `memory remember` any new reusable rules or anti-patterns
discovered during refinement (category: architecture/testing/security/etc.),
so the next plan starts smarter.

## Notes

- This is plan-space work: do not write implementation code during the loop.
- Prefer revising the existing initiative over creating new ones; the update log
  is the diff trail that shows convergence.
- If the user gave a goal sentence, draft first, then refine. If they gave a full
  plan, load it as the draft and go straight to the refine loop.
