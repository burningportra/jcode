//! Live graph queries: dependents, dependencies, co-changes, symbols.
//!
//! All functions are blocking and take an explicit repo `root` — they never
//! touch process CWD (the daemon serves many repos). Timeouts bound every
//! subprocess call; partial results carry `partial: true` rather than failing.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

/// How long each live sub-call may run before its results go partial.
pub const LIVE_SUBCALL_TIMEOUT: Duration = Duration::from_secs(10);
/// Cap on dependents / co-change partners returned per query.
pub const LIVE_RESULT_CAP: usize = 100;
/// Cap on `git log` commits mined for co-change partners.
pub const COCHANGE_COMMIT_CAP: usize = 500;
/// Max regex pattern length accepted (ReDoS guard; plan R7/security lens).
pub const MAX_PATTERN_LEN: usize = 200;

/// Which backend produced a result. Identical shapes across all three.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphSource {
    /// SQLite index (Phase 3; not produced by this module).
    Index,
    /// Live `rg` + `git` + import-regex computation.
    Live,
    /// Degraded: `rg` missing (internal walk fallback), non-git repo, timeout.
    Degraded,
}

impl GraphSource {
    pub fn as_str(self) -> &'static str {
        match self {
            GraphSource::Index => "index",
            GraphSource::Live => "live",
            GraphSource::Degraded => "degraded",
        }
    }
}

/// A file related to the queried file, with an optional weight/count.
#[derive(Debug, Clone, PartialEq)]
pub struct RelatedFile {
    /// Repo-relative path with forward slashes.
    pub path: String,
    /// Dependent weight (dependents/dependencies) or co-commit count.
    pub weight: f64,
}

/// A symbol exported by a file (regex-extracted; kind is advisory).
#[derive(Debug, Clone, PartialEq)]
pub struct ExportedSymbol {
    pub name: String,
    pub kind: String,
}

/// Aggregated query result with provenance.
#[derive(Debug, Clone)]
pub struct LiveGraph {
    pub source: GraphSource,
    /// True when a sub-call timed out or a fallback was used.
    pub partial: bool,
    pub dependents: Vec<RelatedFile>,
    pub dependencies: Vec<RelatedFile>,
    pub cochanges: Vec<RelatedFile>,
    pub symbols: Vec<ExportedSymbol>,
    /// Human-readable notes about degradation (empty when clean).
    pub notes: Vec<String>,
}

/// Resolve the repo root for a working dir: walk up to `.git`, but stop at
/// the first ancestor WITHOUT a `.git` marker chain break — i.e. only claim
/// a parent as root if the walk from `working_dir` reaches it through
/// directories that contain no other repo boundary. In practice: walk up
/// while each level is not itself a repo root's child escape; non-git repos
/// use the working dir itself (co-changes then degrade).
///
/// IMPORTANT: never claim a repo root ABOVE the canonicalized working dir's
/// own subtree unless a `.git` is found — and cap the walk: if the working
/// dir itself has no `.git` and its parent chain reaches `/` or `$HOME`
/// without one, return the working dir (prevents scratch dirs under `$HOME`
/// being swallowed by an unrelated ancestor repo).
pub fn resolve_repo_root(working_dir: &Path) -> PathBuf {
    // Fast path: the dir itself is a root.
    if working_dir.join(".git").exists() {
        return working_dir.to_path_buf();
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut dir = working_dir.parent();
    while let Some(d) = dir {
        // Stop at filesystem root or $HOME: never claim those as repo roots
        // for a working dir that has no .git chain leading there.
        if d.parent().is_none() {
            break;
        }
        if let Some(h) = home.as_deref() {
            if d == h {
                break;
            }
        }
        if d.join(".git").exists() {
            return d.to_path_buf();
        }
        dir = d.parent();
    }
    working_dir.to_path_buf()
}

/// Canonicalize `rel` against `root`, rejecting escapes (plan security lens:
/// symlinks resolved, prefix-checked, never string-compared).
pub fn resolve_within_root(root: &Path, rel: &str) -> Result<PathBuf, String> {
    // Canonicalize root FIRST (/tmp -> /private/tmp on macOS); otherwise the
    // prefix check below rejects every file under a symlinked root.
    let canon_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let joined = canon_root.join(rel);
    match joined.canonicalize() {
        Ok(canon) => {
            if canon.starts_with(&canon_root) {
                Ok(canon)
            } else {
                Err(format!("path escapes repo root: {rel}"))
            }
        }
        Err(_) => {
            // File may not exist yet (e.g. queried mid-edit); fall back to
            // lexical check on the joined path.
            let mut normal = PathBuf::new();
            for comp in joined.components() {
                use std::path::Component::*;
                match comp {
                    CurDir => {}
                    ParentDir => {
                        if !normal.pop() {
                            return Err(format!("path escapes repo root: {rel}"));
                        }
                    }
                    c => normal.push(c.as_os_str()),
                }
            }
            if normal.starts_with(&canon_root) {
                Ok(normal)
            } else {
                Err(format!("path escapes repo root: {rel}"))
            }
        }
    }
}

/// Repo-relative forward-slash path for display; falls back to lossy absolute.
pub fn rel_display(root: &Path, abs: &Path) -> String {
    abs.strip_prefix(root)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| abs.to_string_lossy().replace('\\', "/"))
}

/// Mirror of `read.rs::is_binary_file` (extension + null-byte sniff over the
/// first 8KB). Kept local so this module stays dependency-free.
pub fn is_binary_file(path: &Path) -> bool {
    if let Some(ext) = path.extension() {
        let ext = ext.to_string_lossy().to_lowercase();
        const BINARY_EXTS: &[&str] = &[
            "png", "jpg", "jpeg", "gif", "bmp", "ico", "webp", "zip", "tar", "gz", "bz2", "xz",
            "7z", "rar", "exe", "dll", "so", "dylib", "o", "a", "class", "pyc", "wasm", "mp3",
            "mp4", "avi", "mov", "mkv", "flac", "ogg", "wav",
        ];
        if BINARY_EXTS.contains(&ext.as_str()) {
            return true;
        }
    }
    if let Ok(mut file) = std::fs::File::open(path) {
        let mut buf = [0u8; 8192];
        if let Ok(n) = file.read(&mut buf) {
            if n > 0 {
                return buf[..n].iter().filter(|&&b| b == 0).count() > n / 10;
            }
        }
    }
    false
}

fn command_exists(bin: &str) -> bool {
    Command::new(bin)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn run_with_timeout(cmd: &mut Command, timeout: Duration) -> Option<std::process::Output> {
    let start = Instant::now();
    let mut child = cmd.stdout(std::process::Stdio::piped()).spawn().ok()?;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return child.wait_with_output().ok(),
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(_) => return None,
        }
    }
}

/// Files that likely import from `rel`: `rg -l -F` on the module stem, then
/// keep only files containing a plausible import line. Falls back to an
/// internal `regex`-crate walk when `rg` is missing (`source: degraded`).
pub fn dependents_live(root: &Path, rel: &str) -> (Vec<RelatedFile>, GraphSource, bool) {
    let abs = match resolve_within_root(root, rel) {
        Ok(p) => p,
        Err(_) => return (vec![], GraphSource::Live, false),
    };
    if is_binary_file(&abs) {
        return (vec![], GraphSource::Live, false);
    }
    // Module stem: strip extension, strip trailing /index (Empryo parity).
    let stem = rel
        .rsplit_once('.')
        .map(|(s, _)| s)
        .unwrap_or(rel)
        .strip_suffix("/index")
        .unwrap_or(rel.rsplit_once('.').map(|(s, _)| s).unwrap_or(rel));
    if stem.len() > MAX_PATTERN_LEN || stem.is_empty() {
        return (vec![], GraphSource::Degraded, true);
    }
    let use_rg = command_exists("rg");
    let candidates: Vec<String> = if use_rg {
        match run_with_timeout(
            Command::new("rg")
                .args([
                    "-l",
                    "-F",
                    "--glob=!node_modules",
                    "--glob=!.git",
                    "--glob=!target",
                    "--max-count=1",
                    stem,
                    ".",
                ])
                .current_dir(root),
            LIVE_SUBCALL_TIMEOUT,
        ) {
            Some(out) if out.status.success() => String::from_utf8_lossy(&out.stdout)
                .lines()
                .map(|l| l.trim_start_matches("./").to_string())
                .filter(|f| f != rel)
                .collect(),
            _ => return (vec![], GraphSource::Degraded, true),
        }
    } else {
        // Internal fallback: walk + literal substring (regex crate is a dep).
        match internal_literal_files(root, stem) {
            Some(files) => files,
            None => return (vec![], GraphSource::Degraded, true),
        }
    };
    let source = if use_rg {
        GraphSource::Live
    } else {
        GraphSource::Degraded
    };
    // Keep only files with a plausible import line mentioning the stem.
    let mut out = vec![];
    for f in candidates {
        if out.len() >= LIVE_RESULT_CAP {
            break;
        }
        let abs = root.join(&f);
        if is_binary_file(&abs) {
            continue;
        }
        if let Ok(content) = std::fs::read_to_string(&abs) {
            if content.lines().any(|l| {
                let t = l.trim_start();
                (t.starts_with("import ")
                    || t.starts_with("from ")
                    || t.starts_with("use ")
                    || t.contains("require(")
                    || t.starts_with("#include"))
                    && l.contains(stem.rsplit('/').next().unwrap_or(stem))
            }) {
                out.push(RelatedFile {
                    path: f,
                    weight: 1.0,
                });
            }
        }
    }
    let partial = !use_rg;
    (out, source, partial)
}

fn internal_literal_files(root: &Path, needle: &str) -> Option<Vec<String>> {
    let re = regex::Regex::new(&regex::escape(needle)).ok()?;
    let mut out = vec![];
    let mut stack = vec![root.to_path_buf()];
    let start = Instant::now();
    while let Some(dir) = stack.pop() {
        if start.elapsed() >= LIVE_SUBCALL_TIMEOUT || out.len() >= LIVE_RESULT_CAP * 2 {
            break;
        }
        let entries = std::fs::read_dir(&dir).ok()?;
        for e in entries.flatten() {
            let p = e.path();
            let name = p.file_name()?.to_string_lossy().to_string();
            if name == "node_modules" || name == ".git" || name == "target" {
                continue;
            }
            if p.is_dir() {
                stack.push(p);
            } else if p.is_file() && !is_binary_file(&p) {
                if let Ok(content) = std::fs::read_to_string(&p) {
                    if re.is_match(&content) {
                        let rel = rel_display(root, &p);
                        out.push(rel);
                    }
                }
            }
        }
    }
    Some(out)
}

/// Import regexes per language family (Phase 1: Rust, TS/JS, Python, Go;
/// rest fall back to the generic pattern).
fn import_patterns(language: &str) -> Vec<regex::Regex> {
    let pats: &[&str] = match language {
        "rust" => &[
            r"(?m)^\s*(?:pub\s+)?use\s+([^;]+);",
            r"(?m)^\s*mod\s+(\w+)\s*;",
        ],
        "ts" | "js" => &[
            r#"(?m)^\s*import\s+(?:[^'"]*from\s+)?['"]([^'"]+)['"]"#,
            r#"require\(\s*['"]([^'"]+)['"]\s*\)"#,
        ],
        "python" => &[r"(?m)^\s*(?:from\s+(\S+)\s+import|import\s+(.+))"],
        "go" => &[r#"(?m)^\s*(?:import\s+(?:\(\s*)?(?:"([^"]+)"|`([^`]+)`))"#],
        _ => &[
            r#"(?m)^\s*import\s+['"]?([^'"\s;]+)['"]?"#,
            r"(?m)^\\s*use\\s+([^;]+);",
        ],
    };
    pats.iter()
        .filter_map(|p| regex::Regex::new(p).ok())
        .collect()
}

pub fn detect_language(path: &str) -> &'static str {
    if path.ends_with(".rs") {
        "rust"
    } else if path.ends_with(".ts") || path.ends_with(".tsx") || path.ends_with(".mts") {
        "ts"
    } else if path.ends_with(".js") || path.ends_with(".jsx") || path.ends_with(".mjs") {
        "js"
    } else if path.ends_with(".py") {
        "python"
    } else if path.ends_with(".go") {
        "go"
    } else {
        "generic"
    }
}

/// Resolve a raw import specifier to a repo-relative path when possible.
/// Returns `(display, resolved: bool)`; unresolved bare imports surface as
/// `external:<spec>` so nothing is silently dropped (plan 5.1).
fn resolve_import(root: &Path, from_file: &str, spec: &str) -> (String, bool) {
    let spec = spec
        .trim()
        .trim_matches(|c| c == '"' || c == '\'' || c == '`');
    if spec.is_empty()
        || spec.starts_with("node:")
        || spec.starts_with("bun:")
        || ["std", "core", "alloc"].contains(&spec)
    {
        return (format!("external:{spec}"), false);
    }
    // Relative specifiers resolve against the importing file's dir.
    if spec.starts_with('.') {
        let from_abs = root.join(from_file);
        if let Some(dir) = from_abs.parent() {
            for cand in [
                dir.join(spec),
                dir.join(format!("{spec}.rs")),
                dir.join(format!("{spec}.ts")),
                dir.join(format!("{spec}.py")),
                dir.join(format!("{spec}.go")),
                dir.join(spec).join("mod.rs"),
                dir.join(spec).join("index.ts"),
                dir.join(spec).join("__init__.py"),
            ] {
                if cand.is_file() {
                    return (
                        rel_display(root, &cand.canonicalize().unwrap_or(cand)),
                        true,
                    );
                }
            }
        }
        return (format!("external:{spec}"), false);
    }
    // Bare specifiers: basename fallback search (first hit wins, capped).
    let base = spec
        .rsplit("::")
        .next()
        .unwrap_or(spec)
        .rsplit('.')
        .next()
        .unwrap_or(spec)
        .rsplit('/')
        .next()
        .unwrap_or(spec);
    if base.len() > MAX_PATTERN_LEN {
        return (format!("external:{spec}"), false);
    }
    (format!("external:{spec}"), false)
}

/// Direct imports of `rel`, resolved to repo paths where possible.
pub fn dependencies_live(root: &Path, rel: &str) -> (Vec<RelatedFile>, GraphSource, bool) {
    let abs = match resolve_within_root(root, rel) {
        Ok(p) => p,
        Err(_) => return (vec![], GraphSource::Live, false),
    };
    let content = match std::fs::read_to_string(&abs) {
        Ok(c) => c,
        Err(_) => return (vec![], GraphSource::Degraded, true),
    };
    let lang = detect_language(rel);
    let mut seen = HashSet::new();
    let mut out = vec![];
    for re in import_patterns(lang) {
        for cap in re.captures_iter(&content) {
            for i in 1..cap.len() {
                if let Some(m) = cap.get(i) {
                    let spec = m
                        .as_str()
                        .split_whitespace()
                        .next()
                        .unwrap_or("")
                        .trim_end_matches(',');
                    if spec.is_empty() || !seen.insert(spec.to_string()) {
                        continue;
                    }
                    let (display, resolved) = resolve_import(root, rel, spec);
                    out.push(RelatedFile {
                        path: display,
                        weight: if resolved { 1.0 } else { 0.0 },
                    });
                    if out.len() >= LIVE_RESULT_CAP {
                        return (out, GraphSource::Live, false);
                    }
                }
            }
        }
    }
    (out, GraphSource::Live, false)
}

/// Files that historically change with `rel` (`git log`, capped).
/// Non-git repos and git failures degrade (co-changes only, never whole-call).
pub fn cochanges_live(root: &Path, rel: &str) -> (Vec<RelatedFile>, GraphSource, bool) {
    if resolve_within_root(root, rel).is_err() {
        return (vec![], GraphSource::Live, false);
    }
    if !root.join(".git").exists() {
        return (vec![], GraphSource::Degraded, true);
    }
    // NOTE: no path filter — `git log -- <path>` narrows `--name-only` output
    // to the matching path, hiding partners. Parse full log blocks instead.
    let out = match run_with_timeout(
        Command::new("git")
            .args([
                "-C",
                &root.to_string_lossy().to_string(),
                "log",
                "--name-only",
                "--pretty=format:COMMIT:%H",
                &format!("--max-count={COCHANGE_COMMIT_CAP}"),
            ])
            .current_dir(root),
        LIVE_SUBCALL_TIMEOUT,
    ) {
        Some(o) if o.status.success() => o,
        _ => return (vec![], GraphSource::Degraded, true),
    };
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut block: Vec<String> = vec![];
    let mut flush = |block: &mut Vec<String>, counts: &mut HashMap<String, usize>| {
        if block.iter().any(|f| f == rel) {
            for f in block.iter() {
                if f != rel && !f.is_empty() {
                    *counts.entry(f.clone()).or_insert(0) += 1;
                }
            }
        }
        block.clear();
    };
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let f = line.trim();
        if f.starts_with("COMMIT:") {
            flush(&mut block, &mut counts);
        } else if !f.is_empty() {
            block.push(f.to_string());
        }
    }
    flush(&mut block, &mut counts);
    let mut v: Vec<RelatedFile> = counts
        .into_iter()
        .map(|(path, count)| RelatedFile {
            path,
            weight: count as f64,
        })
        .collect();
    v.sort_by(|a, b| b.weight.partial_cmp(&a.weight).unwrap());
    v.truncate(20);
    (v, GraphSource::Live, false)
}

/// Exported symbols via per-language regexes (advisory kinds, not AST).
pub fn exported_symbols_live(root: &Path, rel: &str) -> Vec<ExportedSymbol> {
    let abs = match resolve_within_root(root, rel) {
        Ok(p) => p,
        Err(_) => return vec![],
    };
    let content = match std::fs::read_to_string(&abs) {
        Ok(c) => c,
        Err(_) => return vec![],
    };
    let lang = detect_language(rel);
    let pats: &[(&str, &str)] = match lang {
        "rust" => &[
            (r"(?m)^\s*pub\s+fn\s+(\w+)", "function"),
            (r"(?m)^\s*pub\s+struct\s+(\w+)", "class"),
            (r"(?m)^\s*pub\s+enum\s+(\w+)", "enum"),
            (r"(?m)^\s*pub\s+trait\s+(\w+)", "interface"),
            (r"(?m)^\s*pub\s+(?:const|static)\s+(\w+)", "constant"),
            (r"(?m)^\s*pub\s+mod\s+(\w+)", "module"),
        ],
        "ts" | "js" => &[
            (
                r"(?m)^\s*export\s+(?:async\s+)?function\s+(\w+)",
                "function",
            ),
            (r"(?m)^\s*export\s+(?:default\s+)?class\s+(\w+)", "class"),
            (
                r"(?m)^\s*export\s+(?:default\s+)?interface\s+(\w+)",
                "interface",
            ),
            (r"(?m)^\s*export\s+type\s+(\w+)", "type"),
            (r"(?m)^\s*export\s+(?:const|let|var)\s+(\w+)", "variable"),
            (r"(?m)^\s*export\s+enum\s+(\w+)", "enum"),
        ],
        "python" => &[
            (r"(?m)^\s*def\s+(\w+)", "function"),
            (r"(?m)^\s*class\s+(\w+)", "class"),
        ],
        "go" => &[
            (
                r"(?m)^\s*func\s+(?:\(\w+\s+\*?\w+\)\s+)?([A-Z]\w*)",
                "function",
            ),
            (r"(?m)^\s*type\s+([A-Z]\w*)", "type"),
        ],
        _ => &[
            (r"(?m)^\s*(?:function\s+|fn\s+|def\s+)(\w+)", "function"),
            (r"(?m)^\s*(?:class|struct)\s+(\w+)", "class"),
        ],
    };
    let mut out = vec![];
    let mut seen = HashSet::new();
    for (pat, kind) in pats {
        let Ok(re) = regex::Regex::new(pat) else {
            continue;
        };
        for cap in re.captures_iter(&content) {
            if let Some(m) = cap.get(1) {
                let name = m.as_str().to_string();
                if seen.insert(name.clone()) {
                    out.push(ExportedSymbol {
                        name,
                        kind: kind.to_string(),
                    });
                }
            }
        }
    }
    // Python dunder/private filter: keep public names only for exports.
    if lang == "python" {
        out.retain(|s| !s.name.starts_with('_'));
    }
    out
}
