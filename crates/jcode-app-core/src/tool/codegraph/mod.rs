//! Live (unindexed) code-graph core for graph-AI Phase 1 (`jcode-rey`).
//!
//! Plain blocking functions over `std::fs` + `Command(rg, git)` — no tokio,
//! no `ToolContext`, no `Tool::execute`. Tools (`code_impact` in `jcode-nd5`,
//! `code_query` in `jcode-hg3`) are thin async wrappers that resolve the repo
//! root from `ToolContext::working_dir` and call these inside `spawn_blocking`
//! (mirrors `agentgrep`'s offload pattern). Phase 3 moves this module into the
//! `jcode-codegraph` crate and adds `indexed.rs` behind the same signatures.
//!
//! Degraded-first contract (plan 5.1 + tool edge cases): every query returns a
//! [`GraphSource`] naming the backend that produced it; missing `rg`, non-git
//! repos, unreadable files, and timeouts degrade to partial results, never
//! hard errors (except path traversal, which is rejected).

pub mod cache;
pub mod indexed;
pub mod live;
pub mod map;

#[cfg(test)]
mod tests;

pub use cache::DepCache;
pub use indexed::{codegraph_disabled, db_path_for, indexed_or_live, rel_for};
pub use live::{
    GraphSource, LiveGraph, cochanges_live, dependencies_live, dependents_live,
    exported_symbols_live, resolve_repo_root,
};
