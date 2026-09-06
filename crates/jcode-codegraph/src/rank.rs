//! PageRank over the import graph + git co-change mining (plan pass-8: weight
//! semantics defined — weight = distinct import statements src→dst, normalized
//! per-source so out-weights sum to 1; zero-out-degree nodes distribute
//! uniformly. Damping 0.85, 20 iterations).

use std::collections::{HashMap, HashSet};

/// Weighted edge src → dst (file ids or paths; generic over node key).
pub fn pagerank<N>(nodes: &[N], edges: &[(N, N, f64)]) -> HashMap<N, f64>
where
    N: Eq + std::hash::Hash + Clone,
{
    const DAMPING: f64 = 0.85;
    const ITERS: usize = 20;
    let n = nodes.len().max(1) as f64;
    let mut rank: HashMap<N, f64> = nodes.iter().map(|x| (x.clone(), 1.0 / n)).collect();
    // Per-source out-weight normalization.
    let mut out_sum: HashMap<N, f64> = HashMap::new();
    for (s, _, w) in edges {
        *out_sum.entry(s.clone()).or_insert(0.0) += w;
    }
    // Inbound adjacency.
    let mut inbound: HashMap<N, Vec<(N, f64)>> = HashMap::new();
    for (s, d, w) in edges {
        let norm = w / out_sum.get(s).cloned().unwrap_or(1.0).max(f64::MIN);
        inbound
            .entry(d.clone())
            .or_default()
            .push((s.clone(), norm));
    }
    for _ in 0..ITERS {
        let mut next = HashMap::new();
        let dangling: f64 = nodes
            .iter()
            .filter(|x| !out_sum.contains_key(*x))
            .map(|x| rank.get(x).cloned().unwrap_or(0.0))
            .sum();
        for node in nodes {
            let mut sum = dangling / n;
            if let Some(ins) = inbound.get(node) {
                for (s, w) in ins {
                    sum += rank.get(s).cloned().unwrap_or(0.0) * w;
                }
            }
            next.insert(node.clone(), (1.0 - DAMPING) / n + DAMPING * sum);
        }
        rank = next;
    }
    rank
}

/// Parse `git log --name-only --pretty=format:COMMIT:%H` output into canonical
/// co-change pairs (`(a, b)` with `a < b`) → commit counts. Full-log parse:
/// never `-- <path>` (it hides partners — see live core lesson).
pub fn cochanges_from_log(log: &str) -> HashMap<(String, String), usize> {
    let mut counts = HashMap::new();
    let mut block: Vec<String> = vec![];
    let mut flush = |block: &mut Vec<String>| {
        block.sort();
        block.dedup();
        for i in 0..block.len() {
            for j in (i + 1)..block.len() {
                let (a, b) = if block[i] < block[j] {
                    (block[i].clone(), block[j].clone())
                } else {
                    (block[j].clone(), block[i].clone())
                };
                *counts.entry((a, b)).or_insert(0) += 1;
            }
        }
        block.clear();
    };
    for line in log.lines() {
        let f = line.trim();
        if f.starts_with("COMMIT:") {
            flush(&mut block);
        } else if !f.is_empty() {
            block.push(f.to_string());
        }
    }
    flush(&mut block);
    counts
}

/// Partners of `file` sorted by count desc. Queries both canonical directions.
pub fn cochange_partners(
    pairs: &HashMap<(String, String), usize>,
    file: &str,
) -> Vec<(String, usize)> {
    let mut out: HashMap<String, usize> = HashMap::new();
    for ((a, b), c) in pairs {
        if a == file {
            *out.entry(b.clone()).or_insert(0) += c;
        } else if b == file {
            *out.entry(a.clone()).or_insert(0) += c;
        }
    }
    let mut v: Vec<(String, usize)> = out.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    v
}

/// Dedupe helper for tests.
#[allow(dead_code)]
pub fn _dedup_note(_x: &HashSet<String>) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pagerank_flows_to_imported() {
        let nodes = vec!["a", "b"];
        let edges = vec![("a", "b", 1.0)];
        let r = pagerank(&nodes, &edges);
        assert!(r["b"] > r["a"], "r={r:?}");
    }

    #[test]
    fn pagerank_uniform_without_edges() {
        let nodes = vec!["a", "b", "c"];
        let r = pagerank::<&str>(&nodes, &[]);
        for v in r.values() {
            assert!((v - 1.0 / 3.0).abs() < 1e-9);
        }
    }

    #[test]
    fn cochange_pairs_canonical() {
        let log = "COMMIT:1\na.txt\nb.txt\nCOMMIT:2\nb.txt\na.txt\n";
        let pairs = cochanges_from_log(log);
        assert_eq!(
            pairs.get(&("a.txt".to_string(), "b.txt".to_string())),
            Some(&2)
        );
        let partners = cochange_partners(&pairs, "a.txt");
        assert_eq!(partners, vec![("b.txt".to_string(), 2)]);
    }
}
