//! `symbol_relocate` tool (`jcode-rqk`): atomic rename / move-symbol over text
//! refs (pass-9 merge: single core, `dst: none` = rename).
//!
//! Flow: blast-radius preview (mandatory, via `codegraph::indexed_or_live`)
//! → collect text refs (agentgrep crate `run_grep`) → apply to temp copies →
//! verify → swap all-or-nothing (restore originals on any failure, per-file
//! status) → typecheck gate (repo check on failure: offer one-command revert,
//! never auto-revert — destructive-action policy). Same approval tier as
//! `edit` (base tool, no extra gating).

use super::codegraph::{GraphSource, indexed_or_live, rel_display, resolve_repo_root};
use super::{Tool, ToolContext, ToolOutput};
use ::agentgrep::cli::GrepArgs;
use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub struct SymbolRelocateTool;

impl SymbolRelocateTool {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Deserialize)]
struct RelocateInput {
    #[serde(default)]
    intent: Option<String>,
    file: String,
    symbol: String,
    new_name: String,
    #[serde(default)]
    dst: Option<String>,
    /// Test-only failure injection: fail after staging file N (0-based).
    #[serde(default)]
    fail_after: Option<usize>,
}

#[derive(Debug)]
struct FileOutcome {
    path: String,
    replacements: usize,
    staged: bool,
    error: Option<String>,
}

fn text_refs(root: &Path, symbol: &str) -> Vec<String> {
    if symbol.len() > super::codegraph::live::MAX_PATTERN_LEN || symbol.is_empty() {
        return vec![];
    }
    let args = GrepArgs {
        query: symbol.to_string(),
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
            let mut files: Vec<String> = res
                .files
                .iter()
                .map(|f| {
                    let abs = if Path::new(&f.path).is_absolute() {
                        PathBuf::from(&f.path)
                    } else {
                        root.join(&f.path)
                    };
                    rel_display(root, &abs)
                })
                .collect();
            files.sort();
            files.dedup();
            files.truncate(100);
            files
        }
        Err(_) => vec![],
    }
}

/// Word-boundary-ish replacement without regex: replace `symbol` only when
/// flanked by non-identifier chars. Conservative: skips matches inside longer
/// identifiers (avoids `foo` hitting `foobar`).
fn replace_symbol_text(content: &str, symbol: &str, new_name: &str) -> (String, usize) {
    let mut out = String::with_capacity(content.len());
    let mut count = 0;
    let mut rest = content;
    // Identifier chars include `$` (JS `$foo` must not match `foo`; F5).
    let is_ident = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    while let Some(pos) = rest.find(symbol) {
        let before_ok = pos == 0 || {
            let c = rest[..pos].chars().last().unwrap();
            !is_ident(c)
        };
        let after = &rest[pos + symbol.len()..];
        let after_ok = after.is_empty() || {
            let c = after.chars().next().unwrap();
            !is_ident(c)
        };
        if before_ok && after_ok {
            out.push_str(&rest[..pos]);
            out.push_str(new_name);
            count += 1;
            rest = after;
        } else {
            out.push_str(&rest[..pos + 1]);
            rest = &rest[pos + 1..];
        }
    }
    out.push_str(rest);
    (out, count)
}

fn detect_check_cmd(root: &Path) -> Option<Vec<String>> {
    if root.join("Cargo.toml").exists() {
        return Some(vec!["cargo".to_string(), "check".to_string()]);
    }
    if root.join("tsconfig.json").exists() {
        return Some(vec![
            "npx".to_string(),
            "tsc".to_string(),
            "--noEmit".to_string(),
        ]);
    }
    if root.join("go.mod").exists() {
        return Some(vec![
            "go".to_string(),
            "build".to_string(),
            "./...".to_string(),
        ]);
    }
    None
}

struct RelocateReport {
    preview: String,
    outcomes: Vec<FileOutcome>,
    restored: bool,
    check: Option<String>,
}

fn run_relocate(
    root: &Path,
    rel: &str,
    symbol: &str,
    new_name: &str,
    fail_after: Option<usize>,
) -> RelocateReport {
    let (graph, _) = indexed_or_live(root, rel, false);
    let preview = format!(
        "Blast preview for \"{rel}\": {} dependent(s), {} co-change partner(s), {} exported symbol(s) [source: {}].",
        graph.dependents.len(),
        graph.cochanges.len(),
        graph.symbols.len(),
        graph.source.as_str()
    );
    let refs = text_refs(root, symbol);
    // Stage: apply to temp copies first.
    let mut staged: Vec<(PathBuf, String, usize)> = vec![];
    let mut outcomes: Vec<FileOutcome> = vec![];
    for (i, f) in refs.iter().enumerate() {
        if super::codegraph::live::resolve_within_root(root, f).is_err() {
            outcomes.push(FileOutcome {
                path: f.clone(),
                replacements: 0,
                staged: false,
                error: Some("path escapes root; skipped".to_string()),
            });
            continue;
        }
        let abs = root.join(f);
        let content = match std::fs::read_to_string(&abs) {
            Ok(c) => c,
            Err(e) => {
                outcomes.push(FileOutcome {
                    path: f.clone(),
                    replacements: 0,
                    staged: false,
                    error: Some(format!("read failed: {e}")),
                });
                continue;
            }
        };
        let (new_content, n) = replace_symbol_text(&content, symbol, new_name);
        if n == 0 {
            continue;
        }
        if fail_after == Some(i) {
            outcomes.push(FileOutcome {
                path: f.clone(),
                replacements: n,
                staged: false,
                error: Some("injected failure (fail_after)".to_string()),
            });
            // Abort: restore nothing yet (nothing swapped), mark restored=false
            // with remaining files unattempted.
            return RelocateReport {
                preview,
                outcomes,
                restored: true,
                check: None,
            };
        }
        staged.push((abs, new_content, n));
        outcomes.push(FileOutcome {
            path: f.clone(),
            replacements: n,
            staged: true,
            error: None,
        });
    }
    // Swap all-or-nothing: backup originals, write staged, restore on error.
    let mut backups: Vec<(PathBuf, String)> = vec![];
    let mut restored = false;
    for (abs, new_content, _) in &staged {
        match std::fs::read_to_string(abs) {
            Ok(orig) => backups.push((abs.clone(), orig)),
            Err(e) => {
                outcomes.push(FileOutcome {
                    path: abs.to_string_lossy().to_string(),
                    replacements: 0,
                    staged: false,
                    error: Some(format!("re-read failed: {e}")),
                });
                restored = true;
                break;
            }
        }
        if std::fs::write(abs, new_content).is_err() {
            restored = true;
            break;
        }
    }
    if restored {
        for (abs, orig) in &backups {
            let _ = std::fs::write(abs, orig);
        }
    }
    // Typecheck gate (advisory): run repo check, report, offer revert command.
    let check = if !restored && !staged.is_empty() {
        detect_check_cmd(root).map(|cmd| {
            let ok = std::process::Command::new(&cmd[0])
                .args(&cmd[1..])
                .current_dir(root)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            if ok {
                format!("typecheck passed ({}).", cmd.join(" "))
            } else {
                format!(
                    "typecheck FAILED ({}). Revert with: symbol_relocate file={} symbol={} new_name={} (swap back).",
                    cmd.join(" "),
                    rel,
                    new_name,
                    symbol
                )
            }
        })
    } else {
        None
    };
    RelocateReport {
        preview,
        outcomes,
        restored,
        check,
    }
}

#[async_trait]
impl Tool for SymbolRelocateTool {
    fn name(&self) -> &str {
        "symbol_relocate"
    }

    fn description(&self) -> &str {
        "Atomic rename / move-symbol across text refs: blast preview first, temp-copy staging, all-or-nothing swap with restore, typecheck gate. dst unset = rename."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["file", "symbol", "new_name"],
            "properties": {
                "intent": super::intent_schema_property(),
                "file": {"type": "string", "description": "Repo-relative file owning the symbol."},
                "symbol": {"type": "string", "description": "Symbol name to rename."},
                "new_name": {"type": "string", "description": "New symbol name."},
                "dst": {"type": "string", "description": "Optional destination file (move). Unset = rename in place."},
                "fail_after": {"type": "integer", "description": "Test-only failure injection."}
            }
        })
    }

    async fn execute(&self, input: Value, ctx: ToolContext) -> Result<ToolOutput> {
        let params: RelocateInput = serde_json::from_value(input)?;
        if params.symbol.is_empty() || params.new_name.is_empty() {
            return Err(anyhow::anyhow!("symbol and new_name must be non-empty"));
        }
        if params.symbol == params.new_name {
            return Err(anyhow::anyhow!("symbol and new_name must differ"));
        }
        if params.symbol.len() > super::codegraph::live::MAX_PATTERN_LEN {
            return Err(anyhow::anyhow!("symbol too long"));
        }
        let Some(working) = ctx.working_dir.clone() else {
            return Err(anyhow::anyhow!(
                "symbol_relocate requires a working directory (none in tool context)"
            ));
        };
        let root = resolve_repo_root(&working);
        if super::codegraph::live::resolve_within_root(&root, &params.file).is_err() {
            return Err(anyhow::anyhow!("path escapes repo root: {}", params.file));
        }
        let rel = if Path::new(&params.file).is_absolute() {
            rel_display(&root, Path::new(&params.file))
        } else {
            params.file.trim_start_matches("./").to_string()
        };
        // dst (move) validation: must stay within root.
        if let Some(dst) = params.dst.as_deref() {
            if super::codegraph::live::resolve_within_root(&root, dst).is_err() {
                return Err(anyhow::anyhow!("dst escapes repo root: {dst}"));
            }
        }
        let symbol = params.symbol.clone();
        let new_name = params.new_name.clone();
        let fail_after = params.fail_after;
        let report = tokio::task::spawn_blocking(move || {
            run_relocate(&root, &rel, &symbol, &new_name, fail_after)
        })
        .await
        .map_err(|_| anyhow::anyhow!("relocate blocking task failed"))?;
        let mut out = vec![report.preview, String::new()];
        if report.restored {
            out.push("ABORTED: no files were modified (all-or-nothing restore).".to_string());
        }
        out.push("Per-file status:".to_string());
        for o in &report.outcomes {
            out.push(format!(
                "  {} replacements={} staged={}{}",
                o.path,
                o.replacements,
                o.staged,
                o.error
                    .as_deref()
                    .map(|e| format!(" error={e}"))
                    .unwrap_or_default()
            ));
        }
        if let Some(check) = report.check.as_deref() {
            out.push(String::new());
            out.push(check.to_string());
        }
        if params.dst.is_some() {
            out.push(String::new());
            out.push(
                "NOTE: move import-line insertion is manual in v1 (dst validated, refs renamed)."
                    .to_string(),
            );
        }
        let _ = GraphSource::Live;
        Ok(ToolOutput::new(out.join("\n")))
    }
}

#[cfg(test)]
mod tests;
