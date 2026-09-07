//! BM25 ranked symbol retrieval: name-exact + conceptual lanes with route
//! disclosure (kev-t5h, mirrors ripwire's `route=` honesty surface).
//!
//! Pure functions over symbol rows: no SQLite here (callers fetch rows via
//! `GraphDb::symbols`-shaped queries, then rank in memory). Constants mirror
//! `memory.rs::bm25_rank` (k1=1.2, b=0.75).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lane {
    NameExact,
    Conceptual,
}

impl Lane {
    pub fn as_str(&self) -> &'static str {
        match self {
            Lane::NameExact => "name-exact",
            Lane::Conceptual => "conceptual",
        }
    }
}

#[derive(Debug, Clone)]
pub struct RankedSymbol {
    pub path: String,
    pub name: String,
    pub kind: String,
    pub line: i64,
    pub score: f32,
    pub lane: Lane,
}

#[derive(Debug, Clone)]
pub struct SymbolDoc {
    pub path: String,
    pub name: String,
    pub kind: String,
    pub line: i64,
}

/// Identifier-shaped query: a single token that looks like a symbol name
/// (camelCase, snake_case, scoped path). Multi-word queries always route
/// conceptual — the all-words-name test from ripwire's router is deliberately
/// omitted (its plausibility gate needs corpus stats; without them it
/// misroutes prose like "blast radius dependents" where every word happens
/// to match some symbol).
pub fn rank_route(query: &str, _known: &std::collections::HashSet<String>) -> Lane {
    let words: Vec<&str> = query.split_whitespace().collect();
    if words.len() == 1 && is_identifier(words[0]) {
        return Lane::NameExact;
    }
    Lane::Conceptual
}

fn is_identifier(tok: &str) -> bool {
    let t = tok.trim_matches(|c: char| !c.is_alphanumeric() && c != '_');
    if t.is_empty() {
        return false;
    }
    let has_case_mix = t.chars().any(|c| c.is_uppercase())
        || t.contains('_')
        || t.contains("::")
        || t.contains('.');
    has_case_mix
        && t.chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == ':' || c == '.')
}

/// Split camelCase/snake_case/scoped names and whitespace-separated phrases
/// into lowercase subtokens.
pub fn subtokens(name: &str) -> Vec<String> {
    let mut out = vec![];
    for part in name.split(|c: char| {
        c == '_' || c == ':' || c == '.' || c == '/' || c == '-' || c.is_whitespace()
    }) {
        let mut cur = String::new();
        for (i, c) in part.char_indices() {
            if c.is_uppercase() && i > 0 {
                if !cur.is_empty() {
                    out.push(cur.to_lowercase());
                }
                cur = String::new();
            }
            cur.push(c);
        }
        if !cur.is_empty() {
            out.push(cur.to_lowercase());
        }
    }
    out.retain(|s| !s.is_empty());
    out
}

/// Name-exact lane: whole-name match, ordered exact > case-folded > prefix.
/// Deterministic tiers; no scores beyond tier order. Empty query → empty.
pub fn rank_name_exact(symbols: &[SymbolDoc], query: &str, top_k: usize) -> Vec<RankedSymbol> {
    let q = query.trim();
    if q.is_empty() {
        return vec![];
    }
    let q_lower = q.to_lowercase();
    let mut exact = vec![];
    let mut folded = vec![];
    let mut prefix = vec![];
    for s in symbols {
        if s.name == q {
            exact.push(s);
        } else if s.name.to_lowercase() == q_lower {
            folded.push(s);
        } else if s.name.to_lowercase().starts_with(&q_lower) {
            prefix.push(s);
        }
    }
    let mut out = vec![];
    for (tier, group) in [(3.0, exact), (2.0, folded), (1.0, prefix)] {
        let mut g: Vec<&SymbolDoc> = group;
        g.sort_by(|a, b| a.path.cmp(&b.path).then(a.line.cmp(&b.line)));
        for s in g {
            out.push(RankedSymbol {
                path: s.path.clone(),
                name: s.name.clone(),
                kind: s.kind.clone(),
                line: s.line,
                score: tier,
                lane: Lane::NameExact,
            });
        }
    }
    if top_k > 0 {
        out.truncate(top_k);
    }
    out
}

/// Conceptual lane: BM25 over per-symbol docs of name + kind + path subtokens.
/// Path subtokens score at half weight (fixture-pollution guard). Zero-overlap
/// rows dropped. top_k=0 means no limit.
pub fn rank_conceptual(symbols: &[SymbolDoc], query: &str, top_k: usize) -> Vec<RankedSymbol> {
    const K1: f32 = 1.2;
    const B: f32 = 0.75;
    const PATH_WEIGHT: f32 = 0.5;

    let q_terms = subtokens(query);
    if q_terms.is_empty() {
        return vec![];
    }
    let q_set: std::collections::HashSet<&String> = q_terms.iter().collect();

    // Per-symbol token streams: name tokens full weight, kind full, path half
    // (implemented by counting path tokens once but scaling their tf).
    struct Doc {
        idx: usize,
        tf: std::collections::HashMap<String, f32>,
        len: f32,
    }
    let mut docs: Vec<Doc> = vec![];
    let mut df: std::collections::HashMap<String, f32> = std::collections::HashMap::new();
    for (idx, s) in symbols.iter().enumerate() {
        let mut tf: std::collections::HashMap<String, f32> = std::collections::HashMap::new();
        for t in subtokens(&s.name) {
            *tf.entry(t).or_insert(0.0) += 1.0;
        }
        for t in subtokens(&s.kind) {
            *tf.entry(t).or_insert(0.0) += 1.0;
        }
        for t in subtokens(&s.path) {
            *tf.entry(t).or_insert(0.0) += PATH_WEIGHT;
        }
        let len: f32 = tf.values().sum();
        for t in tf.keys() {
            *df.entry(t.clone()).or_insert(0.0) += 1.0;
        }
        docs.push(Doc { idx, tf, len });
    }
    let n = docs.len().max(1) as f32;
    let avgdl = docs.iter().map(|d| d.len).sum::<f32>() / n;

    let mut scored: Vec<(usize, f32)> = vec![];
    for doc in &docs {
        if doc.len == 0.0 {
            continue;
        }
        let mut score = 0.0f32;
        for term in &q_set {
            let Some(&f) = doc.tf.get(term.as_str()) else {
                continue;
            };
            let n_q = df.get(term.as_str()).cloned().unwrap_or(0.0);
            if n_q == 0.0 {
                continue;
            }
            let idf = (((n - n_q + 0.5) / (n_q + 0.5)) + 1.0).ln();
            let denom = f + K1 * (1.0 - B + B * doc.len / avgdl);
            score += idf * (f * (K1 + 1.0)) / denom;
        }
        if score > 0.0 {
            scored.push((doc.idx, score));
        }
    }
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    if top_k > 0 {
        scored.truncate(top_k);
    }
    scored
        .into_iter()
        .map(|(idx, score)| {
            let s = &symbols[idx];
            RankedSymbol {
                path: s.path.clone(),
                name: s.name.clone(),
                kind: s.kind.clone(),
                line: s.line,
                score,
                lane: Lane::Conceptual,
            }
        })
        .collect()
}

/// Route then rank: the minimal `ranked_symbols` API over in-memory rows.
pub fn ranked_symbols(symbols: &[SymbolDoc], query: &str, top_k: usize) -> Vec<RankedSymbol> {
    if query.trim().is_empty() {
        return vec![];
    }
    let known: std::collections::HashSet<String> =
        symbols.iter().map(|s| s.name.to_lowercase()).collect();
    match rank_route(query, &known) {
        Lane::NameExact => rank_name_exact(symbols, query.trim(), top_k),
        Lane::Conceptual => rank_conceptual(symbols, query, top_k),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn docs() -> Vec<SymbolDoc> {
        vec![
            SymbolDoc {
                path: "tool/code_impact.rs".into(),
                name: "CodeImpactTool".into(),
                kind: "class".into(),
                line: 21,
            },
            SymbolDoc {
                path: "tool/code_impact.rs".into(),
                name: "advisory_blast_line".into(),
                kind: "function".into(),
                line: 49,
            },
            SymbolDoc {
                path: "tool/code_impact.rs".into(),
                name: "advisory_blast_line_async".into(),
                kind: "function".into(),
                line: 61,
            },
            SymbolDoc {
                path: "tool/code_impact.rs".into(),
                name: "ImpactReport".into(),
                kind: "class".into(),
                line: 41,
            },
            SymbolDoc {
                path: "tool/edit.rs".into(),
                name: "execute".into(),
                kind: "method".into(),
                line: 68,
            },
        ]
    }

    #[test]
    fn name_exact_rank_1() {
        for q in [
            "CodeImpactTool",
            "ImpactReport",
            "advisory_blast_line_async",
            "advisory_blast_line",
        ] {
            let r = ranked_symbols(&docs(), q, 5);
            assert!(!r.is_empty(), "empty for {q}");
            assert_eq!(r[0].name, q, "rank-1 for {q}: {:?}", r[0]);
            assert_eq!(r[0].lane, Lane::NameExact);
        }
    }

    #[test]
    fn name_exact_tiers_ordered() {
        let r = rank_name_exact(&docs(), "advisory_blast_line", 10);
        assert_eq!(r[0].name, "advisory_blast_line");
        assert_eq!(r[0].score, 3.0);
        // Prefix twin ranks below the exact hit.
        assert!(r.iter().any(|x| x.name == "advisory_blast_line_async"));
        assert_eq!(r[1].score, 1.0);
    }

    #[test]
    fn conceptual_lands_code_impact_first() {
        let r = ranked_symbols(&docs(), "blast radius dependents code impact", 5);
        assert!(!r.is_empty());
        assert_eq!(r[0].lane, Lane::Conceptual);
        assert!(
            r[0].path.contains("code_impact"),
            "top conceptual: {:?}",
            r[0]
        );
    }

    #[test]
    fn empty_and_unknown_return_empty() {
        assert!(ranked_symbols(&docs(), "", 5).is_empty());
        assert!(ranked_symbols(&docs(), "   ", 5).is_empty());
        assert!(ranked_symbols(&docs(), "zzzznothing", 5).is_empty());
    }

    #[test]
    fn route_disclosure_identifier_vs_prose() {
        let known: std::collections::HashSet<String> =
            docs().iter().map(|s| s.name.to_lowercase()).collect();
        assert_eq!(rank_route("CodeImpactTool", &known), Lane::NameExact);
        assert_eq!(
            rank_route("blast radius dependents", &known),
            Lane::Conceptual
        );
    }

    #[test]
    fn same_named_methods_distinguished_by_path_line() {
        let syms = vec![
            SymbolDoc {
                path: "a.rs".into(),
                name: "new".into(),
                kind: "method".into(),
                line: 3,
            },
            SymbolDoc {
                path: "b.rs".into(),
                name: "new".into(),
                kind: "method".into(),
                line: 7,
            },
        ];
        let r = ranked_symbols(&syms, "new", 10);
        assert_eq!(r.len(), 2);
        assert_ne!(
            (r[0].path.clone(), r[0].line),
            (r[1].path.clone(), r[1].line)
        );
    }
}
