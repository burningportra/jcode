# File Reservations (Swarm Coordination)

Reservations are advisory file-ownership markers that jcode agents can hold on
project-relative paths or globs before editing. They implement the flywheel's
"Agent Mail"-style coordination so parallel agents avoid editing the same files.
Reservations are **advisory**, not rigid locks: they expire (TTL), can be
released, and never block a crashed agent forever.

## Why

When two agents work in the same repository, the most common failure is two of
them editing overlapping files. Reservations let an agent claim a surface in
advance so others can route around it. Because they are advisory with expiry,
a dead or compacted agent's hold degrades gracefully instead of deadlocking the
swarm.

## Tool surface

File reservations are exposed through the `swarm` tool (`communicate`) via three
actions:

- `reserve` — claim one or more project-relative paths/globs.
- `release` — free reservations held by the current session.
- `reservations` — list active reservations for the project.

Reservations are keyed by **project root** (the session's working directory), so
every agent session in the same project sees the same reservation set.

### `reserve`

| Field | Meaning | Default |
|---|---|---|
| `paths` (required) | project-relative paths or `*`/`**` globs | — |
| `ttl_secs` | hold time in seconds | `3600` |
| `exclusive` | only one session may own the surface | `true` |
| `holder_label` | human/agent label (e.g. "auth", "BlueLake") | — |
| `reason` | free-text reason | — |

Returns the created reservation id, expiry, and any **advisory conflicts**
against other sessions (overlaps are surfaced, not refused).

```
swarm {"action":"reserve","paths":["src/auth/*.rs"],"ttl_secs":3600,
       "holder_label":"auth","reason":"refactor auth"}
```

### `release`

Releases reservations held by the current session. With `paths` and
`reservation_ids` empty, releases everything the session holds.

```
swarm {"action":"release","paths":["src/auth/*.rs"]}
swarm {"action":"release"}                      # release all owned
```

### `reservations`

```
swarm {"action":"reservations"}
swarm {"action":"reservations","target_session":"<session-id>"}
```

## Edit-tool advisory warnings

The `edit` and `write` tools append a warning when another active session holds
an exclusive reservation on the file being changed. The edit is **not** refused
(reservations are advisory by design); the warning gives the agent a chance to
update the holder or proceed deliberately.

## Glob semantics

- `*` matches zero or more characters within one path segment (never `/`).
- `**` matches zero or more whole segments (across `/`).
- Exact paths are the strongest match; longer globs rank above shorter.

Paths are normalized to `/` with no leading slash before matching.

## Operations module

The backing store lives in `crates/jcode-base/src/reservation.rs`
(`pub mod reservation`). Storage mirrors the todo store: a JSON file per project
under `~/.jcode/reservations/<project-key>.json` with `storage::read_json` /
`write_json_fast`. Expiry is evaluated lazily: listing prunes expired entries;
stale holds are never counted as conflicts.

Public API: `reserve_paths`, `release_paths`, `list_reservations`,
`find_exclusive_reservation`, `exclusive_warning`, `glob_match`.