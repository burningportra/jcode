---
name: change-check
description: "Vet your own diff before pushing. Use after editing code and before committing or opening a PR: blast radius of changed files, co-change partners you missed, and the budgeted test list. Single pass, then act."
---

# Change-check with codegraph

> Stop rules (read these first, they override the steps below):
> - Run once per diff. Do not re-run after every fixup; re-run only when the file set changes.
> - Skip entirely for docs-only or single-file leaf changes with no dependents.
> - This skill is advisory: it never blocks a commit. A flagged file is a prompt to look, not a veto.

## Steps

**1. What changed** — working-tree file list (git diff/status). This is the input set, not a guess.

**2. Blast radius per file** — `code_impact` with `action=blast_radius` on each changed file.
Note dependents and co-change partners you did not touch.

**3. Budgeted test list** — `code_query` pipeline `search -> filter` for test files reaching the
changed symbols, ending with `{"op": "budget", "n": 10, "max_tokens": 2000}`. Run the listed
tests before calling the change done.

## Output

One short paragraph per changed file: what it affects, what you did not touch but should check,
and which tests to run. Then act on it.
