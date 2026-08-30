use super::{AgentGrepInput, ToolContext, ToolOutput};
use crate::config::FffBackendMode;
use fff_search::{
    FFFMode, FFFQuery, FilePicker, FilePickerOptions, FuzzyQuery, GrepMode, GrepSearchOptions,
    SharedFilePicker, SharedFrecency,
};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
#[cfg(test)]
use std::time::Duration;
use std::time::Instant;

const FFF_PAGE_LIMIT: usize = 512;
const FFF_SEARCH_BUDGET_MS: u64 = 4_000;

pub(super) enum BackendAttempt {
    Answer(ToolOutput),
    Fallback {
        metadata: Value,
        shadow_output: Option<String>,
    },
}

pub(super) struct FffIndexRegistry {
    entry: Mutex<Option<Arc<IndexEntry>>>,
    next_generation: AtomicU64,
    active_searches: AtomicUsize,
}

struct IndexEntry {
    root: PathBuf,
    root_hash: String,
    generation: u64,
    picker: SharedFilePicker,
    active_searches: AtomicUsize,
    created_at: Instant,
}

struct SearchPermit<'a> {
    registry: &'a AtomicUsize,
    entry: &'a AtomicUsize,
}

impl Drop for SearchPermit<'_> {
    fn drop(&mut self) {
        self.entry.fetch_sub(1, Ordering::AcqRel);
        self.registry.fetch_sub(1, Ordering::AcqRel);
    }
}

impl FffIndexRegistry {
    pub(super) fn new() -> Self {
        Self {
            entry: Mutex::new(None),
            next_generation: AtomicU64::new(1),
            active_searches: AtomicUsize::new(0),
        }
    }

    fn begin_search<'a>(&'a self, entry: &'a IndexEntry) -> Result<SearchPermit<'a>, &'static str> {
        let guard = self
            .entry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !guard
            .as_ref()
            .is_some_and(|current| current.generation == entry.generation)
        {
            return Err("stale_generation");
        }
        if self
            .active_searches
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                (active < 2).then_some(active + 1)
            })
            .is_err()
        {
            return Err("search_capacity");
        }
        entry.active_searches.fetch_add(1, Ordering::AcqRel);
        drop(guard);
        Ok(SearchPermit {
            registry: &self.active_searches,
            entry: &entry.active_searches,
        })
    }

    fn entry_for(&self, root: &Path) -> Result<Arc<IndexEntry>, &'static str> {
        let evicted = {
            let mut guard = self
                .entry
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(entry) = guard.as_ref() {
                if entry.root == root {
                    return Ok(entry.clone());
                }
                if entry.active_searches.load(Ordering::Acquire) != 0 {
                    return Err("index_busy");
                }
            }
            guard.take()
        };
        if let Some(entry) = evicted {
            entry.picker.cancel();
        }

        let generation = self.next_generation.fetch_add(1, Ordering::AcqRel);
        let picker = SharedFilePicker::default();
        let entry = Arc::new(IndexEntry {
            root: root.to_path_buf(),
            root_hash: crate::quality::sha256_bytes(root.to_string_lossy().as_bytes()),
            generation,
            picker: picker.clone(),
            active_searches: AtomicUsize::new(0),
            created_at: Instant::now(),
        });
        {
            let mut guard = self
                .entry
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(existing) = guard.as_ref() {
                if existing.root == root {
                    return Ok(existing.clone());
                }
                return Err("index_busy");
            }
            *guard = Some(entry.clone());
        }
        if FilePicker::new_with_shared_state(
            picker,
            SharedFrecency::default(),
            FilePickerOptions {
                base_path: root.display().to_string(),
                enable_content_indexing: true,
                mode: FFFMode::Ai,
                watch: true,
                ..Default::default()
            },
        )
        .is_err()
        {
            let mut guard = self
                .entry
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if guard
                .as_ref()
                .is_some_and(|current| current.generation == generation)
            {
                guard.take();
            }
            return Err("index_initialization_failed");
        }
        Ok(entry)
    }

    #[cfg(test)]
    pub(super) fn wait_for_ready(&self, timeout: Duration) -> bool {
        let entry = self
            .entry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let Some(entry) = entry else {
            return false;
        };
        entry.picker.wait_for_scan(timeout)
            && entry.picker.wait_for_indexing_complete(timeout)
            && entry.picker.wait_for_watcher(timeout)
            && entry.is_ready()
    }
}

impl IndexEntry {
    fn is_ready(&self) -> bool {
        self.picker
            .read()
            .ok()
            .and_then(|guard| {
                guard.as_ref().map(|picker| {
                    let progress = picker.get_scan_progress();
                    !progress.is_scanning
                        && progress.is_watcher_ready
                        && progress.is_warmup_complete
                        && !picker.is_post_scan_active()
                })
            })
            .unwrap_or(false)
    }
}

pub(super) fn attempt(
    params: &AgentGrepInput,
    ctx: &ToolContext,
    registry: &FffIndexRegistry,
    mode: FffBackendMode,
) -> BackendAttempt {
    if mode == FffBackendMode::Off {
        return fallback("ineligible", "disabled", None, None);
    }
    if let Some(reason) = ineligible_reason(params, ctx) {
        return fallback("ineligible", reason, None, None);
    }
    let Some(root) = index_root(ctx) else {
        return fallback("ineligible", "unsafe_or_missing_root", None, None);
    };
    let entry = match registry.entry_for(&root) {
        Ok(entry) => entry,
        Err(reason) => return fallback("ineligible", reason, None, None),
    };
    if !entry.is_ready() {
        return fallback(
            "warming",
            "index_warming",
            Some(&entry),
            Some(entry.created_at.elapsed().as_millis() as u64),
        );
    }
    let started = Instant::now();
    let _permit = match registry.begin_search(&entry) {
        Ok(permit) => permit,
        Err(reason) => return fallback("ready", reason, Some(&entry), None),
    };
    let search = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        search_paths(&entry, params.query.as_deref().unwrap_or_default())
    }))
    .unwrap_or(Err("fff_search_panicked"));
    match search {
        Ok(paths) => {
            let output = paths.join("\n");
            let metadata = json!({
                "search_backend": if mode == FffBackendMode::Prefer { "fff" } else { "linked_agentgrep" },
                "index_state": "ready",
                "fallback_reason": if mode == FffBackendMode::Prefer { Value::Null } else { Value::String("shadow_mode".into()) },
                "search_root_hash": entry.root_hash,
                "index_generation": entry.generation,
                "elapsed_ms": started.elapsed().as_millis() as u64,
                "returned_matches": paths.len(),
                "total_files_with_matches": paths.len(),
            });
            if mode == FffBackendMode::Prefer {
                BackendAttempt::Answer(
                    ToolOutput::new(output)
                        .with_title("agentgrep grep")
                        .with_metadata(metadata),
                )
            } else {
                BackendAttempt::Fallback {
                    metadata,
                    shadow_output: Some(output),
                }
            }
        }
        Err(reason) => fallback("ready", reason, Some(&entry), None),
    }
}

fn ineligible_reason(params: &AgentGrepInput, ctx: &ToolContext) -> Option<&'static str> {
    if ctx.working_dir.is_none() {
        return Some("missing_session_root");
    }
    if params.mode != "grep" {
        return Some("unsupported_mode");
    }
    if params.regex.unwrap_or(false) {
        return Some("regex_deferred");
    }
    if !params.paths_only.unwrap_or(false) {
        return Some("excerpt_mode_deferred");
    }
    if params.hidden.unwrap_or(false) {
        return Some("hidden_files_requested");
    }
    if params.no_ignore.unwrap_or(false) {
        return Some("ignored_files_requested");
    }
    if params.path.is_some() || params.file.is_some() {
        return Some("path_scope_deferred");
    }
    if params.glob.is_some() {
        return Some("glob_scope_deferred");
    }
    if params.file_type.is_some() {
        return Some("type_scope_deferred");
    }
    if params.query.as_deref().is_none_or(str::is_empty) {
        return Some("empty_query");
    }
    None
}

fn index_root(ctx: &ToolContext) -> Option<PathBuf> {
    let session_root = ctx.working_dir.as_ref()?.canonicalize().ok()?;
    let git_root = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(&session_root)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| {
            let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
            (!root.is_empty()).then(|| PathBuf::from(root))
        })
        .and_then(|root| root.canonicalize().ok())
        .unwrap_or(session_root);
    if git_root.parent().is_none() {
        return None;
    }
    if dirs::home_dir()
        .and_then(|home| home.canonicalize().ok())
        .is_some_and(|home| home == git_root)
    {
        return None;
    }
    Some(git_root)
}

fn search_paths(entry: &IndexEntry, query_text: &str) -> Result<Vec<String>, &'static str> {
    let guard = entry.picker.read().map_err(|_| "picker_lock_failed")?;
    let picker = guard.as_ref().ok_or("picker_missing")?;
    let query = FFFQuery {
        raw_query: query_text,
        constraints: vec![],
        fuzzy_query: FuzzyQuery::Text(query_text),
        location: None,
    };
    let mut offset = 0;
    let mut paths = Vec::new();
    loop {
        let result = picker.grep(
            &query,
            &GrepSearchOptions {
                max_matches_per_file: 1,
                smart_case: false,
                file_offset: offset,
                page_limit: FFF_PAGE_LIMIT,
                mode: GrepMode::PlainText,
                time_budget_ms: FFF_SEARCH_BUDGET_MS,
                classify_definitions: false,
                ..Default::default()
            },
        );
        if result.regex_fallback_error.is_some() || result.literal_fallback {
            return Err("fff_semantic_fallback");
        }
        paths.extend(result.files.iter().filter_map(|file| {
            let relative = PathBuf::from(file.relative_path(picker).to_string());
            if relative.is_absolute()
                || relative.components().any(|component| {
                    matches!(
                        component,
                        std::path::Component::ParentDir
                            | std::path::Component::RootDir
                            | std::path::Component::Prefix(_)
                    )
                })
                || !entry.root.join(&relative).is_file()
            {
                return None;
            }
            Some(relative.to_string_lossy().replace('\\', "/"))
        }));
        if result.next_file_offset == 0 || result.next_file_offset == offset {
            break;
        }
        offset = result.next_file_offset;
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn fallback(
    state: &'static str,
    reason: &'static str,
    entry: Option<&IndexEntry>,
    warming_ms: Option<u64>,
) -> BackendAttempt {
    BackendAttempt::Fallback {
        metadata: json!({
            "search_backend": "linked_agentgrep",
            "index_state": state,
            "fallback_reason": reason,
            "search_root_hash": entry.map(|entry| entry.root_hash.as_str()),
            "index_generation": entry.map(|entry| entry.generation),
            "warming_ms": warming_ms,
        }),
        shadow_output: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_index_is_not_replaced_and_global_search_capacity_is_bounded() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let first_root = first.path().canonicalize().unwrap();
        let second_root = second.path().canonicalize().unwrap();
        let registry = FffIndexRegistry::new();
        let entry = registry.entry_for(&first_root).unwrap();

        let first_permit = registry.begin_search(&entry).unwrap();
        let second_permit = registry.begin_search(&entry).unwrap();
        assert_eq!(registry.begin_search(&entry).err(), Some("search_capacity"));
        assert_eq!(registry.entry_for(&second_root).err(), Some("index_busy"));

        drop(first_permit);
        drop(second_permit);
        let replacement = registry.entry_for(&second_root).unwrap();
        assert!(replacement.generation > entry.generation);
        assert_eq!(replacement.root, second_root);
    }

    #[test]
    fn filesystem_and_home_roots_are_rejected() {
        let filesystem_root = PathBuf::from(std::path::MAIN_SEPARATOR.to_string());
        let filesystem_ctx = ToolContext {
            session_id: "root-test".into(),
            message_id: "root-test".into(),
            tool_call_id: "root-test".into(),
            working_dir: Some(filesystem_root),
            stdin_request_tx: None,
            graceful_shutdown_signal: None,
            execution_mode: crate::tool::ToolExecutionMode::Direct,
        };
        assert!(index_root(&filesystem_ctx).is_none());

        if let Some(home) = dirs::home_dir() {
            let home_ctx = ToolContext {
                working_dir: Some(home),
                ..filesystem_ctx
            };
            assert!(index_root(&home_ctx).is_none());
        }
    }
}
