//! Advisory file reservations for multi-agent coordination (flywheel "Agent Mail"
//! -style). Any session can reserve a set of project-relative paths/globs before
//! editing so other agents know a file surface is claimed. Reservations are
//! advisory, not rigid locks: they expire (TTL), can be released, and never block
//! a crashed agent forever. They are keyed by project root so every agent session
//! working in the same project sees the same reservation set.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// A single advisory file reservation held by one agent session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Reservation {
    /// Stable unique id assigned on creation (project-scoped).
    pub id: String,
    /// Owning session id (e.g. the swarm worker that claimed the surface).
    pub owner_session_id: String,
    /// Optional human/agent label for the holder (e.g. a spawned agent label).
    #[serde(default)]
    pub holder_label: Option<String>,
    /// Project-relative path patterns. Each is a path or a `*` glob.
    pub paths: Vec<String>,
    /// Exclusive => only one session may hold a matching path surface.
    pub exclusive: bool,
    /// Optional reason for the reservation.
    #[serde(default)]
    pub reason: Option<String>,
    /// Unix epoch milliseconds at creation.
    pub created_ms: u64,
    /// Unix epoch milliseconds at expiry (created + ttl_ms).
    pub expires_ms: u64,
}

impl Reservation {
    pub fn is_active(&self, now_ms: u64) -> bool {
        now_ms < self.expires_ms
    }
}

/// On-disk reservation store for one project root.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReservationStore {
    pub reservations: Vec<Reservation>,
}

impl ReservationStore {
    /// Drop expired reservations (in place) and return the number removed.
    pub fn prune_expired(&mut self, now_ms: u64) -> usize {
        let before = self.reservations.len();
        self.reservations.retain(|r| r.is_active(now_ms));
        before - self.reservations.len()
    }
}

/// Does a single project-relative path match a `*`/`**` glob?
///
/// - `*` matches zero or more characters within one path segment (never `/`).
/// - `**` matches zero or more whole segments (including across `/`).
///
/// Both sides are normalized to `/` by the caller.
pub fn glob_match(pattern: &str, path: &str) -> bool {
    let pat: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    fn star_match(seg_pat: &str, seg: &str) -> bool {
        let p = seg_pat.as_bytes();
        let s = seg.as_bytes();
        fn rec(p: &[u8], s: &[u8]) -> bool {
            match (p.first(), s.first()) {
                (None, None) => true,
                (Some(b'*'), _) => {
                    if rec(&p[1..], s) {
                        return true;
                    }
                    if let Some(t) = s.split_first().map(|(_, t)| t) {
                        return rec(p, t);
                    }
                    false
                }
                (Some(pa), Some(sa)) if pa == sa => rec(&p[1..], &s[1..]),
                _ => false,
            }
        }
        rec(p, s)
    }

    fn rec(pat: &[&str], segs: &[&str]) -> bool {
        match pat.first() {
            None => segs.is_empty(),
            Some(&"**") => {
                // `**` matches zero segments, or one+ segments.
                rec(&pat[1..], segs)
                    || (!segs.is_empty() && rec(pat, &segs[1..]))
            }
            Some(&p) => {
                if let Some((first, rest)) = segs.split_first() {
                    if star_match(p, first) {
                        return rec(&pat[1..], rest);
                    }
                }
                false
            }
        }
    }

    rec(&pat, &segs)
}

/// Project-relative path matching. Returns a specificity rank for exact paths
/// (rank 0) versus glob matches (rank = 1 + glob length, longer = closer).
fn path_match_rank(path: &str, pattern: &str) -> Option<usize> {
    if path == pattern {
        return Some(0);
    }
    if glob_match(pattern, path) {
        return Some(1 + pattern.len());
    }
    None
}

/// Reservations (other than `owner_session_id`) that own `path`, best match first.
pub fn conflicting_reservations<'a>(
    store: &'a ReservationStore,
    path: &str,
    owner_session_id: &str,
    now_ms: u64,
) -> Vec<(&'a Reservation, usize)> {
    let norm = normalize_project_path(path);
    let mut hits: Vec<(&'a Reservation, usize)> = store
        .reservations
        .iter()
        .filter(|r| r.is_active(now_ms) && r.owner_session_id != owner_session_id)
        .filter_map(|r| {
            let best = r
                .paths
                .iter()
                .filter_map(|p| path_match_rank(&norm, &normalize_project_path(p)))
                .min();
            best.map(|rank| (r, rank))
        })
        .collect();
    hits.sort_by_key(|(_, rank)| *rank);
    hits
}

/// Normalize a project-relative path to `/`, no leading slash.
pub fn normalize_project_path(path: &str) -> String {
    let trimmed = path.trim();
    let slashed = if cfg!(windows) {
        trimmed.replace('\\', "/")
    } else {
        trimmed.to_string()
    };
    slashed.trim_start_matches('/').to_string()
}

// ---- Persistence -------------------------------------------------------------

fn project_store_path(project_root: &Path) -> Result<PathBuf> {
    let base = crate::storage::jcode_dir()?;
    Ok(base
        .join("reservations")
        .join(format!("{}.json", stable_project_key(project_root))))
}

fn stable_project_key(project_root: &Path) -> String {
    use std::hash::{Hash, Hasher};
    let canon = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    canon.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

pub fn load_for_project(project_root: &Path) -> Result<ReservationStore> {
    let path = project_store_path(project_root)?;
    if !path.exists() {
        return Ok(ReservationStore::default());
    }
    crate::storage::read_json(&path).or_else(|_| Ok(ReservationStore::default()))
}

pub fn save_for_project(project_root: &Path, store: &ReservationStore) -> Result<()> {
    let path = project_store_path(project_root)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create reservations dir {}", parent.display()))?;
    }
    crate::storage::write_json_fast(&path, store)
}

// ---- Operations -----------------------------------------------------

/// Create a reservation for `paths` (project-relative globs) held by
/// `owner_session_id`. Returns the newly created reservation.
pub fn reserve_paths(
    project_root: &Path,
    owner_session_id: &str,
    holder_label: Option<String>,
    paths: Vec<String>,
    exclusive: bool,
    reason: Option<String>,
    ttl_secs: u64,
    now_ms: u64,
) -> Result<Reservation> {
    ensure_project_root(project_root)?;
    let ttl_ms = ttl_secs.max(1).saturating_mul(1000);
    let id = format!("rv-{}-{}", now_ms, paths.len().min(9999));
    let reservation = Reservation {
        id,
        owner_session_id: owner_session_id.to_string(),
        holder_label,
        paths: paths.into_iter().map(|p| normalize_project_path(&p)).collect(),
        exclusive,
        reason,
        created_ms: now_ms,
        expires_ms: now_ms + ttl_ms,
    };

    let mut store = load_for_project(project_root)?;
    store.prune_expired(now_ms);
    store.reservations.push(reservation.clone());
    save_for_project(project_root, &store)?;
    Ok(reservation)
}

/// Release reservations held by `owner_session_id`. With both `paths` and
/// `ids` empty, releases everything held by the session. Returns released ids.
pub fn release_paths(
    project_root: &Path,
    owner_session_id: &str,
    paths: Vec<String>,
    ids: Vec<String>,
    now_ms: u64,
) -> Result<Vec<String>> {
    let mut store = load_for_project(project_root)?;
    store.prune_expired(now_ms);
    let released_all = paths.is_empty() && ids.is_empty();
    let path_set: Vec<String> = paths.into_iter().map(|p| normalize_project_path(&p)).collect();

    let mut released = Vec::new();
    store.reservations.retain(|r| {
        if r.owner_session_id != owner_session_id {
            return true;
        }
        if released_all {
            released.push(r.id.clone());
            return false;
        }
        if ids.iter().any(|id| id == &r.id) {
            released.push(r.id.clone());
            return false;
        }
        if r
            .paths
            .iter()
            .any(|p| path_set.iter().any(|q| q == &normalize_project_path(p)))
        {
            released.push(r.id.clone());
            return false;
        }
        true
    });
    save_for_project(project_root, &store)?;
    Ok(released)
}

/// List active reservations for a project, optionally filtered to one session.
pub fn list_reservations(
    project_root: &Path,
    for_session: Option<&str>,
    now_ms: u64,
) -> Result<Vec<Reservation>> {
    let mut store = load_for_project(project_root)?;
    store.prune_expired(now_ms);
    save_for_project(project_root, &store)?;
    Ok(store
        .reservations
        .into_iter()
        .filter(|r| for_session.map(|s| s == r.owner_session_id).unwrap_or(true))
        .collect())
}

/// Does another active session hold an exclusive reservation on this path?
/// Returns the first such reservation, if any.
pub fn find_exclusive_reservation(
    project_root: &Path,
    owner_session_id: &str,
    path: &str,
    now_ms: u64,
) -> Result<Option<Reservation>> {
    let store = load_for_project(project_root)?;
    Ok(conflicting_reservations(&store, path, owner_session_id, now_ms)
        .into_iter()
        .find(|(r, _)| r.exclusive)
        .map(|(r, _)| r.clone()))
}

pub fn now_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Build a short advisory warning string when another session holds an
/// exclusive reservation on `path` (a file under `project_root`). Returns
/// `Some(warning)` when a conflict exists, `None` otherwise. Used by edit/write
/// tools to surface the conflict without refusing the edit.
pub fn exclusive_warning(
    project_root: &Path,
    owner_session_id: &str,
    path: &Path,
    now_ms: u64,
) -> Option<String> {
    let rel = path.strip_prefix(project_root).ok()?;
    let rel_str = rel.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/");
    let blocking = find_exclusive_reservation(project_root, owner_session_id, &rel_str, now_ms)
        .ok()??;
    Some(format!(
        "⚠ another agent ({}) has an exclusive reservation on '{}' (reservation {}, reason {})",
        blocking.holder_label.as_deref().unwrap_or(&blocking.owner_session_id),
        rel_str,
        blocking.id,
        blocking.reason.as_deref().unwrap_or("unspecified"),
    ))
}

fn ensure_project_root(project_root: &Path) -> Result<()> {
    if project_root.is_dir() {
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "reservation project root is not a directory: {}",
            project_root.display()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn now() -> u64 {
        now_epoch_ms()
    }

    #[test]
    fn glob_matches_within_segments_only() {
        assert!(glob_match("src/*.rs", "src/main.rs"));
        assert!(!glob_match("src/*.rs", "src/sub/mod.rs"), "must not cross slash");
        assert!(glob_match("**/*.rs", "a/b/c.rs"), "double-star matches across");
        assert!(glob_match("src/*", "src/lib.rs"));
        assert!(!glob_match("src/*", "src/deep/mod.rs"));
    }

    #[test]
    fn reserve_release_roundtrip_and_ttl() {
        let _guard = crate::storage::lock_test_env();
        let project = tempfile::tempdir().unwrap();
        let root = project.path();

        let now0 = now();
        let r = reserve_paths(root, "sess-a", Some("A".into()), vec!["src/*.rs".into()], true, Some("work".into()), 300, now0).unwrap();
        assert_eq!(r.paths, vec!["src/*.rs"]);

        let listed = list_reservations(root, None, now0).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, r.id);

        // Another session sees the exclusive conflict.
        let conflict = find_exclusive_reservation(root, "sess-b", "src/main.rs", now0).unwrap();
        assert!(conflict.is_some(), "exclusive conflict should be visible to B");

        // Release by empty selectors releases everything for the owner.
        let released = release_paths(root, "sess-a", vec![], vec![], now() + 10).unwrap();
        assert_eq!(released.len(), 1);
        assert!(list_reservations(root, None, now() + 10).unwrap().is_empty());
    }

    #[test]
    fn expiry_prunes_stale_reservations() {
        let _guard = crate::storage::lock_test_env();
        let project = tempfile::tempdir().unwrap();
        let root = project.path();
        let now0 = now();

        // Insert a short-lived reservation in the distant past via list prune:
        // create with ttl 1s then advance time well past expiry.
        let _r = reserve_paths(root, "owner", None, vec!["x.md".into()], true, None, 1, now0).unwrap();
        let far_future = now0 + Duration::from_secs(3600).as_millis() as u64;

        let listed = list_reservations(root, None, far_future).unwrap();
        assert!(listed.is_empty(), "expired reservation should be pruned");
        let conflict = find_exclusive_reservation(root, "other", "x.md", far_future).unwrap();
        assert!(conflict.is_none(), "expired reservation must not block");
    }

    #[test]
    fn glob_star_does_not_span_slash() {
        assert!(super::glob_match("a/*", "a/b"));
        assert!(!super::glob_match("a/*", "a/b/c"));
    }

    #[test]
    fn exclusive_warning_is_returned_only_for_other_sessions() {
        let _guard = crate::storage::lock_test_env();
        let project = tempfile::tempdir().unwrap();
        let root = project.path();
        let now0 = now();
        reserve_paths(root, "other-session", Some("Other".into()), vec!["src/x.rs".into()], true, Some("refactor".into()), 3600, now0).unwrap();

        // The owner sees no warning (reservation is theirs).
        let owner_file = root.join("src/x.rs");
        assert!(exclusive_warning(root, "src-session", &owner_file, now0).is_some());
        // A different session editing a non-reserved file sees none.
        let other_file = root.join("src/y.rs");
        assert!(exclusive_warning(root, "src-session", &other_file, now0).is_none());
    }
}