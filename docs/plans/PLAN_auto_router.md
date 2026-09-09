# PLAN: Auto Router Model (Cursor-style adaptive routing)

Status: v2.1 (premortem-revised + cross-model review findings folded)
Initiative: Auto Router Model
Scope: jcode-base provider layer, config, TUI picker/status, session persistence

## 1. Executive summary

Introduce a virtual `auto` model. When selected, jcode routes each turn to a
concrete model chosen from three tiers: FRONTIER (subscription OAuth Claude/GPT
flagships) for planning/judgment/first-turns, IMPLEMENT (subscription mid-tier)
for normal coding, and FAST (served-throughput >100 tps openweight/flash models
via Vercel AI Gateway, then OpenRouter) for mechanical work. The design is
premortem-informed: conversation-scoped provider affinity (no cross-provider
switching mid-conversation, which corrupts provider-affine history and forfeits
cache), conservative-first classification (a wrongly-cheap turn costs retries; a
wrongly-expensive turn costs cents), and mandatory transparency (visible
resolved model, decision audit, one-turn forced tier).

## 2. Background and why

Users hand-pick models and juggle cost/quality/throughput per task. Artificial
Analysis coding-agent data shows: token efficiency and cache hit rate materially
change effective cost; cheap-but-weak agents burn more tokens on retries;
upper-left (index vs cost) models are the value region. Vercel AI Gateway
pricing/latency shows a >100x price spread between frontier ($10/$50 per M) and
flash-class ($0.07/$0.24, 0.4-2s TTFT) models.

Premortem failure modes this design explicitly counters:
1. Naive keyword classifier routing "fix this + huge error log" to FAST →
   wrong patch → retry loops cost more than frontier. → conservative bias;
   ambiguous turns never down-shift.
2. Per-conversation stickiness freezing a bad first classification. →
   stickiness is per-category (same category → same model); category change
   re-routes within family.
3. Mid-conversation provider switch corrupting provider-affine history
   (Anthropic signed thinking, OpenAI encrypted reasoning) and killing prompt
   cache. → provider affinity per conversation; tier shifts only within the
   same provider family.
4. Users hating opacity. → auto→model status, /auto audit, /auto <tier>.
5. OpenRouter flakiness becoming default behavior. → FAST transport priority
   Vercel AI Gateway first; FAST tier absent entirely when no transport
   configured.
6. Hardcoded model ids rotting. → family-prefix seed patterns matched against
   the live catalog; versions auto-inherit the tier.

## 3. Non-goals

- No LLM-based classification (no extra model calls to decide routing).
- No cross-provider hopping mid-conversation (except hard failure escalation).
- No new provider runtime (Vercel uses the existing openai-compatible profile).
- No effort/latency-mode tricks for FAST: FAST means the model natively serves
  >100 output tps without special flags.
- No auto-routing for subagent/swarm spawns in v1 (reject `auto` there with an
  actionable error).

## 4. Core types and data model

### 4.1 Findings folded from cross-model review (gpt-5.5, verified)

1. **Virtual id collision (was Critical).** A real provider-local model named
   `auto` already exists (Belvedir, provider_catalog.rs:589-591), and the TUI
   groups picker rows by `route.model`, so bare `auto` collides.
   **Resolution:** the virtual id is namespaced as `jcode-auto`
   (`AUTO_MODEL_ID = "jcode-auto"`); `is_virtual_model` matches that string
   only. No collision with any live catalog id; picker grouping is
   unambiguous.
2. **Session restore leaks `auto` into concrete providers (was Critical).**
   Restore prefixes model ids by route method (`claude:auto` etc.,
   selection.rs:461-487), so a persisted virtual id can be sent to a concrete
   provider. **Resolution:** the virtual id is special-cased BEFORE any
   provider-key prefixing in `model_switch_request_for_session_route`: bare
   `jcode-auto` re-enters auto mode (re-resolve at first turn of the resumed
   conversation); prefixed forms (`claude:jcode-auto`) are treated as the bare
   virtual id too.
3. **Provider affinity vs FAST tier contradiction (was Critical).** Vercel/
   OpenRouter FAST belongs to different ActiveProvider families, so strict
   "no cross-provider" affinity makes FAST unreachable after an Anthropic
   frontier turn. **Resolution:** affinity is refined into *history-format
   affinity*: switching between providers is safe when the conversation
   history is portable (plain text + tool use/result with no
   provider-signed blocks). Provider-signed content (Anthropic thinking,
   OpenAI reasoning items) pins the conversation to that family. So:
   FAST (Vercel/OpenRouter) is reachable from a Claude-frontier conversation
   as long as replay of the frontier turn's signed blocks is not required;
   if it is (OpenAI reasoning chain), the FAST candidate is skipped and the
   in-family implement tier is used. The router checks `last_resolved.family
   == candidate.family OR history_is_portable(messages)` before selecting.
4. **No request-scoped dispatch (was Critical).** Dispatch is by
   ActiveProvider, and selecting a concrete model mutates shared provider
   state (mod.rs:1065-1283). **Resolution (v2.1 core change):** Auto does NOT
   mutate active provider/model per turn. Instead, resolve to a
   `RouteSelection` and call the resolved family's provider runtime directly
   via a new internal `complete_on_provider_with_model(provider, model_spec,
   ...)` seam: obtain the family's `Arc<dyn Provider>`, call its
   `set_route_selection`-equivalent on a **forked provider instance**
   (`fork()` already exists and is per-session) so shared state is never
   mutated. Concurrency: the fork is taken per completion call; the shared
   template is untouched, so concurrent turns/forks are race-free.
   `Provider::model()` of the fork reports the concrete model while the
   session-facing `MultiProvider::model()` still reports `jcode-auto`.
5. **Capability filtering order (High).** Image filtering runs before
   resolution using the active provider's capabilities. **Resolution:** when
   auto is active, image clamp/filter moves AFTER resolution and uses the
   resolved candidate's `supports_image_input()`.
6. **Escalation vs failover semantics (High).** Existing failover is
   provider-level and bails on cross-provider when a reason exists.
   **Resolution:** tier escalation happens INSIDE the auto seam (retry the
   same family one tier up on retryable errors, before returning to
   `complete_with_failover`'s provider failover), which composes with, not
   replaces, existing provider failover.
7. **Vercel assumptions (High).** Vercel has no static model fallback and
   routes carry `cheapness: None`, so live-catalog price ranking has no data
   for Vercel routes today. **Resolution:** FAST resolution for Vercel uses
   the cached live catalog only (no price ranking: seed tps order is
   authoritative); add `cheapness` for compatible-profile routes as a small
   follow-up, not launch-blocking.
8. **Typed route identity (Med-high).** `ModelRouteApiMethod` has no Auto
   variant; unknown methods become `Other` and lose identity.
   **Resolution:** add `ModelRouteApiMethod::Auto` and a matching
   `RuntimeKey`-aware `RouteSelection` so `routed_model_spec()` returns the
   bare virtual id and round-trips through set_route_selection.
9. **Remote protocol surface (Med-high).** `/auto` and forced tiers live in
   the daemon (MultiProvider state) but remote clients need read/write
   access. **Resolution:** extend the wire protocol: `set_auto_tier {tier?}`
   request and an `auto_state` field on the existing state/status event
   carrying `{active, last_resolved, decisions_tail}`. TUI local mode reads
   MultiProvider directly.
10. **Swarm spawn rejection (Med).** Spawns treat any non-`inherit` model as
    concrete; `auto` would create a route-less session and coordinator
    inheritance propagates the virtual id. **Resolution:** reject
    `is_virtual_model()` in spawn model resolution with an actionable error,
    and resolve `inherit`-from-auto to the coordinator's last concrete
    resolved model (not the virtual id).
11. **Context-window filtering (Med).** No authoritative per-candidate
    context metadata for gateway routes. **Resolution:** conservative default
    context for unknown gateway candidates (128k); only filter when metadata
    exists; rely on existing in-flight overflow errors + escalation as the
    backstop.
12. **Status data path (Med).** Widget has no virtual-vs-resolved field.
    **Resolution:** extend the model status payload with `resolved_model:
    Option<String>` set by the auto seam; widget renders `auto → <resolved>`
    when present; remote clients get it via the state event.

New module `crates/jcode-base/src/provider/auto_router.rs`:

```rust
pub const AUTO_MODEL_ID: &str = "jcode-auto";
pub fn is_virtual_model(model: &str) -> bool { model.trim() == AUTO_MODEL_ID }


#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AutoTier { Frontier, Implement, Fast }

// Request classification (explicit signals only)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TurnCategory { Planning, Implementation, Mechanical, Unknown }

pub struct AutoDecision {
    pub tier: AutoTier,
    pub model_spec: String,        // concrete routed spec, e.g. "zai/glm-5.3-flash"
    pub provider_family: String,   // "anthropic" | "openai" | "vercel-ai-gateway" | "openrouter" | ...
    pub reason: String,            // human-readable, shown by /auto
    pub at: std::time::Instant,
}

// Per-conversation state (inside MultiProvider, RwLock<AutoRouteState>)
pub struct AutoRouteState {
    pub conversation_provider_family: Option<String>, // chosen at first turn
    pub last_category: Option<TurnCategory>,
    pub last_resolved: Option<AutoDecision>,
    pub forced_next_tier: Option<AutoTier>, // /auto <tier>, one turn only
    pub decisions: VecDeque<AutoDecision>,  // bounded (last 50)
}
```

Config (`jcode-config-types` + `[auto_router]` in config.toml):

```toml
[auto_router]
enabled = true                    # default true
fast_provider = "auto"            # "auto" | "vercel" | "openrouter" | "none"
frontier = ""                     # optional model override (routed spec)
implement = ""                    # optional model override
fast = ""                         # optional model override
```

## 5. Tier resolution

FRONTIER (subscription-first): Anthropic OAuth Opus/Fable → OpenAI OAuth
GPT-codex flagship. Only families with live credentials are candidates.

IMPLEMENT (subscription mid): Anthropic OAuth Sonnet-class → OpenAI OAuth
GPT mid-tier.

FAST (>100 tps as served, transport priority):
1. Vercel AI Gateway (built-in profile `vercel-ai-gateway`,
   api_base https://ai-gateway.vercel.sh/v1, key AI_GATEWAY_API_KEY) —
   preferred; serves exactly the throughput class.
2. OpenRouter (fallback transport).
3. Subscription flash (e.g. Gemini flash OAuth) — last resort.
None configured → FAST tier does not exist; Mechanical down-shifts collapse to
IMPLEMENT (never up-shift silently).

Seed table (family-prefix patterns, not pinned versions), ranked by seeded tps
then live-catalog price:

| pattern | example ids | seeded tps class |
|---|---|---|
| `*mercury*` | inception/mercury-2.5 | ~150+ |
| `glm-*-flash` / `zai/glm-*-flash` | zai/glm-5.3-flash | ~120 |
| `qwen*-flash` | alibaba/qwen3.8-flash | ~110 |
| `deepseek-v*flash` | deepseek/deepseek-v4.1-flash | ~110 |
| `gemini-*-flash` | google/gemini-3.8-flash | ~100+ |
| `minimax-h*` | minimax/minimax-h3 | ~100+ |

Resolution intersects seed patterns with the live catalog of the chosen
transport; unmatched/deprecated entries drop automatically. A config override
string skips all heuristics for that tier.

## 6. Classifier (explicit signals, conservative)

Input: message vector + tool defs + turn index within conversation.

- First substantive user turn of the conversation → Planning (FRONTIER).
- Planning/judgment signals in the latest user text: plan/architect/design/
  refactor/review/why/how-does (word-boundary), or todo/plan tool usage in the
  assistant's last turn → Planning.
- Mechanical signals: the latest message is ONLY tool results (no new user
  intent), or short user text (<200 chars) matching mechanical patterns
  (format/rename/run tests/lint/fix typo) → Mechanical.
- Everything else → Implementation.
- Adversarial guard: large tool-result/error-log content with a short "fix
  this" does NOT classify Mechanical — it is Unknown → Implementation at
  minimum, Planning if the conversation is new.

Stickiness rule: if the new category equals `last_category`, reuse
`last_resolved` (cache + stability). Category change → re-resolve within the
same provider family. `forced_next_tier` (from /auto <tier>) overrides exactly
one turn and clears.

## 7. Interception points (grounded in code)

- `MultiProvider::set_model` (mod.rs:2060): accept `auto`, store
  `auto_active=true`, do not switch sub-providers; reserve id.
- `MultiProvider::model` (mod.rs:1866): return `auto` when active.
- `MultiProvider::fork` (mod.rs:2940): carry `auto_active` + conversation state.
- `MultiProvider::complete_with_failover` (mod.rs:612): when auto active,
  resolve via auto_router FIRST, then dispatch to the resolved family's
  provider with the concrete model (existing failover still applies within the
  resolved provider). Record decision; escalate one tier on failure
  (Fast→Implement→Frontier; cross-provider only if the family is dead).
- `catalog_routes.rs` simplified + full route builders: pinned "Auto" entry;
  available when ≥2 tiers have candidates; detail lists tier models.
- `session.rs`: persist `auto` as the model id; restore re-resolves.
- Guards: swarm spawn `model=auto` → actionable error; telemetry price lookup
  maps virtual id to the resolved decision; `/model` display shows
  `auto → <resolved>` (info_widget_model + model_context).

## 8. Error handling and edge cases

- No tier candidates at all (no auth) → Auto route unavailable in picker; if
  selected anyway, fall back to normal default-model behavior with a notice.
- Estimated request tokens (reuse estimate_request_input) exceed a candidate's
  context window → drop candidate, take next in tier; log reason.
- Failure escalation: retry once one tier up; cross-provider only as last
  resort with a system notice (provider-affine history risk acknowledged).
- /rewind: rewind also clears `forced_next_tier`; conversation state survives.
- Compaction: conversation_provider_family survives compaction (state lives in
  provider, not messages).
- Concurrency: AutoRouteState behind the same RwLock discipline as other
  MultiProvider state.

## 9. Roadmap (dependency-ordered)

1. `auto_router.rs` core: tiers, seed table, classifier, state (no wiring).
   Tests: classifier truth table incl. adversarial cases; seed matching;
   tier fallback; context filtering; stickiness.
2. MultiProvider wiring: set_model/model/fork/complete_with_failover,
   virtual-id guards. Tests: dispatch to mock resolved provider, escalation,
   fork/session round-trip.
3. Config `[auto_router]` + route-builder Auto entries. Tests: route presence
   and availability.
4. TUI: picker entry, status `auto → model`, `/auto` + `/auto <tier>`,
   notices. Tests: command handling, status rendering.
5. Selfdev build, reload, manual mixed-transcript verification.

## 10. Comparison table

| Alternative | Rejected because |
|---|---|
| v1 per-turn classification + cross-provider hops | provider-affine history corruption, cache loss, opacity (premortem) |
| LLM-based router calls | extra cost/latency on every turn |
| OpenRouter-only FAST | flakiness becomes default; Vercel serves the same class cheaper |
| Pinned model ids | rot; family-prefix matching auto-inherits versions |
| Per-conversation model freeze | first classification pins all 40 later mechanical turns to Opus |
| Task-boundary tracking | complexity; category-based stickiness achieves the same cache goal |

## 11. Open questions

- Within-family downshift ladder for OpenAI (which codex ids = implement tier)
  resolved from live catalog metadata at implementation time.
- Local tps measurement (observed output tokens / wall time per model in
  ~/.jcode) as a self-correcting ranker: follow-up, not launch scope.
- Should /auto fast persist for session? Default decided: one turn only.
