# Named agent routing

Use named policies when worker selection must not depend on a coordinator's model hints.
The `agent_role` routing name is separate from the `role: "agent" | "coordinator"`
field that controls swarm topology and permissions. Labels remain display text only.

## Configuration

```toml
[agents]
default_role = "implementer"
swarm_spawn_mode = "inline"

[agents.roles.implementer]
model = "openai-api:gpt-5.6-luna"
fallbacks = ["claude-api:claude-opus-4-6"]

[agents.roles.reviewer]
model = "claude-api:claude-opus-4-6"
```

Role names are case-sensitive and contain ASCII letters, digits, `_`, or `-`.
Each role requires a concrete `model`. `fallbacks` is optional and ordered.
Empty models, inheritance/auto sentinels, whitespace, unknown role-table fields,
unknown `[agents]` fields, and an undefined `default_role` are configuration errors.
Worker dispatch reads configuration strictly, so malformed TOML cannot silently
replace an enforcement policy with defaults. Fix the configuration before retrying.

Use `swarm list_models` to find locally available provider/auth routes. Prefer
qualified identifiers such as `openai-api:<model>`, `openai-oauth:<model>`,
`claude-api:<model>`, or `<compatible-profile>:<model>`. Bare model IDs are allowed
only when the available catalog identifies one unambiguous route. Multiple
available routes require qualification rather than a catalog-order tiebreaker.
For an OpenRouter upstream pin, use `<catalog-model>@<upstream-provider>`.

## Precedence and dispatch

```json
{"action":"spawn","label":"API implementation","agent_role":"implementer","model":"inherit","prompt":"Implement the API change"}
```

1. An explicit `agent_role` selects that configured policy. Unknown or blank names fail.
2. Otherwise, `agents.default_role` applies. Setting it prevents omission from bypassing policy.
3. The selected named policy overrides **all** worker `model` hints, including `inherit`.
4. Without a named policy, existing precedence remains: explicit model, `agents.swarm_model`, then coordinator inheritance.

The routing input is forwarded through `spawn`, `assign_task`, `assign_next`,
`fill_slots`, and `run_plan`, including background plan drivers. Assignment reuse
is constrained by the selected role, model, provider, and auth route. Explicit
worker targets do not bypass that check. A mismatched worker is never silently
switched. Automatic selection excludes it and, when spawning is enabled, creates
a matching worker instead. Otherwise assignment fails without dispatching the task.

The selection is persisted on both the worker and assigned task. Explicit task
recovery (`retry`, `replace`, `reassign`, `salvage`, `start`, `wake`, `resume`)
preserves the task's recorded route, including after its former worker is removed.
An unrelated unavailable default does not override a recorded task policy.
Legacy tasks without a recorded route use the current default policy.
Recovery actions do not accept a new `agent_role`; create a new assignment to
intentionally change the routing policy.

## Exact fallback boundary

Fallbacks are **catalog-availability selection**, not inference retries:

- Before dispatch, check the primary candidate and then each fallback in order.
- Choose the first candidate with one locally available catalog route.
- Preserve the complete selection, including a compatible profile or OpenRouter upstream pin.
- Initialize a provider fork and verify its actual model, provider, and auth identity before starting a named worker.
- If the catalog says available but initialization fails, return that error. Do not try another candidate after initialization starts.

Catalog availability does not prove that credentials are valid remotely, a quota
is sufficient, or an endpoint will accept a request. There is no live inference
probe. If a worker later encounters authentication, quota, transport, or model
errors, this policy does not replay its task on a fallback. Existing explicit
recovery commands remain deliberate coordinator actions, not automatic fallback.
Normal provider transport retries are separate from this routing policy.

Named provider-qualified creation requests must not silently substitute another
provider/auth route, even when the other route serves the same model. Legacy
unqualified opaque/custom model IDs remain supported without requiring catalog
membership. Named policies require catalog membership to make availability and
ambiguity checks deterministic.

## Inspecting the decision

`spawn` results, `list`, and `status` expose the selected `routing` object:

```json
{
  "agent_role": "implementer",
  "model": "gpt-5.6-luna",
  "provider": "OpenAI",
  "api_method": "openai-api-key",
  "provider_key": "openai-api-key"
}
```

Text output includes the same role, model, provider, and route. The selection
survives swarm persistence/reload and spawn-response deduplication. A historical
selection is not sufficient for reuse: the current provider is checked as well.
When a worker is busy, persisted exact identity is used conservatively. Busy
OpenRouter workers are not reusable when their upstream pin cannot be verified.

## Scope and validation

This feature covers the active swarm tool and its server protocol. It does not
restore the legacy, unregistered `subagent` tool referenced by old protocol paths.
No desktop-specific behavior is required.

Focused tests live in `server/named_agent_routing_tests.rs`, with assignment and
provider-runtime regression tests alongside their existing suites. Runtime smoke
should use a separate `JCODE_HOME` and socket, never the user's configuration or
shared daemon. Build with `selfdev build target=tui` before running that smoke.
