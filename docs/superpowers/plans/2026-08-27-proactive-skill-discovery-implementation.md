# Proactive Skill Discovery implementation plan

1. Add a bounded discovery module beside crystallization with deterministic candidate identity, recent-session scanning, persisted suggestion records, dismissal, and suppression.
2. Extend `skill_manage` with discover, review, dismiss, and suppress actions plus action-specific schemas and versioned metadata.
3. Route Review into the existing `crystallize` call shape without creating a second proposal or approval mechanism.
4. Add detector, persistence, tamper, public workflow, and regression tests.
5. Run focused and complete SkillTool tests, no-default-features compilation, live public-interface acceptance, commit, build, and reload.
