//! Graph-theory triage over a swarm plan (the `bv` role).
//!
//! `summarize_plan_graph` tells you *what is ready* vs *what is blocked*. This
//! module answers the harder question the coordination tools care about: among
//! the runnable (ready) items, **which one unlocks the most downstream work**.
//!
//! It computes classical graph metrics over the `blocked_by` dependency graph:
//!
//! - **Critical-path depth** — the longest dependency chain rooted at an item.
//!   A deep ready item is close to many dependents, so clearing it has a large
//!   multiplicative payoff.
//! - **Out-degree reach / "unblock reach"** — how many other open items would
//!   become runnable once this item completes (transitive dependents minus
//!   already-completed predecessors). This is the closest cheap surrogate for
//!   betweenness centrality on a DAG and is the flywheel's "what unlocks the
//!   most" signal.
//! - **PageRank** — a power-iteration centrality that weights items many open
//!   items transitively depend on.
//!
//! The result ranks runnable items deterministically by
//! `unblock_reach` → `critical_depth` → `page_rank` → priority → id, so any
//! agent asking "what's next" gets the same answer without a central planner.

use crate::PlanItem;
use std::collections::{HashMap, HashSet, VecDeque};

/// One triaged item: a runnable (ready) item plus its computed metrics.
#[derive(Debug, Clone, PartialEq)]
pub struct TriageEntry {
    pub id: String,
    pub content: String,
    pub priority: String,
    /// Longest dependency chain rooted at this item (self = 0 when leaf).
    pub critical_depth: usize,
    /// How many other open items become runnable if this one completes.
    pub unblock_reach: usize,
    /// Power-iteration PageRank (0..=1 scale, sorted descending).
    pub page_rank: f64,
    /// Composite rank (lower = do first).
    pub rank: usize,
}

/// Result of triaging a plan: ranked runnable items plus aggregate counts.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GraphTriage {
    /// Runnable items, best-first.
    pub entries: Vec<TriageEntry>,
    /// Total open (non-terminal) items in the plan.
    pub open_count: usize,
    /// Number of runnable items.
    pub ready_count: usize,
}

/// PageRank via power iteration over dependency edges (dependents → dependents
/// that this item blocks). Returns `id → rank`, normalized so max = 1.0.
pub fn page_rank(items: &[PlanItem], iterations: usize) -> HashMap<String, f64> {
    let item_ids: HashSet<&str> = items.iter().map(|i| i.id.as_str()).collect();
    // dependents[d] = ids that d blocks (forward edges d -> dependent).
    let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();
    for item in items {
        for dep in item
            .blocked_by
            .iter()
            .filter(|dep| item_ids.contains(dep.as_str()))
        {
            dependents.entry(dep.as_str()).or_default().push(item.id.as_str());
        }
    }
    if dependents.is_empty() {
        return HashMap::new();
    }

    let damping = 0.85;
    let n = items.len().max(1) as f64;
    let mut rank: HashMap<&str, f64> = HashMap::new();
    for item in items {
        rank.insert(item.id.as_str(), 1.0 / n);
    }
    // Reverse map for the "which nodes point to me" needed by PageRank.
    // PageRank(node) = (1-d)/n + d * sum(rank(predecessor)/out_degree(predecessor)).
    // A node's "predecessors" are the items it is blocked_by.
    let mut blocked_by: HashMap<&str, Vec<&str>> = HashMap::new();
    for item in items {
        for dep in item
            .blocked_by
            .iter()
            .filter(|dep| item_ids.contains(dep.as_str()))
        {
            blocked_by.entry(item.id.as_str()).or_default().push(dep.as_str());
        }
    }
    // out_degree of each node = how many others it blocks (its dependents).
    for _ in 0..iterations {
        let mut next: HashMap<&str, f64> = HashMap::new();
        for item in items {
            let id = item.id.as_str();
            let mut sum = 0.0;
            if let Some(preds) = blocked_by.get(id) {
                for pred in preds {
                    let out = dependents.get(pred).map(|v| v.len()).unwrap_or(1).max(1) as f64;
                    sum += rank.get(pred).copied().unwrap_or(0.0) / out;
                }
            }
            next.insert(id, (1.0 - damping) / n + damping * sum);
        }
        rank = next;
    }
    // Normalize so max = 1.0 for stable, comparable output.
    let max = rank.values().cloned().fold(0.0_f64, f64::max);
    if max > 0.0 {
        rank.iter_mut().for_each(|(_, v)| *v /= max);
    }
    rank.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

/// Longest dependency chain (in items, self-inclusive) for each id. A leaf with
/// no open-graph depth scores 0; an item that transitively blocks a long chain
/// scores the chain length.
pub fn critical_depths(items: &[PlanItem]) -> HashMap<String, usize> {
    let item_ids: HashSet<&str> = items.iter().map(|i| i.id.as_str()).collect();
    // depth[id] = 1 + max depth over its dependencies (blocked_by) present in the graph.
    let mut blocked_by: HashMap<String, Vec<String>> = HashMap::new();
    for item in items {
        for dep in item
            .blocked_by
            .iter()
            .filter(|dep| item_ids.contains(dep.as_str()))
        {
            blocked_by
                .entry(item.id.clone())
                .or_default()
                .push(dep.clone());
        }
    }

    let mut memo: HashMap<String, usize> = HashMap::new();
    fn depth(
        id: &str,
        blocked_by: &HashMap<String, Vec<String>>,
        memo: &mut HashMap<String, usize>,
        visited: &mut HashSet<String>,
    ) -> usize {
        if let Some(&d) = memo.get(id) {
            return d;
        }
        if !visited.insert(id.to_string()) {
            // Cycle guard: don't recurse infinitely; treat as depth 1.
            return 1;
        }
        let child_max = blocked_by
            .get(id)
            .map(|preds| {
                preds
                    .iter()
                    .map(|p| depth(p, blocked_by, memo, visited))
                    .max()
                    .unwrap_or(0)
            })
            .unwrap_or(0);
        visited.remove(id);
        let d = child_max + 1;
        memo.insert(id.to_string(), d);
        d
    }

    for item in items {
        let mut visited = HashSet::new();
        depth(item.id.as_str(), &blocked_by, &mut memo, &mut visited);
    }
    // Leaf derivation for ids with no dependencies gets 0; internal nodes get chain length - 1
    // so leaves read as depth 0 (a single item to do, nothing blocked behind it).
    memo.into_iter()
        .map(|(k, v)| (k, v.saturating_sub(1)))
        .collect()
}

/// For each open (non-terminal) item, how many other *open* items transitively
/// depend on it (its transitive dependents). This is the "what unlocks the
/// most" signal: completing an item with a large downstream closure frees the
/// most other work.
pub fn unblock_reach(items: &[PlanItem], completed_ids: &HashSet<String>) -> HashMap<String, usize> {
    let item_ids: HashSet<&str> = items.iter().map(|i| i.id.as_str()).collect();
    let open: HashSet<String> = items
        .iter()
        .map(|i| i.id.clone())
        .filter(|id| !completed_ids.contains(id))
        .collect();
    // dependents[d] = open items directly blocked by d.
    let mut dependents: HashMap<String, Vec<String>> = HashMap::new();
    for item in items {
        for dep in item
            .blocked_by
            .iter()
            .filter(|dep| item_ids.contains(dep.as_str()))
        {
            if open.contains(dep) && open.contains(&item.id) {
                dependents
                    .entry(dep.clone())
                    .or_default()
                    .push(item.id.clone());
            }
        }
    }

    // Seed every open id so leaves (no dependents) read as 0 explicitly.
    let mut reach: HashMap<String, usize> = open.iter().map(|id| (id.clone(), 0usize)).collect();
    for id in &open {
        // BFS over direct dependents; each visited open node counts as one
        // downstream item this completion unlocks.
        let mut seen = HashSet::new();
        let mut queue = VecDeque::new();
        if let Some(start) = dependents.get(id) {
            for d in start {
                queue.push_back(d.clone());
            }
        }
        let mut count = 0usize;
        while let Some(node) = queue.pop_front() {
            if !seen.insert(node.clone()) || !open.contains(&node) {
                continue;
            }
            count += 1;
            if let Some(children) = dependents.get(&node) {
                for c in children {
                    if !seen.contains(c) {
                        queue.push_back(c.clone());
                    }
                }
            }
        }
        if count > 0 {
            reach.insert(id.clone(), count);
        }
    }
    reach
}

/// Compute triage metrics and return a deterministic best-first ranking of the
/// runnable (ready) items. Non-runnable items are excluded from the ranking.
pub fn triage(items: &[PlanItem], ready_ids: &[String]) -> GraphTriage {
    let completed_ids: HashSet<String> = items
        .iter()
        .filter(|i| crate::is_completed_status(&i.status))
        .map(|i| i.id.clone())
        .collect();
    let rank = page_rank(items, 24);
    let depths = critical_depths(items);
    let reach = unblock_reach(items, &completed_ids);

    let ready: HashSet<&str> = ready_ids.iter().map(String::as_str).collect();
    let mut entries: Vec<TriageEntry> = items
        .iter()
        .filter(|i| ready.contains(i.id.as_str()))
        .map(|i| TriageEntry {
            id: i.id.clone(),
            content: i.content.clone(),
            priority: i.priority.clone(),
            critical_depth: depths.get(&i.id).copied().unwrap_or(0),
            unblock_reach: reach.get(&i.id).copied().unwrap_or(0),
            page_rank: rank.get(&i.id).copied().unwrap_or(0.0),
            rank: 0,
        })
        .collect();

    // Sort: unblock_reach desc, critical_depth desc, page_rank desc, priority asc, id asc.
    entries.sort_by(|a, b| {
        b.unblock_reach
            .cmp(&a.unblock_reach)
            .then_with(|| b.critical_depth.cmp(&a.critical_depth))
            .then_with(|| b.page_rank.partial_cmp(&a.page_rank).unwrap_or(std::cmp::Ordering::Equal))
            .then_with(|| crate::priority_rank(&a.priority).cmp(&crate::priority_rank(&b.priority)))
            .then_with(|| a.id.cmp(&b.id))
    });
    for (i, entry) in entries.iter_mut().enumerate() {
        entry.rank = i + 1;
    }

    let open_count = items
        .iter()
        .filter(|i| !crate::is_terminal_status(&i.status))
        .count();

    GraphTriage {
        open_count,
        ready_count: entries.len(),
        entries,
    }
}
