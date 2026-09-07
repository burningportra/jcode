---
name: orient
description: "Map code before reading it. Use the moment you land in unfamiliar code (new ticket, asked 'how does X work / where is Y'), need an architecture overview, or are resuming cold after compaction. Runs code_query/code_impact (deterministic call-graph map) instead of blind grep + whole-file reads. Stop at the first rung that answers."
---

# Orient with codegraph

> Stop rules (read these first, they override the ladder below):
> - Stop at the first rung that answers the question. The one-screen map usually does.
> - Skip this skill entirely for one-line fixes in files you already know.
> - Never run more than 3 rungs without acting on what you found.

`code_query` and `code_impact` are built-in tools. No install, no daemon. Start from the repo root
or the subsystem you are working in.

## The escalation ladder — climb only until oriented

**1. Task-relevant code** — `code_query` pipeline `search -> filter -> outline`:
narrow by vocabulary, then read signatures (not bodies). This answers "where is the code for X".

**2. Blast radius** — `code_impact` with `action=blast_radius` on the file you plan to touch:
dependents, co-change partners, affected count. Read this before editing a widely-imported file.

**3. Budget it** — append `{"op": "budget", "n": 10, "max_tokens": 2000, "signatures_only": true}`
as the last pipeline stage when the map is large. Cuts at block boundaries, discloses `capped=`,
self-reports `est_tokens`. Prefer signatures over bodies until you pick one symbol to expand.

## Then read, and trust the honesty signals

Read the specific files the map surfaces. A `partial: true` result means the index was degraded —
treat counts as floors. A `capped=` line means rows were cut; narrow the query instead of assuming
completeness.
