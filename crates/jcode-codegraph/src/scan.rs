//! Repo scan: ignore-respecting walk, language detect, import→file resolution.
//!
//! Exclusions are a named fixture (plan pass-2): generated dirs never enter
//! the graph or PageRank. Symlinks that escape the root are skipped; loops
//! guarded by a visited-inode set (dev, ino on unix).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Directories/files never scanned (vendored output, lockfiles, VCS).
pub const DEFAULT_EXCLUDED_DIRS: &[&str] = &[
    "node_modules",
    ".git",
    "target",
    "dist",
    "build",
    ".next",
    ".nuxt",
    "venv",
    ".venv",
    "__pycache__",
    "coverage",
    ".cache",
];

pub const DEFAULT_EXCLUDED_EXTS: &[&str] = &[
    "lock", "lockb", "png", "jpg", "jpeg", "gif", "zip", "tar", "gz", "exe", "dll", "so", "dylib",
    "o", "a", "class", "pyc", "wasm", "mp3", "mp4",
];

/// Max files per scan (plan R2/perf budgets).
pub const SCAN_FILE_CAP: usize = 50_000;

#[derive(Debug, Clone)]
pub struct ScanExclusions {
    pub dirs: HashSet<String>,
    pub exts: HashSet<String>,
}

impl Default for ScanExclusions {
    fn default() -> Self {
        Self {
            dirs: DEFAULT_EXCLUDED_DIRS
                .iter()
                .map(|s| s.to_string())
                .collect(),
            exts: DEFAULT_EXCLUDED_EXTS
                .iter()
                .map(|s| s.to_string())
                .collect(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScannedFile {
    /// Repo-relative forward-slash path.
    pub rel: String,
    pub abs: PathBuf,
    pub language: &'static str,
    /// Seconds since epoch.
    pub mtime: i64,
}

/// Walk `root`, returning scannable files. Canonicalizes root first (macOS
/// /tmp → /private/tmp); skips escaping symlinks and excluded dirs/exts.
pub fn scan_repo(root: &Path, exclusions: &ScanExclusions) -> Vec<ScannedFile> {
    let canon_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut out = vec![];
    let mut stack = vec![canon_root.clone()];
    let mut visited: HashSet<(u64, u64)> = HashSet::new();
    while let Some(dir) = stack.pop() {
        if out.len() >= SCAN_FILE_CAP {
            break;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if let Ok(md) = std::fs::metadata(&dir) {
                if !visited.insert((md.dev(), md.ino())) {
                    continue;
                }
            }
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
            // Symlink guard: resolve; skip loops and root escapes.
            let real = p.canonicalize().unwrap_or_else(|_| p.clone());
            if !real.starts_with(&canon_root) {
                continue;
            }
            if real.is_dir() {
                if exclusions.dirs.contains(&name) {
                    continue;
                }
                stack.push(real);
            } else if real.is_file() {
                let ext = p
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.to_lowercase())
                    .unwrap_or_default();
                if exclusions.exts.contains(&ext) {
                    continue;
                }
                let rel = real
                    .strip_prefix(&canon_root)
                    .map(|r| r.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_default();
                if rel.is_empty() {
                    continue;
                }
                let mtime = std::fs::metadata(&real)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                out.push(ScannedFile {
                    language: crate::symbols::detect_language(&rel),
                    rel,
                    abs: real,
                    mtime,
                });
                if out.len() >= SCAN_FILE_CAP {
                    break;
                }
            }
        }
    }
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    out
}

/// Resolve a raw import spec to a repo-relative path (same semantics as the
/// live core: relative specifiers resolved against the importing file's dir
/// with extension/index probes; bare specifiers unresolved → None).
pub fn resolve_import(root: &Path, from_rel: &str, spec: &str) -> Option<String> {
    let spec = spec
        .trim()
        .trim_matches(|c| c == '"' || c == '\'' || c == '`');
    if spec.is_empty() || spec.starts_with("node:") || spec.starts_with("bun:") {
        return None;
    }
    if !spec.starts_with('.') {
        return None;
    }
    let from_abs = root.join(from_rel);
    let dir = from_abs.parent()?;
    for cand in [
        dir.join(spec),
        dir.join(format!("{spec}.rs")),
        dir.join(format!("{spec}.ts")),
        dir.join(format!("{spec}.tsx")),
        dir.join(format!("{spec}.js")),
        dir.join(format!("{spec}.py")),
        dir.join(format!("{spec}.go")),
        dir.join(spec).join("mod.rs"),
        dir.join(spec).join("index.ts"),
        dir.join(spec).join("__init__.py"),
    ] {
        if cand.is_file() {
            let real = cand.canonicalize().unwrap_or(cand);
            let canon_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
            if let Ok(rel) = real.strip_prefix(&canon_root) {
                return Some(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_respects_exclusions() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("node_modules")).unwrap();
        std::fs::write(root.join("node_modules").join("x.js"), "x").unwrap();
        std::fs::write(root.join("a.rs"), "fn a() {}").unwrap();
        std::fs::write(root.join("Cargo.lock"), "lock").unwrap();
        let files = scan_repo(root, &ScanExclusions::default());
        let rels: Vec<_> = files.iter().map(|f| f.rel.as_str()).collect();
        assert!(rels.contains(&"a.rs"), "rels={rels:?}");
        assert!(!rels.iter().any(|r| r.contains("node_modules")));
        assert!(!rels.contains(&"Cargo.lock"));
    }

    #[test]
    fn resolve_import_relative() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("util.ts"), "export {}").unwrap();
        std::fs::write(root.join("main.ts"), "import './util'").unwrap();
        assert_eq!(
            resolve_import(root, "main.ts", "./util"),
            Some("util.ts".to_string())
        );
        assert_eq!(resolve_import(root, "main.ts", "node:fs"), None);
    }
}
