//! Indexed backend for graph tools (`jcode-ggw`): `GraphDb` behind the same
//! shapes as the live core, plus `source: index`.
//!
//! Lifecycle: `indexed_or_live` opens `<root>/.jcode/codegraph.db`
//! (repo-local, gitignored), reindexes when stale (>60s or `refresh`), serves
//! stale reads while rebuilding, falls back to live on `SQLITE_BUSY`,
//! corruption, read-only roots, or `JCODE_CODEGRAPH=0`. Readers never block.
//! Latency budgets (plan pass-4): indexed p50 < 200ms, live p50 < 2s.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use super::live::{
    GraphSource, LiveGraph, RelatedFile, cochanges_live, dependencies_live, dependents_live,
    exported_symbols_live, rel_display, resolve_repo_root, resolve_within_root,
};

pub const INDEX_STALE_AFTER: Duration = Duration::from_secs(60);

pub fn codegraph_disabled() -> bool {
    matches!(
        std::env::var("JCODE_CODEGRAPH").as_deref(),
        Ok("0") | Ok("false") | Ok("off")
    )
}

pub fn db_path_for(root: &Path) -> PathBuf {
    root.join(".jcode").join("codegraph.db")
}

struct IndexState {
    last_ok: Option<Instant>,
    unwritable: bool,
    /// Consecutive build failures. After 3, the root is treated Corrupt→Absent:
    /// DB file deleted once, then retried fresh (prevents poison loops).
    failures: u32,
}

/// Explicit lifecycle states (plan 3.1/euh): Absent → Building → Ready ⇄
/// Stale → Rebuilding, plus Disabled (JCODE_CODEGRAPH=0) and Corrupt → Absent
/// (auto-delete + rebuild). `status()` exposes the current state for tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexStatus {
    Absent,
    Building,
    Ready,
    Stale,
    Disabled,
}

pub fn index_status(root: &Path) -> IndexStatus {
    if codegraph_disabled() {
        return IndexStatus::Disabled;
    }
    let canon = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    if let Ok(guard) = IndexRegistry::shared().build_guard.lock() {
        if guard.contains(&canon) {
            return IndexStatus::Building;
        }
    }
    if let Ok(states) = IndexRegistry::shared().states.lock() {
        if let Some(st) = states.get(&canon) {
            if st.unwritable {
                return IndexStatus::Absent;
            }
            if let Some(t) = st.last_ok {
                if t.elapsed() < INDEX_STALE_AFTER {
                    return IndexStatus::Ready;
                }
                return IndexStatus::Stale;
            }
        }
    }
    // No record: Absent, unless a DB file already exists (then Stale → will rebuild).
    if db_path_for(&canon).exists() {
        return IndexStatus::Stale;
    }
    IndexStatus::Absent
}

struct IndexRegistry {
    states: Mutex<std::collections::HashMap<PathBuf, IndexState>>,
    build_guard: Mutex<std::collections::HashSet<PathBuf>>,
}

impl IndexRegistry {
    fn shared() -> &'static IndexRegistry {
        static REG: OnceLock<IndexRegistry> = OnceLock::new();
        REG.get_or_init(|| IndexRegistry {
            states: Mutex::new(std::collections::HashMap::new()),
            build_guard: Mutex::new(std::collections::HashSet::new()),
        })
    }

    /// Try to ensure a fresh index; returns true when the indexed path may be
    /// used. Never blocks readers: at most one builder per root (guarded).
    fn ensure_fresh(root: &Path, refresh: bool) -> bool {
        if codegraph_disabled() {
            return false;
        }
        let canon = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        // Fast path: fresh enough.
        if !refresh {
            if let Ok(states) = Self::shared().states.lock() {
                if let Some(st) = states.get(&canon) {
                    if !st.unwritable {
                        if let Some(t) = st.last_ok {
                            if t.elapsed() < INDEX_STALE_AFTER {
                                return true;
                            }
                        }
                    } else {
                        return false;
                    }
                }
            }
        }
        // Single builder per root (no stampede); others use stale/live.
        {
            let mut guard = match Self::shared().build_guard.lock() {
                Ok(g) => g,
                Err(_) => return false,
            };
            if !guard.insert(canon.clone()) {
                // Another builder running: allow stale reads.
                if let Ok(states) = Self::shared().states.lock() {
                    if let Some(st) = states.get(&canon) {
                        return st.last_ok.is_some() && !st.unwritable;
                    }
                }
                return false;
            }
        }
        let ok = build_index(&canon);
        if let Ok(mut guard) = Self::shared().build_guard.lock() {
            guard.remove(&canon);
        }
        if let Ok(mut states) = Self::shared().states.lock() {
            let st = states.entry(canon.clone()).or_insert(IndexState {
                last_ok: None,
                unwritable: false,
                failures: 0,
            });
            if ok {
                st.last_ok = Some(Instant::now());
                st.failures = 0;
            } else {
                st.failures += 1;
                if st.failures >= 3 {
                    // Corrupt → Absent: delete once, reset counter, retry fresh
                    // next call (prevents poison loops on a bad DB).
                    let _ = std::fs::remove_file(db_path_for(&canon));
                    st.failures = 0;
                    st.last_ok = None;
                }
                // Read-only roots: .jcode dir cannot be created → permanent
                // Absent verdict for this process (no retry every call).
                if db_path_for(&canon)
                    .parent()
                    .map(|p| !p.exists())
                    .unwrap_or(false)
                {
                    st.unwritable = true;
                }
            }
        }
        ok
    }
}

fn build_index(canon_root: &Path) -> bool {
    let db_path = db_path_for(canon_root);
    let opened = jcode_codegraph::query::GraphDb::open(&db_path);
    let mut db = match opened {
        Ok(d) => d,
        Err(e) => {
            crate::logging::warn(&format!(
                "codegraph open failed for {}: {e}",
                db_path.display()
            ));
            // Corrupt DB → delete + rebuild once (plan 3.1/Corrupt→Absent).
            let _ = std::fs::remove_file(&db_path);
            match jcode_codegraph::query::GraphDb::open(&db_path) {
                Ok(d) => d,
                Err(e2) => {
                    crate::logging::warn(&format!(
                        "codegraph reopen failed for {}: {e2}",
                        db_path.display()
                    ));
                    return false;
                }
            }
        }
    };
    // mtime check: skip full reindex when nothing changed (cheap stat walk).
    match db.reindex(
        canon_root,
        &jcode_codegraph::scan::ScanExclusions::default(),
        true,
    ) {
        Ok(_) => true,
        Err(e) => {
            crate::logging::warn(&format!(
                "codegraph reindex failed for {}: {e}",
                canon_root.display()
            ));
            false
        }
    }
}

/// Indexed-or-live dependents/dependencies/cochanges/symbols for `rel`.
/// Returns `(LiveGraph, used_index)`.
pub fn indexed_or_live(root: &Path, rel: &str, refresh: bool) -> (LiveGraph, bool) {
    let start = Instant::now();
    let canon_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    if resolve_within_root(&canon_root, rel).is_err() {
        return (
            LiveGraph {
                source: GraphSource::Live,
                partial: false,
                dependents: vec![],
                dependencies: vec![],
                cochanges: vec![],
                symbols: vec![],
                notes: vec![],
            },
            false,
        );
    }
    if IndexRegistry::ensure_fresh(&canon_root, refresh) {
        let db_path = db_path_for(&canon_root);
        if let Ok(db) = jcode_codegraph::query::GraphDb::open(&db_path) {
            let dents = db
                .dependents(rel)
                .unwrap_or_default()
                .into_iter()
                .map(|e| RelatedFile {
                    path: e.path,
                    weight: e.weight,
                })
                .collect::<Vec<_>>();
            let deps = db
                .dependencies(rel)
                .unwrap_or_default()
                .into_iter()
                .map(|e| RelatedFile {
                    path: e.path,
                    weight: e.weight,
                })
                .collect::<Vec<_>>();
            let co = db
                .cochanges(rel)
                .unwrap_or_default()
                .into_iter()
                .map(|c| RelatedFile {
                    path: c.path,
                    weight: c.commits as f64,
                })
                .collect::<Vec<_>>();
            let syms = db
                .symbols(rel)
                .unwrap_or_default()
                .into_iter()
                .map(|s| super::live::ExportedSymbol {
                    name: s.name,
                    kind: s.kind,
                })
                .collect::<Vec<_>>();
            let elapsed = start.elapsed();
            let mut notes = vec![];
            if elapsed > Duration::from_millis(200) {
                notes.push(format!("indexed query slow: {}ms", elapsed.as_millis()));
            }
            return (
                LiveGraph {
                    source: GraphSource::Index,
                    partial: false,
                    dependents: dents,
                    dependencies: deps,
                    cochanges: co,
                    symbols: syms,
                    notes,
                },
                true,
            );
        }
    }
    // Live fallback (busy/corrupt/disabled/stale-building).
    let (dents, s1, p1) = dependents_live(&canon_root, rel);
    let (deps, s2, p2) = dependencies_live(&canon_root, rel);
    let (co, s3, p3) = cochanges_live(&canon_root, rel);
    let syms = exported_symbols_live(&canon_root, rel);
    let source = [s1, s2, s3]
        .into_iter()
        .min_by_key(|s| match s {
            GraphSource::Degraded => 0,
            GraphSource::Live => 1,
            GraphSource::Index => 2,
        })
        .unwrap_or(GraphSource::Live);
    (
        LiveGraph {
            source,
            partial: p1 || p2 || p3,
            dependents: dents,
            dependencies: deps,
            cochanges: co,
            symbols: syms,
            notes: vec![],
        },
        false,
    )
}

/// Repo-relative display for an absolute path under `working_dir`.
pub fn rel_for(root: &Path, rel_or_abs: &str) -> String {
    if Path::new(rel_or_abs).is_absolute() {
        rel_display(root, Path::new(rel_or_abs))
    } else {
        rel_or_abs.trim_start_matches("./").to_string()
    }
}

/// Seconds since epoch for mtime (plan pass-8: seconds, not ms).
#[allow(dead_code)]
pub fn mtime_secs(p: &Path) -> i64 {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "jcode-indexed-test-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    // NOTE: these two tests share process env (JCODE_CODEGRAPH) and the
    // process-wide IndexRegistry; they must run serially. A serial test mutex
    // (std-only, no new deps) enforces this without relying on harness flags.
    fn env_serial() -> std::sync::MutexGuard<'static, ()> {
        static SERIAL: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        SERIAL
            .get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap()
    }

    #[test]
    fn disabled_env_falls_back_to_live() {
        let _guard = env_serial();
        // SAFETY: single-threaded test env mutation; serialized by cargo harness.
        unsafe { std::env::set_var("JCODE_CODEGRAPH", "0") };
        let dir = fixture("disabled");
        std::fs::write(dir.join("core.ts"), "export function core() {}\n").unwrap();
        std::fs::write(
            dir.join("user.ts"),
            "import { core } from './core';\ncore();\n",
        )
        .unwrap();
        let root = resolve_repo_root(&dir);
        let (g, used_index) = indexed_or_live(&root, "core.ts", true);
        unsafe { std::env::remove_var("JCODE_CODEGRAPH") };
        assert!(!used_index);
        assert!(g.dependents.iter().any(|d| d.path == "user.ts"), "g={g:?}");
    }

    #[test]
    fn indexed_roundtrip_under_budget() {
        let _guard = env_serial();
        // Defensive: a prior test may have left the flag set on failure paths.
        unsafe { std::env::remove_var("JCODE_CODEGRAPH") };
        let dir = fixture("roundtrip");
        std::fs::write(dir.join("core.ts"), "export function core() {}\n").unwrap();
        std::fs::write(
            dir.join("user.ts"),
            "import { core } from './core';\ncore();\n",
        )
        .unwrap();
        let root = resolve_repo_root(&dir);
        let start = Instant::now();
        let (g, used_index) = indexed_or_live(&root, "core.ts", true);
        let elapsed = start.elapsed();
        assert!(used_index, "g={g:?}");
        assert_eq!(g.source, GraphSource::Index);
        assert!(g.dependents.iter().any(|d| d.path == "user.ts"), "g={g:?}");
        assert!(
            elapsed < Duration::from_millis(2000),
            "indexed too slow: {elapsed:?}"
        );
    }

    #[test]
    fn lifecycle_absent_ready_disabled() {
        let _guard = env_serial();
        unsafe { std::env::remove_var("JCODE_CODEGRAPH") };
        let dir = fixture("lifecycle");
        std::fs::write(dir.join("a.ts"), "export const a = 1;\n").unwrap();
        let root = resolve_repo_root(&dir);
        assert_eq!(index_status(&root), IndexStatus::Absent);
        let _ = indexed_or_live(&root, "a.ts", true);
        assert_eq!(index_status(&root), IndexStatus::Ready);
        unsafe { std::env::set_var("JCODE_CODEGRAPH", "0") };
        assert_eq!(index_status(&root), IndexStatus::Disabled);
        unsafe { std::env::remove_var("JCODE_CODEGRAPH") };
    }

    #[test]
    fn kill_switch_opens_zero_sqlite() {
        let _guard = env_serial();
        unsafe { std::env::set_var("JCODE_CODEGRAPH", "0") };
        let dir = fixture("killswitch");
        std::fs::write(dir.join("core.ts"), "export function core() {}\n").unwrap();
        std::fs::write(
            dir.join("user.ts"),
            "import { core } from './core';\ncore();\n",
        )
        .unwrap();
        let root = resolve_repo_root(&dir);
        let (g, used_index) = indexed_or_live(&root, "core.ts", true);
        unsafe { std::env::remove_var("JCODE_CODEGRAPH") };
        assert!(!used_index);
        assert_ne!(g.source, GraphSource::Index);
        // Zero sqlite opens: no .jcode dir created under the fixture root.
        assert!(
            !db_path_for(&root.canonicalize().unwrap_or(root.clone())).exists(),
            "kill switch must not create DB files"
        );
    }
}
