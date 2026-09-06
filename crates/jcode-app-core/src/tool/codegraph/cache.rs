//! Process-wide dependency cache for live graph queries.
//!
//! Base tools are constructed once (`Registry::base_tools` + `OnceLock`) and
//! shared across sessions, so per-session state must live OUTSIDE the tool
//! struct — this cache does that. Keyed by canonicalized repo root, TTL 60s,
//! LRU-evicted at 100 roots. Phase 3 moves this into `jcode-codegraph`.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::live::{RelatedFile, cochanges_live, dependencies_live, dependents_live};

/// Cache entry lifetime (plan 3.1).
pub const DEP_CACHE_TTL: Duration = Duration::from_secs(60);
/// Max repo roots held (plan perf budgets: ~100 repos x small).
pub const DEP_CACHE_MAX_ROOTS: usize = 100;

#[derive(Debug, Clone, Default)]
pub struct CachedGraph {
    pub dependents: HashMap<String, Vec<RelatedFile>>,
    pub dependencies: HashMap<String, Vec<RelatedFile>>,
    pub cochanges: HashMap<String, Vec<RelatedFile>>,
}

struct Entry {
    graph: CachedGraph,
    inserted: Instant,
}

/// Process-wide `DepCache` behind a `OnceLock` (like `base_tools`).
pub struct DepCache {
    inner: Mutex<CacheInner>,
}

struct CacheInner {
    entries: HashMap<PathBuf, Entry>,
    order: VecDeque<PathBuf>,
}

impl DepCache {
    fn new() -> Self {
        Self {
            inner: Mutex::new(CacheInner {
                entries: HashMap::new(),
                order: VecDeque::new(),
            }),
        }
    }

    /// Global shared instance.
    pub fn shared() -> &'static DepCache {
        static CACHE: OnceLock<DepCache> = OnceLock::new();
        CACHE.get_or_init(DepCache::new)
    }

    fn canonical_root(root: &Path) -> PathBuf {
        root.canonicalize().unwrap_or_else(|_| root.to_path_buf())
    }

    /// Cached dependents for `rel` under `root`; computes + stores on miss.
    /// Returns `(files, from_cache)`.
    pub fn dependents(&self, root: &Path, rel: &str) -> (Vec<RelatedFile>, bool) {
        let key = Self::canonical_root(root);
        if let Some(hit) = self.get(&key, rel, |g| g.dependents.get(rel).cloned()) {
            return (hit, true);
        }
        let (files, _source, _partial) = dependents_live(root, rel);
        self.put_dependents(&key, rel.to_string(), files.clone());
        (files, false)
    }

    /// Cached dependencies for `rel` under `root`.
    pub fn dependencies(&self, root: &Path, rel: &str) -> (Vec<RelatedFile>, bool) {
        let key = Self::canonical_root(root);
        if let Some(hit) = self.get(&key, rel, |g| g.dependencies.get(rel).cloned()) {
            return (hit, true);
        }
        let (files, _source, _partial) = dependencies_live(root, rel);
        self.put_dependencies(&key, rel.to_string(), files.clone());
        (files, false)
    }

    /// Cached co-changes for `rel` under `root`.
    pub fn cochanges(&self, root: &Path, rel: &str) -> (Vec<RelatedFile>, bool) {
        let key = Self::canonical_root(root);
        if let Some(hit) = self.get(&key, rel, |g| g.cochanges.get(rel).cloned()) {
            return (hit, true);
        }
        let (files, _source, _partial) = cochanges_live(root, rel);
        self.put_cochanges(&key, rel.to_string(), files.clone());
        (files, false)
    }

    fn get(
        &self,
        key: &PathBuf,
        _rel: &str,
        pick: impl FnOnce(&CachedGraph) -> Option<Vec<RelatedFile>>,
    ) -> Option<Vec<RelatedFile>> {
        let inner = self.inner.lock().ok()?;
        let entry = inner.entries.get(key)?;
        if entry.inserted.elapsed() >= DEP_CACHE_TTL {
            return None;
        }
        pick(&entry.graph)
    }

    fn entry_mut(&self, key: &PathBuf) {
        let mut inner = match self.inner.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        if !inner.entries.contains_key(key) {
            if inner.order.len() >= DEP_CACHE_MAX_ROOTS {
                if let Some(old) = inner.order.pop_front() {
                    inner.entries.remove(&old);
                }
            }
            inner.order.push_back(key.clone());
            inner.entries.insert(
                key.clone(),
                Entry {
                    graph: CachedGraph::default(),
                    inserted: Instant::now(),
                },
            );
        }
    }

    fn put_dependents(&self, key: &PathBuf, rel: String, files: Vec<RelatedFile>) {
        self.entry_mut(key);
        if let Ok(mut inner) = self.inner.lock() {
            if let Some(e) = inner.entries.get_mut(key) {
                e.graph.dependents.insert(rel, files);
            }
        }
    }

    fn put_dependencies(&self, key: &PathBuf, rel: String, files: Vec<RelatedFile>) {
        self.entry_mut(key);
        if let Ok(mut inner) = self.inner.lock() {
            if let Some(e) = inner.entries.get_mut(key) {
                e.graph.dependencies.insert(rel, files);
            }
        }
    }

    fn put_cochanges(&self, key: &PathBuf, rel: String, files: Vec<RelatedFile>) {
        self.entry_mut(key);
        if let Ok(mut inner) = self.inner.lock() {
            if let Some(e) = inner.entries.get_mut(key) {
                e.graph.cochanges.insert(rel, files);
            }
        }
    }

    /// Test hook: number of cached roots.
    #[cfg(test)]
    pub fn len_for_test(&self) -> usize {
        self.inner.lock().map(|i| i.entries.len()).unwrap_or(0)
    }

    /// Test hook: clear all entries.
    #[cfg(test)]
    pub fn clear_for_test(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.entries.clear();
            inner.order.clear();
        }
    }
}
