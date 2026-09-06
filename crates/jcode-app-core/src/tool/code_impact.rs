//! `code_impact` tool (`jcode-nd5`): dependents / dependencies / co-changes /
//! blast radius over the live graph core, plus the advisory blast-radius line
//! appended to `edit`/`multiedit` results.
//!
//! Thin async wrapper: resolves root from `ctx.working_dir`, calls blocking
//! `codegraph::live` fns inside `spawn_blocking`, formats Empryo
//! `soul_impact`-shaped text plus a `source` field. Advisory only — never
//! blocks or allows edits (approval policy unchanged).

use super::codegraph::live::{
    GraphSource, LiveGraph, cochanges_live, dependents_live, rel_display, resolve_repo_root,
    resolve_within_root,
};
use super::{Tool, ToolContext, ToolOutput};
use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::Path;

pub struct CodeImpactTool;

impl CodeImpactTool {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Deserialize)]
#[allow(dead_code)] // `intent` is schema-required but consumed by the harness, not the tool
struct CodeImpactInput {
    #[serde(default)]
    intent: Option<String>,
    action: String,
    file: String,
    #[serde(default)]
    refresh: bool,
}

#[derive(Debug, Clone)]
pub struct ImpactReport {
    pub source: GraphSource,
    pub partial: bool,
    pub graph: LiveGraph,
    pub rel: String,
}

#[allow(dead_code)] // production callers use the async variant; tests use this
pub fn advisory_blast_line(ctx: &ToolContext, abs_path: &Path) -> Option<String> {
    // Sync wrapper for tests and non-async callers. Async edit paths should
    // prefer `advisory_blast_line_async` (F3: two 10s-bounded blocking calls
    // must not run on a tokio worker).
    let root = ctx.working_dir.as_deref()?.to_path_buf();
    if !root.is_dir() {
        return None;
    }
    advisory_blast_line_for(&root, abs_path)
}

/// Async advisory: blocking graph calls inside `spawn_blocking`.
pub async fn advisory_blast_line_async(ctx: &ToolContext, abs_path: &Path) -> Option<String> {
    let root = ctx.working_dir.as_deref()?.to_path_buf();
    if !root.is_dir() {
        return None;
    }
    let abs = abs_path.to_path_buf();
    tokio::task::spawn_blocking(move || advisory_blast_line_for(&root, &abs))
        .await
        .ok()
        .flatten()
}

fn advisory_blast_line_for(root: &std::path::PathBuf, abs_path: &Path) -> Option<String> {
    let root = resolve_repo_root(root);
    let rel = rel_display(&root, abs_path);
    // Best-effort, bounded: dependents only, no git. Never fail the edit.
    let (dents, _, _) = dependents_live(&root, &rel);
    let (co, _, _) = cochanges_live(&root, &rel);
    let affected: std::collections::HashSet<&str> = dents
        .iter()
        .map(|d| d.path.as_str())
        .chain(co.iter().map(|c| c.path.as_str()))
        .collect();
    if dents.is_empty() && co.is_empty() {
        return None;
    }
    Some(format!(
        "[codegraph] blast radius for {rel}: {} dependent(s), {} co-change partner(s), {} file(s) affected.",
        dents.len(),
        co.len(),
        affected.len()
    ))
}

async fn compute_report(root: std::path::PathBuf, rel: String, refresh: bool) -> ImpactReport {
    let rel_fallback = rel.clone();
    tokio::task::spawn_blocking(move || {
        // Indexed-or-live (jcode-ggw): same shapes, plus source: index.
        // refresh=true forces reindex (F2: flag was dead before threading).
        let root = resolve_repo_root(&root);
        let (graph, _used_index) = super::codegraph::indexed_or_live(&root, &rel, refresh);
        ImpactReport {
            source: graph.source,
            partial: graph.partial,
            graph,
            rel,
        }
    })
    .await
    .unwrap_or(ImpactReport {
        source: GraphSource::Degraded,
        partial: true,
        graph: LiveGraph {
            source: GraphSource::Degraded,
            partial: true,
            dependents: vec![],
            dependencies: vec![],
            cochanges: vec![],
            symbols: vec![],
            notes: vec!["graph computation failed; degraded".to_string()],
        },
        rel: rel_fallback,
    })
}

fn render(report: &ImpactReport, action: &str) -> String {
    let g = &report.graph;
    let mut out = String::new();
    let header = |t: &str| {
        format!(
            "{t} (source: {},{})",
            report.source.as_str(),
            if report.partial { " partial" } else { "" }
        )
    };
    match action {
        "dependents" => {
            if g.dependents.is_empty() {
                out.push_str(&format!(
                    "No files depend on \"{}\" (or file not indexed). [{}]",
                    report.rel,
                    header("dependents")
                ));
            } else {
                out.push_str(&format!(
                    "{} files import from \"{}\":\n{}",
                    g.dependents.len(),
                    report.rel,
                    g.dependents
                        .iter()
                        .map(|d| format!("  {} (w:{})", d.path, d.weight.round()))
                        .collect::<Vec<_>>()
                        .join("\n")
                ));
            }
        }
        "dependencies" => {
            if g.dependencies.is_empty() {
                out.push_str(&format!(
                    "\"{}\" has no tracked dependencies (or file not indexed). [{}]",
                    report.rel,
                    header("dependencies")
                ));
            } else {
                out.push_str(&format!(
                    "\"{}\" imports from {} files:\n{}",
                    report.rel,
                    g.dependencies.len(),
                    g.dependencies
                        .iter()
                        .map(|d| format!("  {} (w:{})", d.path, d.weight.round()))
                        .collect::<Vec<_>>()
                        .join("\n")
                ));
            }
        }
        "cochanges" => {
            if g.cochanges.is_empty() {
                out.push_str(&format!(
                    "No co-change partners found for \"{}\". [{}]",
                    report.rel,
                    header("cochanges")
                ));
            } else {
                out.push_str(&format!(
                    "Files that historically change together with \"{}\":\n{}",
                    report.rel,
                    g.cochanges
                        .iter()
                        .map(|c| format!("  {} ({} co-commits)", c.path, c.weight.round()))
                        .collect::<Vec<_>>()
                        .join("\n")
                ));
            }
        }
        _ => {
            // blast_radius (default)
            if g.dependents.is_empty() && g.cochanges.is_empty() && g.symbols.is_empty() {
                out.push_str(&format!(
                    "\"{}\" not found in graph. [{}]",
                    report.rel,
                    header("blast_radius")
                ));
            } else {
                let mut affected = std::collections::HashSet::new();
                for d in &g.dependents {
                    affected.insert(d.path.as_str());
                }
                for c in &g.cochanges {
                    affected.insert(c.path.as_str());
                }
                out.push_str(&format!(
                    "Blast radius for \"{}\":\n  Direct dependents: {}\n  Co-change partners: {}\n  Total affected files: {} [{}]",
                    report.rel,
                    g.dependents.len(),
                    g.cochanges.len(),
                    affected.len(),
                    header("blast_radius")
                ));
                if !g.symbols.is_empty() {
                    out.push_str(&format!(
                        "\n\nExported symbols ({}):\n{}",
                        g.symbols.len(),
                        g.symbols
                            .iter()
                            .map(|s| format!("  {} {}", s.kind, s.name))
                            .collect::<Vec<_>>()
                            .join("\n")
                    ));
                }
                if !g.dependents.is_empty() {
                    out.push_str(&format!(
                        "\n\nDirect dependents ({}):\n{}",
                        g.dependents.len(),
                        g.dependents
                            .iter()
                            .take(20)
                            .map(|d| format!("  {} (w:{})", d.path, d.weight.round()))
                            .collect::<Vec<_>>()
                            .join("\n")
                    ));
                    if g.dependents.len() > 20 {
                        out.push_str(&format!("\n  ... and {} more", g.dependents.len() - 20));
                    }
                }
                let co_only: Vec<_> = g
                    .cochanges
                    .iter()
                    .filter(|c| !g.dependents.iter().any(|d| d.path == c.path))
                    .collect();
                if !co_only.is_empty() {
                    out.push_str("\n\nCo-change only (related by git history, not imports):\n");
                    out.push_str(
                        &co_only
                            .iter()
                            .take(10)
                            .map(|c| format!("  {} ({} co-commits)", c.path, c.weight.round()))
                            .collect::<Vec<_>>()
                            .join("\n"),
                    );
                }
            }
        }
    }
    out
}

#[async_trait]
impl Tool for CodeImpactTool {
    fn name(&self) -> &str {
        "code_impact"
    }

    fn description(&self) -> &str {
        "Check before editing high-impact files. Queries: dependents, dependencies, cochanges, blast_radius. Use when a file may be widely imported."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["action", "file"],
            "properties": {
                "intent": super::intent_schema_property(),
                "action": {
                    "type": "string",
                    "enum": ["dependents", "dependencies", "cochanges", "blast_radius"],
                    "description": "Query type."
                },
                "file": {
                    "type": "string",
                    "description": "Repo-relative or absolute file path."
                },
                "refresh": {
                    "type": "boolean",
                    "description": "Bypass the dep cache (default false)."
                }
            }
        })
    }

    async fn execute(&self, input: Value, ctx: ToolContext) -> Result<ToolOutput> {
        let params: CodeImpactInput = serde_json::from_value(input)?;
        let action = params.action.as_str();
        if !["dependents", "dependencies", "cochanges", "blast_radius"].contains(&action) {
            return Err(anyhow::anyhow!("Unknown action: {}", params.action));
        }
        let Some(working) = ctx.working_dir.clone() else {
            return Err(anyhow::anyhow!(
                "code_impact requires a working directory (none in tool context)"
            ));
        };
        let root = resolve_repo_root(&working);
        // Path traversal guard (plan security lens): reject escapes.
        if resolve_within_root(&root, &params.file).is_err() {
            // Allow absolute paths inside root too.
            let abs = Path::new(&params.file);
            let inside = if abs.is_absolute() {
                abs.starts_with(root.canonicalize().unwrap_or_else(|_| root.clone()))
            } else {
                false
            };
            if !inside {
                return Err(anyhow::anyhow!("path escapes repo root: {}", params.file));
            }
        }
        let rel = if Path::new(&params.file).is_absolute() {
            rel_display(&root, Path::new(&params.file))
        } else {
            params.file.trim_start_matches("./").to_string()
        };
        if params.refresh {
            super::codegraph::DepCache::shared().clear_for_test();
        }
        let report = compute_report(root, rel, params.refresh).await;
        Ok(ToolOutput::new(render(&report, action)))
    }
}

#[cfg(test)]
mod tests;
