# Agent Flywheel (ACFS) — Planning Approach

Source: https://agent-flywheel.com/learn (Jeffrey Emanuel / dicklesworthstone), 65 lessons.
Captured 2026-08-27. This distills the planning-relevant lessons for enhancing jcode's planning.

## Core thesis: "Plan Space"

> "It's a lot easier and faster to operate in 'plan space' before we start
> implementing these things!"

Revising plans is ~10x cheaper than debugging implementations. Force verification
at the planning stage, not after code exists. The whole methodology front-loads
rigor into the spec/plan and treats implementation as a comparatively cheap,
parallelizable step.

## The Flywheel Loop (lesson: flywheel-loop)

A compounding loop where each cycle makes the next better:

1. **Identify Task** — scan backlog, triage by priority, pick highest-impact
   (`bv --robot-triage`, `br ready`).
2. **Start agents** — spawn parallel sessions (NTM).
3. **Set context** — load distilled memory (`cm context`).
4. **Prompt** — direct agents (see prompting patterns).
5. **Monitor/guide**.
6. **Scan before commit** — quality gate (`ubs .`).
7. **Update memory** — `cm reflect` distills learnings.
8. **Close task** — `br close`, `br sync`.

Compounding effect: CASS remembers what worked, CM distills reusable patterns,
UBS catches more, coordination improves, sessions get more effective.

## APR — Automated Plan Reviser (lesson: apr)

The signature planning tool. Automates **iterative specification refinement**
using extended reasoning. Instead of manually running 15-20 review rounds, APR
orchestrates them automatically.

- **Convergence behavior**: "resembles numerical optimization settling into a
  minimum." Early rounds fix architectural issues, middle rounds refine
  structure, later rounds polish abstractions.
- **Review passes** (example ladder): Initial Draft → Architecture Review →
  Edge Case Analysis → Performance Optimization → Security Audit → Final Polish.
- **Automated checks** flag underspecification, e.g.:
  - API endpoints defined? Data models present? Auth strategy defined?
  - Input validation? Rate limiting? Error handling beyond "basic"?
- **Diff-tracked**: each pass marks Added / Removed / Modified / Unchanged, with
  a quality % progression per pass.
- **Workflow**: LLM generates initial `plan.md` → `apr refine plan.md -o
  refined-plan.md` → review → feed refined plan back to the coding agent to
  implement.
- Commands: `apr refine plan.md`, `apr refine --output revised.md`.

Key idea for jcode: a plan is not a static todo list. It is a document that
should be driven through **multiple adversarial review passes with typed checks**
until quality converges, BEFORE any implementation.

## Beads — graph-aware task tracking (lessons: beads, bv)

Plans become a **dependency graph of issues** (`br` = beads_rust CLI, `bv` =
viewer/intelligence layer), not a flat list.

- Issue types: bug, feature, task, epic, chore. Priorities 0 (critical) - 4 (backlog).
- Dependencies are explicit: `br dep add <issue> <depends-on>`.
- Graph metrics guide prioritization:
  - **PageRank** — centrality; high = many things depend on it → fix blockers first.
  - **Betweenness** — sits on critical paths; clearing unblocks most work.
  - **Critical path** — longest dependency chain; work it to reduce total time.
  - **Cycles** — circular deps = deadlocks, must be resolved.
- Robot mode (machine-readable, for agents): `bv --robot-triage` (mega-command:
  recommendations, quick wins, blockers to clear, project health),
  `--robot-next` (single top pick + claim command), `--robot-plan` (parallel
  execution tracks + unblocks lists), `--robot-insights` (full metrics).
- Workflow: create → add deps → `br ready` → claim (`--status=in_progress`) →
  close → `br sync`. `.beads/` is committed as source of truth.
- Beads should be **self-contained and self-documenting**: detailed comments with
  background, reasoning, considerations — everything "future self" needs.

## Prompting patterns for planning (lesson: prompt-engineering)

The direction style that produces good plans:

1. **Intensity calibration** — stacked modifiers ("super careful, methodical, and
   critical") are calibration signals to allocate more reasoning depth. Claude
   Code `/effort max` for hard tasks.
2. **Scope control** — push against premature narrowing: "take ALL of that",
   "cast a wider net", "go super deep", "first-principle analysis".
3. **Forcing self-verification** — questions trigger metacognition: "Are you
   sure it makes sense? Is it optimal? Could we change anything to make it work
   better for users? If so, revise." Applied per-bead: "Check over each bead
   super carefully."
4. **Fresh eyes** — "with fresh eyes", "randomly explore the code", peer framing
   ("reviewing code written by your fellow agents") to break confirmation bias.
5. **Temporal awareness** — write self-documenting, self-contained artifacts for
   "future self"; connect current work to over-arching goals.
6. **Context anchoring** — re-read AGENTS.md especially after compaction to
   prevent drift.
7. **First principles** — diagnose root causes, understand before fixing, see
   larger context.

The **Plan Review Pattern** (canonical):
> "Check over each bead super carefully—are you sure it makes sense? Is it
> optimal? Could we change anything to make the system work better? If so, revise
> the beads. It's a lot easier and faster to operate in 'plan space' before we
> start implementing these things!"

## Memory / compounding (lessons: cm, context-mastery)

- **CM (CASS Memory)**: distills sessions into single-line rules with categories
  (debugging, security, performance, architecture, testing, tooling) and
  anti-patterns. Protocol: START `cm context "<task>"` → WORK (reference rule
  IDs) → FEEDBACK (inline `[cass: helpful/harmful <id>]`) → END (learning is
  automatic). Plans should be seeded with retrieved rules + anti-patterns.
- **Context efficiency**: TRU (token compression 40-70%), S2P (bundle sources),
  CASS (retrieve specific past solutions not whole files), CM (distilled rules).
  Fit distilled knowledge + targeted solutions + compressed source into context.

## Project bootstrap / AGENTS.md (lessons: project-bootstrap, agents-md)

- Bootstrap layers: issue tracking (beads) → safety hooks (DCG/SLB) → quality
  gates (UBS) → AGENTS.md → agent coordination (Agent Mail).
- AGENTS.md is the "API contract" every agent reads at session start. Keep under
  ~200 lines (context budget). Be specific, not vague. Include build/test
  commands, conventions, safety rules, coordination protocol. "Rule 0": user
  override prerogative.

## Swarm coordination (lesson: swarm-coordination)

Pipeline: Pick (`bv --robot-next`) → Claim (Agent Mail + bead in_progress) →
Reserve (lock exact files) → Execute (narrow slice) → Verify (gates + UBS) →
Land (close/sync beads, release reservations, post handoff). Memory is a
preflight hint; live repo files + beads state are source of truth.

---

## Implications for jcode planning (synthesis)

The flywheel's planning DNA that jcode currently lacks:

1. **Plan-space primacy** — a distinct, reviewable plan artifact (a `plan.md`),
   not just an ephemeral todo list.
2. **Iterative adversarial refinement (APR)** — drive the plan through N typed
   review passes (architecture, edge cases, perf, security, polish) until quality
   converges, with diff tracking and a per-pass quality signal. This is the
   single most distinctive piece.
3. **Typed underspecification checks** — automated gates that flag vague plans
   (no endpoints, no schema, no error strategy, "handle tokens" etc.).
4. **Graph-structured work** — plans decompose into a dependency DAG with
   priorities and graph-metric-based prioritization (jcode's swarm task-graph is
   the closest existing primitive).
5. **Self-documenting units** — each task carries background, reasoning,
   justification for "future self".
6. **Memory-seeded planning** — load distilled rules + anti-patterns before
   planning; distill new ones after.
7. **The Plan Review prompt** — bake the "are you sure? is it optimal? revise in
   plan space" metacognition into the planning flow.
