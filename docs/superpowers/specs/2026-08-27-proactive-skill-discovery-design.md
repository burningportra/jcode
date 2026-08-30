# Proactive Skill Discovery

Date: 2026-08-27
Status: approved for the smallest implementation slice

## Goal

Let the user ask Jcode to scan recent persisted sessions for one repeated workflow worth turning into a global skill. Discovery never installs a skill and never widens autonomy.

## Smallest slice

Add four `skill_manage` actions:

- `discover_crystallization` scans at most 100 recent Jcode sessions and returns one high-confidence suggestion.
- `review_crystallization` shows the evidence and the exact existing `crystallize` call shape.
- `dismiss_crystallization` hides only that evidence snapshot.
- `suppress_crystallization` permanently hides that normalized workflow pattern until its state file is removed.

The public output labels the controls **Review**, **Dismiss**, and **Never suggest this**. The existing `crystallize` and `approve_crystallization` actions remain the only proposal and installation path.

## Detection rule

The first slice intentionally favors precision over recall. A candidate must be the same normalized, eligible user message in at least three distinct persisted sessions. Normalization collapses whitespace and compares case-insensitively. Messages shorter than 20 characters, longer than 2,000 characters, approval messages, and internal or scheduled messages are excluded.

Candidates are ranked by distinct-session count, then recency, then stable fingerprint. At most 12 evidence references are retained. The scanner is bounded and deterministic.

## State and identity

- `pattern_id` is the SHA-256 digest of normalized workflow text.
- `suggestion_id` also includes the exact sorted evidence references.
- Suggestions live under `~/.jcode/skill-crystallization/discovery/`.
- Dismissed suggestion IDs and suppressed pattern IDs use bounded JSON state.
- All mutations reuse the existing process and filesystem crystallization lock.

Dismissal allows a materially newer evidence snapshot to surface. Suppression blocks the workflow pattern regardless of later examples.

## Review and approval flow

1. Discovery returns one suggestion and three action calls in versioned metadata.
2. Review reloads the persisted suggestion, revalidates every evidence reference, and shows the repeated workflow.
3. The agent drafts a focused skill and calls the existing `crystallize` action with those references.
4. The existing persisted user-approval gate installs the skill.

## Failure behavior

Unreadable or invalid sessions are skipped and counted. Corrupt discovery state fails closed rather than forgetting suppression. A suggestion whose evidence changed cannot be reviewed. No match is a successful, explicit empty result.

## Acceptance

- Three matching sessions produce one reviewable suggestion.
- Two matching sessions do not.
- Review uses the existing crystallization evidence shape.
- Dismiss hides only the current snapshot.
- Never suggest hides the pattern after new matching sessions appear.
- Approval still requires the existing persisted user confirmation.
