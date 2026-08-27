# Skill Crystallization implementation plan

Design: `docs/superpowers/specs/2026-08-27-skill-crystallization-design.md`

## Dependency graph

```text
A. Proposal domain and persistence
  ├─ B. SkillTool public actions
  ├─ C. Evidence and deduplication tests
  └─ D. Approval and installation tests
B + C + D
  └─ E. Public workflow acceptance
E
  ├─ F. Simplify roadmap
  └─ G. Commit, build, reload, live acceptance
```

## A. Proposal domain and persistence

Create `crates/jcode-app-core/src/tool/skill/crystallization.rs`.

Implement:

- bounded input and persisted proposal types
- kebab-case skill-name validation
- normalized body and SHA-256 content fingerprint
- canonical content-addressed proposal ID
- journal-aware session/message evidence verification with bounded single-component IDs and stored-ID equality checks
- full persisted-message evidence digests and a shared session-level eligibility helper
- persisted user approval-message verification bound to the proposal ID and creation time using one exact canonical approval sentence
- safe evidence excerpt rendering without unrelated transcript text
- pending proposal directory and approved archive paths
- bounded JSON reads
- temporary-file plus atomic no-clobber proposal writes with private Unix permissions
- process-wide and cross-process serialization around dedupe, install, registry replacement, and archival
- bounded pending proposal enumeration for deterministic deduplication
- safe YAML frontmatter generation
- no-overwrite skill installation with explicit idempotent recovery states

Keep the module independent of session-search rendering and indexing. It may use `Session::load`, `SkillRegistry`, storage paths, serde, chrono, and sha2.

## B. SkillTool public actions

Extend `SkillInput` with optional proposal fields and evidence references.

Add schema actions:

- `crystallize`
- `approve_crystallization`

Add fields:

- `description`
- `content`
- `evidence`
- `proposal_id`
- `confirmed`
- `approval_evidence`

Route proposal and approval actions into the crystallization module. Preserve all existing action behavior and aliases.

Proposal output must show:

- proposal ID
- exact generated `SKILL.md` preview
- verified evidence references and excerpts
- explicit statement that no skill was created
- exact approval call shape

Approval without `confirmed = true` must show the same preview and confirmation call without mutating.

Confirmed approval must load a fresh global registry off-lock, verify the created skill, atomically replace `SkillTool`'s injected registry, and archive the proposal. Failures after installation return typed recovery states that a retry can resume.

## C. Evidence and deduplication tests

Use temporary Jcode homes guarded by the repository's environment test lock. Save real `Session` snapshots and messages.

Test:

- two distinct valid sessions pass
- repeated session ID fails
- missing session or message fails
- internal system, scheduled, background, and tool-only messages fail
- excerpts are derived and bounded
- the complete persisted message digest is revalidated and detects changes outside the excerpt
- approval must cite a later persisted user message containing the full proposal ID and explicit approval
- unsafe names and oversized fields fail
- existing global name fails
- duplicate pending name fails
- normalized duplicate content fails
- canonical `~/.jcode/skills` duplicates are found even if another source shadows them in the winner-only registry
- proposal creation writes no skill
- concurrent propose calls produce one immutable proposal

## D. Approval and installation tests

Test through `SkillTool::execute`:

- unconfirmed approval does not mutate
- confirmation without valid persisted approval evidence does not mutate
- confirmed approval writes exact global skill
- generated frontmatter cannot be injected
- registry reload makes `read` return the skill
- existing destination is never overwritten
- modified proposal under the approved content-addressed ID fails
- unsupported schema, unsafe evidence IDs, and changed evidence fail
- a new duplicate introduced after proposal fails approval
- successful approval archives the proposal
- replaying approval fails
- concurrent approvals install once
- registry verification and archival failures return typed incomplete states and retry idempotently

## E. Public workflow acceptance

Add one end-to-end test that:

1. persists two sessions with repeated workflow evidence
2. calls public `skill_manage crystallize`
3. extracts the proposal ID from typed metadata or bounded output
4. proves `skill_manage read` cannot find the skill
5. calls unconfirmed approval and proves no mutation
6. persists an explicit user approval message containing the proposal ID and calls confirmed approval with that evidence
7. calls `skill_manage read` and observes the approved content
8. proposes the same workflow again and observes duplicate rejection

Return typed metadata for proposal and approval results so acceptance does not parse prose.

## F. Simplify roadmap

Replace the long product roadmap with a short document containing only:

1. Compound intelligence.
2. Trust autonomous work.
3. See and steer work.

Under Compound intelligence, list user-invoked Skill Crystallization as current, proactive scanning as next, and the context compiler/personal engineering model as later.

Retain links to the detailed Skill Crystallization design rather than repeating architecture.

## G. Verification and delivery

Run:

```text
cargo fmt --all -- --check
cargo test -p jcode-base durable_conversation_evidence --lib
cargo test -p jcode-app-core skill_crystallization --lib
cargo test -p jcode-app-core public_skill_crystallization_workflow --lib
cargo check -p jcode-app-core --no-default-features
```

Then:

- commit implementation and tests
- build and reload the TUI
- exercise the live `skill_manage` proposal, unconfirmed approval, confirmed approval, read, and duplicate-rejection path using disposable global sessions and a disposable skill
- remove only the disposable acceptance artifacts
- verify the user's existing skills are unchanged
- record requirement-to-check evidence
