# Repeated-workflow Skill Crystallization

Date: 2026-08-27
Status: approved

## Goal

Let Jcode turn a repeated successful workflow into a global reusable skill without silently learning, overwriting skills, or trusting unverified conversational claims.

The first slice is user-invoked. Proactive scanning is deferred, but it must later feed the same proposal format and approval path.

## User workflow

1. The user asks Jcode to crystallize a repeated workflow.
2. The agent uses existing `session_search` to find supporting examples.
3. The agent calls `skill_manage` with `action = "crystallize"`, a proposed skill, and session/message evidence references.
4. Jcode verifies the evidence and deduplicates the proposal.
5. Jcode persists a proposal and returns an exact preview plus an approval command. No skill exists yet.
6. After explicit user approval, the agent calls `skill_manage` with `action = "approve_crystallization"`, the proposal ID, and `confirmed = true`.
7. Jcode revalidates the proposal, atomically creates the global skill, reloads the shared registry, and proves the skill is readable.

## Public interface

Extend the existing `skill_manage` tool instead of introducing another public tool.

### Propose

```json
{
  "action": "crystallize",
  "name": "provider-catalog-refresh",
  "description": "Refresh provider model metadata using official sources and route-specific regression checks.",
  "content": "# Provider catalog refresh\n\n...",
  "evidence": [
    {"session_id": "session_a", "message_id": "message_a"},
    {"session_id": "session_b", "message_id": "message_b"}
  ]
}
```

### Approve

```json
{
  "action": "approve_crystallization",
  "proposal_id": "...",
  "confirmed": true
}
```

`approve_crystallization` is the only mutating action in this slice.

## Evidence model

Each evidence reference contains a Jcode `session_id` and `message_id`.

Proposal validation must:

- require at least two references from two distinct sessions
- bound the number of references
- load the persisted session snapshot plus journal
- verify the exact message ID exists in that session
- reject system, background-task, scheduled-task, and tool-only evidence
- require a user or assistant message containing a non-empty text block
- derive a bounded human-readable excerpt from the persisted message rather than accepting an agent-supplied quote
- retain only IDs, role, timestamp, and bounded excerpt in the proposal

This proves that the cited interaction exists. It does not claim that repetition alone proves the proposed workflow is optimal.

## Proposal storage

Store proposals under:

```text
~/.jcode/skill-proposals/<proposal-id>.json
```

A proposal contains:

- schema version
- proposal ID
- normalized skill name
- description
- body content
- normalized content fingerprint
- verified evidence
- creation timestamp

The proposal ID is the SHA-256 digest of the canonical proposal payload, excluding the ID itself. The full lowercase hex digest is the filename. Approval recomputes that digest, so editing a persisted proposal without changing the approved ID fails closed.

Writes use a temporary file followed by an atomic rename.

Proposal files are bounded and reject unknown schema versions.

## Skill installation

Approved skills are written to:

```text
~/.jcode/skills/<name>/SKILL.md
```

The generated file contains only standard skill frontmatter and the approved body. Frontmatter values are serialized safely rather than interpolated as raw YAML. Evidence remains in the proposal record and is not injected into the runtime prompt.

Installation must:

- validate a lowercase kebab-case name
- reject path traversal and reserved dot names
- reject an existing effective global skill with the same name
- reject an existing destination directory or file
- create the skill directory and file without overwrite
- reload the shared global registry
- confirm the reloaded skill has the expected path and content fingerprint

If reload verification fails, return an incomplete result and leave the created file visible for recovery. Do not silently delete user data.

## Deduplication

The first slice performs deterministic deduplication only.

Reject a proposal when:

- its normalized name already exists in the global registry
- another pending proposal has the same normalized name
- another pending proposal or installed Jcode global skill has the same normalized content fingerprint

Normalization standardizes line endings, trims trailing whitespace per line, collapses excessive blank lines, and excludes generated frontmatter. Semantic similarity is deferred to proactive scanning.

Approval repeats all deduplication checks so state changes after proposal creation cannot cause an overwrite.

## Approval and stale proposals

A proposal call never writes a skill.

Approval requires `confirmed = true`. Without confirmation, return the preview and exact confirmation request.

Before installation, approval re-reads the proposal from disk, validates its schema and bounds, verifies its evidence again, recomputes its content-addressed proposal ID, and repeats all deduplication checks. Malformed, stale, duplicate, or modified-under-the-approved-ID proposals fail closed.

After successful installation, move the proposal to:

```text
~/.jcode/skill-proposals/approved/<proposal-id>.json
```

This preserves provenance and prevents replay.

## Code boundaries

- Keep `SkillTool` as the public owner.
- Put proposal, evidence, persistence, deduplication, and installation logic in `tool/skill/crystallization.rs`.
- Reuse `Session::load` for journal-aware evidence verification.
- Reuse `SkillRegistry` for global collision checks and reload verification.
- Do not couple the first slice to the `session_search` formatter or index internals.

The agent orchestrates search and proposal creation. A future scanner can call the same internal proposal function.

## Bounds

- name: 64 bytes
- description: 500 bytes
- body: 64 KiB
- evidence references: 2 to 12
- evidence excerpt: 500 characters each
- proposal file: 128 KiB

Invalid bounds return a typed, non-mutating error message.

## Tests

### Proposal

- schema advertises both new actions and fields
- valid evidence from two persisted sessions creates one proposal
- proposal output includes exact skill preview and verified evidence excerpts
- proposing never creates or reloads a skill
- one session, duplicate sessions, missing session, missing message, system message, and tool-only message fail
- unsafe name and oversized inputs fail
- existing skill name, duplicate pending name, and duplicate normalized content fail

### Approval

- missing confirmation does not mutate
- confirmed approval creates the exact global `SKILL.md`
- shared registry reload makes the skill readable through `skill_manage read`
- destination collision never overwrites
- tampered proposal, unsupported schema, changed evidence, and changed deduplication state fail closed
- raw YAML control characters in name or description cannot alter frontmatter
- successful proposal is archived and cannot be replayed

### Acceptance

Use temporary Jcode homes and real persisted sessions. Exercise `SkillTool::execute` for propose, unconfirmed approval, confirmed approval, read, and duplicate rejection. Then exercise the reloaded daemon's public `skill_manage` interface with a disposable global skill and remove the disposable artifact after validation.

## Deferred

- proactive or scheduled scanning
- semantic clustering
- model-generated proposals without a user request
- auto-approval
- project-local skill generation
- policies, automations, and gates as output types
- TUI inbox or proposal browser
- editing or merging existing skills

## Roadmap simplification

The product roadmap becomes three outcomes:

1. See and steer work.
2. Trust autonomous work.
3. Compound intelligence.

Repeated-workflow Skill Crystallization is the first item under Compound intelligence. Proactive scanning is the next increment only after the user-invoked workflow proves useful and safe.
