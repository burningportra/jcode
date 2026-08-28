# jcode Plan Refinement (APR-style) — Design

Goal: bring dicklesworthstone's "plan space" methodology into jcode planning.
The heart is APR: drive a plan through N adversarial review passes with typed
underspecification checks until quality converges, BEFORE implementing.

## What jcode already has (reuse, don't rebuild)

- `initiative` tool + `Goal` model (`jcode-task-types`): durable, project-scoped
  plan with `title`, `why`, `description`, `success_criteria[]`, `milestones[]`
  (each with `steps[]`), `next_steps[]`, `blockers[]`, `progress_percent`,
  `updates[]` log. This IS jcode's `plan.md` equivalent, already persisted.
- `todo` tool: ephemeral per-session execution list + plan intent object.
- `swarm` task-graph: dependency DAG (explore/implement/verify/fix) — the
  beads/`bv --robot-plan` analogue for parallel execution.
- `memory` tool: the CM analogue (rules + anti-patterns, project/global scope).
- `skill` system: load/compose specialist prompts.

## Mapping APR -> jcode

| APR concept | jcode landing |
|---|---|
| `plan.md` artifact | a `Goal` (initiative), persisted already |
| refine passes | a review loop the agent runs over the Goal |
| typed checks (endpoints? schema? errors?) | a checklist rubric applied each pass |
| quality % convergence | a score per pass; stop when delta < threshold |
| diff per pass (add/mod/remove) | `Goal.updates[]` entries + milestone diffs |
| feed refined plan to implement | existing swarm task-graph / todo execution |
| memory seeding | `memory recall` before, `remember` after |

## Two-phase plan (recommended)

### Phase 1 — `/plan-refine` skill (no Rust changes, fully reversible)

A native jcode skill that runs the APR loop by orchestrating existing tools.
Fast to ship, proves the loop end-to-end, easy to iterate on the rubric.

Flow:
1. **Seed**: `memory recall` relevant rules + anti-patterns for the goal.
2. **Draft**: create/load an `initiative` capturing title, why,
   success_criteria, milestones/steps.
3. **Refine loop** (default up to 6 passes, stop on convergence):
   - Pass ladder: Architecture -> Edge cases -> Failure/error handling ->
     Performance -> Security/safety -> Simplicity/subtraction -> Final polish.
   - Each pass runs the **fresh-eyes review prompt** over the current plan,
     applies the **typed check rubric**, records found gaps, and revises the
     initiative via `initiative update` (which appends to `updates[]`).
   - Score the plan 0-100 against the rubric. Stop when `score >= 90` or
     `delta < 3` two passes running, or max passes hit.
4. **Converge**: emit a final plan summary + residual risks + open questions.
5. **Handoff**: optionally seed a swarm task-graph / todo list from milestones.
6. **Distill**: `memory remember` any new reusable rules discovered.

Typed underspecification rubric (adapted from APR, generalized beyond web APIs):
- Scope & non-goals explicit?
- Interfaces/contracts defined (signatures, data shapes, CLI/API surface)?
- Data model / state transitions specified?
- Error + failure handling beyond "basic"?
- Edge cases enumerated?
- Performance / resource constraints stated where they matter?
- Security / safety / destructive-op guards addressed?
- Test / verification strategy present (how we'll know it works)?
- Dependencies & ordering captured (what blocks what)?
- Each unit self-documenting for "future self"?
- Simplicity: anything to subtract before adding?

### Phase 2 — promote the highest-value pieces into Rust `initiative`

Once the loop is proven, harden the parts that benefit from being first-class:
- Add a `refine` action (or `review_pass`) to the `initiative` tool that stores
  structured pass records: `{ pass, lens, score, gaps[], resolved[] }` on the
  Goal, so convergence history is durable and renderable in the side panel.
- Add a `PlanReview` struct to `Goal` (`reviews: Vec<PlanReview>`), plus a
  computed `quality_score`. Render the quality progression in the goals overview.
- Optional: a lightweight dependency-metric helper (critical path / blocker
  centrality) over milestones+steps to mirror `bv --robot-plan`.

## Why skill-first

- Zero binary risk; reversible; iterate the rubric in plain markdown.
- The rubric + prompts are the actual product here; getting them right in a
  skill first avoids baking a weak rubric into Rust.
- Promotion path is clean: the skill's structured pass records become the Rust
  `PlanReview` schema.

## Non-goals (for now)

- No reimplementation of beads/`br` as a separate tracker; the swarm task-graph
  + initiative cover decomposition and execution.
- No separate `apr` binary; the loop lives inside the agent.
