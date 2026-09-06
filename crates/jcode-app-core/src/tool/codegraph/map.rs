//! Prompt map block builder (`jcode-hkd`): top-30 files by dependent count as
//! `path (→N dependents)`, rank-ordered for prompt-cache stability, ≤2k tokens.
//!
//! Pure blocking helper over `codegraph::live`; the caller decides injection.
//! Off by default (flag `agents.codegraph_map`, env `JCODE_CODEGRAPH_MAP`).

use super::live::{dependents_live, resolve_repo_root};
use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

/// Hard token-ish budget: ~4 chars per token → 8k chars ≈ 2k tokens.
pub const MAP_CHAR_BUDGET: usize = 8000;
pub const MAP_FILE_CAP: usize = 30;

/// Build the map block for `working_dir`. Returns `None` when disabled,
/// when the dir is unusable, or when no dependents were found.
pub fn build_map_block(working_dir: &Path, enabled: bool) -> Option<String> {
    if !enabled || !working_dir.is_dir() {
        return None;
    }
    let root = resolve_repo_root(working_dir);
    // Collect candidate files: walk top two levels + git ls-files when fast.
    let files = candidate_files(&root);
    if files.is_empty() {
        return None;
    }
    let start = Instant::now();
    let budget = std::time::Duration::from_secs(5);
    let mut counts: HashMap<String, usize> = HashMap::new();
    for f in files.iter().take(500) {
        if start.elapsed() >= budget {
            break;
        }
        let (dents, _, _) = dependents_live(&root, f);
        if !dents.is_empty() {
            counts.insert(f.clone(), dents.len());
        }
    }
    if counts.is_empty() {
        return None;
    }
    let mut ranked: Vec<(String, usize)> = counts.into_iter().collect();
    // Rank-ordered (stable): count desc, path asc. Never recency — cache.
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    ranked.truncate(MAP_FILE_CAP);
    let mut lines = vec![
        "Code map (top files by dependents — consult code_impact before editing):".to_string(),
    ];
    let mut chars = lines[0].len();
    for (path, n) in ranked {
        let line = format!("  {path} (→{n})");
        if chars + line.len() + 1 > MAP_CHAR_BUDGET {
            break;
        }
        chars += line.len() + 1;
        lines.push(line);
    }
    if lines.len() < 2 {
        return None;
    }
    Some(lines.join("\n"))
}

fn candidate_files(root: &Path) -> Vec<String> {
    // Fast path: git ls-files (respects .gitignore, millisecond-scale).
    if root.join(".git").exists() {
        if let Ok(out) = std::process::Command::new("git")
            .args(["-C", &root.to_string_lossy().to_string(), "ls-files"])
            .output()
        {
            if out.status.success() {
                let files: Vec<String> = String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .map(|l| l.trim().to_string())
                    .filter(|l| {
                        !l.is_empty()
                            && [".rs", ".ts", ".tsx", ".js", ".jsx", ".py", ".go"]
                                .iter()
                                .any(|e| l.ends_with(e))
                    })
                    .take(500)
                    .collect();
                if !files.is_empty() {
                    return files;
                }
            }
        }
    }
    // Fallback: two-level walk.
    let mut out = vec![];
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        if depth > 2 || out.len() >= 500 {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            if name == "node_modules" || name == ".git" || name == "target" {
                continue;
            }
            if p.is_dir() {
                stack.push((p, depth + 1));
            } else if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
                if ["rs", "ts", "tsx", "js", "jsx", "py", "go"].contains(&ext) {
                    if let Ok(rel) = p.strip_prefix(root) {
                        out.push(rel.to_string_lossy().replace('\\', "/"));
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_returns_none() {
        assert!(build_map_block(Path::new("/tmp"), false).is_none());
    }

    #[test]
    fn missing_dir_returns_none() {
        assert!(build_map_block(Path::new("/nonexistent-xyz"), true).is_none());
    }

    #[test]
    fn map_block_ranks_and_caps() {
        let dir = std::env::temp_dir().join(format!(
            "jcode-map-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("core.ts"), "export function core() {}\n").unwrap();
        std::fs::write(
            dir.join("user.ts"),
            "import { core } from './core';\ncore();\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("user2.ts"),
            "import { core } from './core';\ncore();\n",
        )
        .unwrap();
        let block = build_map_block(&dir, true).unwrap();
        assert!(block.contains("core.ts (→2)"), "block={block}");
        assert!(block.len() <= MAP_CHAR_BUDGET + 256, "len={}", block.len());
    }
}
