//! `code_query` tool (`jcode-hg3`): composable code-exploration pipeline in one
//! round trip — search, find, filter, deps, outline, read, limit.
//!
//! Stages call shared blocking cores directly (agentgrep crate fns +
//! `codegraph::live` fns), never `Tool::execute` (plan 3.1: no nested runtime
//! inside `spawn_blocking`). Zero file I/O until an explicit `read` stage.
//! Caps: ≤12 ops, ≤200 files working set, outline ≤40 files, read ≤10 files.

use super::codegraph::live::{
    dependencies_live, dependents_live, rel_display, resolve_repo_root, resolve_within_root,
};
use super::{Tool, ToolContext, ToolOutput};
use ::agentgrep::cli::{FindArgs, GrepArgs, OutlineArgs};
use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub const MAX_FILES: usize = 200;
pub const MAX_PIPELINE_OPS: usize = 12;
pub const OUTLINE_FILE_CAP: usize = 40;
pub const READ_FILE_CAP: usize = 10;

pub struct CodeQueryTool;

impl CodeQueryTool {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Deserialize, Debug)]
struct QueryOp {
    op: String,
    #[serde(default)]
    pattern: Option<String>,
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    ext: Option<String>,
    #[serde(default, rename = "pathContains")]
    path_contains: Option<String>,
    #[serde(default)]
    direction: Option<String>,
    #[serde(default)]
    ranges: Option<ReadRanges>,
    #[serde(default)]
    n: Option<usize>,
}

#[derive(Deserialize, Debug, Clone)]
struct ReadRanges {
    start: usize,
    end: usize,
}

#[derive(Deserialize)]
#[allow(dead_code)] // `intent` is schema-required but consumed by the harness, not the tool
struct CodeQueryInput {
    #[serde(default)]
    intent: Option<String>,
    pipeline: Vec<QueryOp>,
}

struct PipeState {
    files: Vec<String>,
    blocks: Vec<String>,
}

fn to_rel(root: &Path, p: &str) -> String {
    let abs = if Path::new(p).is_absolute() {
        PathBuf::from(p)
    } else {
        root.join(p)
    };
    rel_display(root, &abs)
}

fn dedupe(files: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = vec![];
    for f in files {
        if seen.insert(f.clone()) {
            out.push(f);
        }
        if out.len() >= MAX_FILES {
            break;
        }
    }
    out
}

fn describe(op: &QueryOp) -> String {
    match op.op.as_str() {
        "search" => format!("search {:?}", op.pattern.as_deref().unwrap_or("")),
        "find" => format!("find {:?}", op.query.as_deref().unwrap_or("")),
        "filter" => format!(
            "filter {} {}",
            op.ext.as_deref().unwrap_or(""),
            op.path_contains.as_deref().unwrap_or("")
        )
        .trim()
        .to_string(),
        "deps" => format!("deps {}", op.direction.as_deref().unwrap_or("imports")),
        "outline" => "outline".to_string(),
        "read" => "read".to_string(),
        "limit" => format!("limit {}", op.n.unwrap_or(0)),
        other => format!("unknown:{other}"),
    }
}

fn run_stage(op: &QueryOp, state: &mut PipeState, root: &Path) -> Result<(), String> {
    match op.op.as_str() {
        "search" => {
            let pattern = op
                .pattern
                .as_deref()
                .ok_or_else(|| "search needs {pattern}".to_string())?;
            if pattern.len() > super::codegraph::live::MAX_PATTERN_LEN {
                return Err("pattern too long (max 200 chars)".to_string());
            }
            let args = GrepArgs {
                query: pattern.to_string(),
                regex: false,
                file_type: None,
                json: false,
                paths_only: true,
                hidden: false,
                no_ignore: false,
                path: None,
                glob: None,
            };
            match ::agentgrep::search::run_grep(root, &args) {
                Ok(res) => {
                    state.files = dedupe(res.files.iter().map(|f| to_rel(root, &f.path)).collect());
                    Ok(())
                }
                Err(e) => Err(format!("search failed: {e}")),
            }
        }
        "find" => {
            let query = op
                .query
                .as_deref()
                .ok_or_else(|| "find needs {query}".to_string())?;
            let args = FindArgs {
                query_parts: vec![query.to_string()],
                file_type: None,
                json: true,
                paths_only: true,
                debug_score: false,
                max_files: MAX_FILES,
                hidden: false,
                no_ignore: false,
                path: None,
                glob: None,
            };
            {
                let res = ::agentgrep::find::run_find(root, &args);
                state.files = dedupe(res.files.iter().map(|f| to_rel(root, &f.path)).collect());
                Ok(())
            }
        }
        "filter" => {
            state.files = state
                .files
                .iter()
                .filter(|f| {
                    if let Some(ext) = op.ext.as_deref() {
                        if !f.ends_with(ext) {
                            return false;
                        }
                    }
                    if let Some(pc) = op.path_contains.as_deref() {
                        if !f.contains(pc) {
                            return false;
                        }
                    }
                    true
                })
                .cloned()
                .collect();
            Ok(())
        }
        "deps" => {
            let dir = op.direction.as_deref().unwrap_or("imports");
            if dir != "imports" && dir != "imported_by" {
                return Err("deps direction must be imports|imported_by".to_string());
            }
            let mut next = HashSet::new();
            for f in state.files.iter().take(MAX_FILES) {
                let (rows, _, _) = if dir == "imported_by" {
                    dependents_live(root, f)
                } else {
                    dependencies_live(root, f)
                };
                for r in rows {
                    // Skip external: pseudo-paths in deps results.
                    if !r.path.starts_with("external:") {
                        next.insert(to_rel(root, &r.path));
                    }
                }
            }
            let mut v: Vec<String> = next.into_iter().collect();
            v.sort();
            state.files = dedupe(v);
            Ok(())
        }
        "outline" => {
            for f in state.files.iter().take(OUTLINE_FILE_CAP) {
                if resolve_within_root(root, f).is_err() {
                    continue;
                }
                let args = OutlineArgs {
                    file: f.clone(),
                    json: true,
                    max_items: Some(60),
                    path: None,
                    context_json: None,
                };
                match ::agentgrep::outline::run_outline(root, &args) {
                    Ok(res) => {
                        let items: Vec<String> = res
                            .structure
                            .items
                            .iter()
                            .take(60)
                            .map(|s| {
                                format!("  {}-{} {} {}", s.start_line, s.end_line, s.kind, s.label)
                            })
                            .collect();
                        if !items.is_empty() {
                            state.blocks.push(format!("{f}:\n{}", items.join("\n")));
                        }
                    }
                    Err(_) => continue,
                }
            }
            Ok(())
        }
        "read" => {
            for f in state.files.iter().take(READ_FILE_CAP) {
                let abs = match resolve_within_root(root, f) {
                    Ok(p) => p,
                    Err(_) => continue,
                };
                if super::codegraph::live::is_binary_file(&abs) {
                    continue;
                }
                match std::fs::read_to_string(&abs) {
                    Ok(content) => {
                        let text = match &op.ranges {
                            Some(r) => {
                                let lines: Vec<&str> = content.lines().collect();
                                let s = r.start.saturating_sub(1).min(lines.len());
                                let e = r.end.min(lines.len()).max(s);
                                lines[s..e].join("\n")
                            }
                            None => content.lines().take(200).collect::<Vec<_>>().join("\n"),
                        };
                        state.blocks.push(format!("── {f} ──\n{text}"));
                    }
                    Err(_) => continue,
                }
            }
            Ok(())
        }
        "limit" => {
            let n = op.n.unwrap_or(MAX_FILES);
            state.files.truncate(n.min(MAX_FILES));
            Ok(())
        }
        other => Err(format!("unknown op: {other}")),
    }
}

#[async_trait]
impl Tool for CodeQueryTool {
    fn name(&self) -> &str {
        "code_query"
    }

    fn description(&self) -> &str {
        "Composable code-exploration pipeline in one call: search, find, filter, deps, outline, read, limit. Each stage narrows a file set and feeds the next."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["pipeline"],
            "properties": {
                "intent": super::intent_schema_property(),
                "pipeline": {
                    "type": "array",
                    "maxItems": MAX_PIPELINE_OPS,
                    "items": {
                        "type": "object",
                        "required": ["op"],
                        "properties": {
                            "op": {"type": "string", "enum": ["search", "find", "filter", "deps", "outline", "read", "limit"]},
                            "pattern": {"type": "string"},
                            "query": {"type": "string"},
                            "ext": {"type": "string"},
                            "pathContains": {"type": "string"},
                            "direction": {"type": "string", "enum": ["imports", "imported_by"]},
                            "ranges": {"type": "object", "properties": {"start": {"type": "integer"}, "end": {"type": "integer"}}},
                            "n": {"type": "integer"}
                        }
                    }
                }
            }
        })
    }

    async fn execute(&self, input: Value, ctx: ToolContext) -> Result<ToolOutput> {
        let params: CodeQueryInput = serde_json::from_value(input)?;
        if params.pipeline.is_empty() {
            return Err(anyhow::anyhow!(
                "Empty pipeline. Provide pipeline:[op:...]."
            ));
        }
        if params.pipeline.len() > MAX_PIPELINE_OPS {
            return Err(anyhow::anyhow!(
                "Pipeline too long ({} ops, max {}).",
                params.pipeline.len(),
                MAX_PIPELINE_OPS
            ));
        }
        let Some(working) = ctx.working_dir.clone() else {
            return Err(anyhow::anyhow!(
                "code_query requires a working directory (none in tool context)"
            ));
        };
        let root = resolve_repo_root(&working);
        let pipeline = params.pipeline;
        let output = tokio::task::spawn_blocking(move || {
            let mut state = PipeState {
                files: vec![],
                blocks: vec![],
            };
            let mut trace = vec![];
            for (i, stage) in pipeline.iter().enumerate() {
                match run_stage(stage, &mut state, &root) {
                    Ok(()) => trace.push(format!(
                        "{}. {} → {} files",
                        i + 1,
                        describe(stage),
                        state.files.len()
                    )),
                    Err(e) => {
                        return format!("Stage {} ({}) failed: {e}", i + 1, stage.op);
                    }
                }
            }
            let mut parts = vec![format!("Pipeline ({} stages):", pipeline.len())];
            parts.extend(trace.iter().map(|t| format!("  {t}")));
            if !state.blocks.is_empty() {
                parts.push(String::new());
                parts.extend(state.blocks);
            } else if !state.files.is_empty() {
                parts.push(String::new());
                parts.push("Files:".to_string());
                parts.extend(state.files.iter().take(MAX_FILES).map(|f| format!("  {f}")));
            } else {
                parts.push(String::new());
                parts.push("(no files matched)".to_string());
            }
            parts.join("\n")
        })
        .await
        .unwrap_or_else(|_| "Pipeline failed: blocking task error.".to_string());
        Ok(ToolOutput::new(output))
    }
}

#[cfg(test)]
mod tests;
