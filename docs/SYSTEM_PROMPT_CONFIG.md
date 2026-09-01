# Configuring the System Prompt

jcode builds its system prompt from several layers. Two of them are user-editable
files, so you can tune agent behavior without rebuilding.

## Layers (in order)

1. **Base system prompt** — built-in `crates/jcode-base/src/prompt/system_prompt.md`,
   overridable by file (see below).
2. Capability modules (e.g. Mermaid guidance).
3. Self-dev guidance (self-dev sessions only).
4. `AGENTS.md` — project `./AGENTS.md` and global `~/AGENTS.md`.
5. Prompt overlay — `./.jcode/prompt-overlay.md` and `~/.jcode/prompt-overlay.md`.
6. Preferred tools — `./.jcode/preferred-tools.md` and `~/.jcode/preferred-tools.md`.
7. Memory and the active skill prompt (dynamic, not cached).

## Adding guidance (most common)

Append instructions without touching the default prompt:

- `~/.jcode/prompt-overlay.md` — applies everywhere.
- `./.jcode/prompt-overlay.md` — applies to one project.

Both are included when present.

## Replacing the base prompt

To fully replace layer 1, create either file:

- `./.jcode/system-prompt.md` (project, highest precedence)
- `~/.jcode/system-prompt.md` (global)

The first non-empty file wins; otherwise the built-in default is used. An empty or
whitespace-only file falls back to the default, so you cannot accidentally ship an
empty prompt.

This replaces only the base prompt. AGENTS.md, overlays, skills, and memory still apply.

## Post-compaction AGENTS.md recall

After context compaction, the summarized transcript can de-emphasize project rules that
were fresh earlier in the session. To counter that drift, the first turn after a
compaction completes prepends a one-shot **Context Compaction Recall** reminder telling
the agent to re-read `AGENTS.md` (and any active skill instructions).

Behavior details:

- Fires **once** per compaction, on the next turn that runs a model request.
- Only injected when project instructions actually exist (`./AGENTS.md` or
  `~/AGENTS.md` are loaded); with no AGENTS.md the flag is dropped silently so it
  cannot fire later.
- The recall is prepended before any other system reminder for that turn. It is
  transient: it lives on the `current_turn_system_reminder`, is never persisted to
  session history, and the flag clears whether or not AGENTS.md exists.

## Notes

- Changes to these files take effect for **new sessions**; a running session keeps the
  prompt captured at start.
- Editing the built-in `system_prompt.md` requires a rebuild (`selfdev build-reload`),
  since it is embedded with `include_str!`.
- Swarm model-routing guidance has its own analogous file: `.jcode/swarm-prompt.md`.
  Use `/swarm-prompt` to edit the active project or global file. New agents load
  the latest contents immediately; already-running agents keep the prompt they
  captured at session creation so their tool definition and context cache stay stable.
