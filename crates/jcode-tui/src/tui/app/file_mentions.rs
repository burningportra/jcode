//! Draft-preserving references to nonignored workspace files.
//! Uses filesystem ignore rules, so tracked-but-ignored files are omitted.
use super::{App, commands};
use crossterm::event::{KeyCode, KeyModifiers};
use std::{
    ops::Range,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

pub(super) const FILE_LABEL: &str = "Repository file";
const STATUS_LABEL: &str = "Repository files";

#[derive(Clone, Debug, PartialEq, Eq)]
struct Token {
    range: Range<usize>,
    query: String,
    text: String,
}

fn active_token(input: &str, cursor: usize) -> Option<Token> {
    if !input.is_char_boundary(cursor) {
        return None;
    }
    let mut consumed_until = 0;
    for (start, ch) in input.char_indices() {
        if start < consumed_until || start >= cursor {
            continue;
        }
        if ch != '@' || (start > 0 && !input[..start].chars().next_back()?.is_whitespace()) {
            continue;
        }
        let tail = &input[start + 1..];
        let quoted = tail.starts_with('"');
        let body_start = start + 1 + usize::from(quoted);
        if cursor < body_start {
            continue;
        }
        let body = &input[body_start..];
        let end = if quoted {
            let mut escaped = false;
            let mut close = None;
            for (i, ch) in body.char_indices() {
                if escaped {
                    escaped = false;
                    continue;
                }
                if ch == '\\' {
                    escaped = true;
                } else if ch == '"' {
                    close = Some(body_start + i + 1);
                    break;
                }
            }
            close.unwrap_or(input.len())
        } else {
            body_start + body.find(char::is_whitespace).unwrap_or(body.len())
        };
        consumed_until = end;
        if cursor > end {
            continue;
        }
        let query_end = if quoted && input[body_start..end].ends_with('"') {
            cursor.min(end - 1)
        } else {
            cursor
        };
        return Some(Token {
            range: start..end,
            query: input[body_start..query_end]
                .replace("\\\"", "\"")
                .replace("\\\\", "\\"),
            text: input[start..end].to_owned(),
        });
    }
    None
}

#[derive(Default)]
struct Listing {
    files: Vec<String>,
    notice: Option<&'static str>,
}

#[derive(Default)]
pub(super) struct FileMentionState {
    root: Option<PathBuf>,
    open: bool,
    files: Vec<String>,
    pending: Option<mpsc::Receiver<Listing>>,
    notice: Option<&'static str>,
    cancel: Option<Arc<AtomicBool>>,
    dismissed: Option<(Range<usize>, String)>,
}

impl Drop for FileMentionState {
    fn drop(&mut self) {
        self.stop();
    }
}

impl FileMentionState {
    fn stop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        self.pending = None;
    }

    fn close(&mut self) {
        self.stop();
        self.open = false;
    }

    fn start(&mut self, root: PathBuf) {
        self.stop();
        self.files.clear();
        self.notice = None;
        self.root = Some(root.clone());
        self.open = true;
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = Some(cancel.clone());
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        std::thread::spawn(move || {
            let _ = tx.send(list_files(&root, &cancel));
        });
    }
}

fn list_files(root: &Path, cancel: &AtomicBool) -> Listing {
    if std::fs::symlink_metadata(root)
        .map_or(true, |meta| !meta.is_dir() || meta.file_type().is_symlink())
    {
        return Listing {
            files: Vec::new(),
            notice: Some("File listing unavailable"),
        };
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut files = Vec::new();
    let mut notice = None;
    let walker = ignore::WalkBuilder::new(root)
        .hidden(false)
        .follow_links(false)
        .require_git(false)
        .filter_entry(|entry| entry.file_name() != ".git")
        .build();
    for (work, entry) in walker.enumerate() {
        if work >= 50_000
            || files.len() >= 20_000
            || Instant::now() >= deadline
            || cancel.load(Ordering::Relaxed)
        {
            notice = Some("Partial file listing: work or time limit reached");
            break;
        }
        let Ok(entry) = entry else {
            notice = Some("Partial file listing: some paths were inaccessible");
            continue;
        };
        if entry.error().is_some() {
            notice = Some("Partial file listing: some ignore rules could not be read");
        }
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let Ok(relative) = entry.path().strip_prefix(root) else {
            continue;
        };
        let Some(path) = relative.to_str() else {
            continue;
        };
        if path.chars().any(char::is_control) {
            continue;
        }
        files.push(path.to_owned());
    }
    files.sort_unstable();
    Listing { files, notice }
}

fn reference(path: &str) -> String {
    if path
        .chars()
        .any(|ch| ch.is_whitespace() || matches!(ch, '"' | '\\' | '`'))
    {
        format!("@\"{}\"", path.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        format!("@{path}")
    }
}

impl App {
    pub(super) fn poll_file_mentions(&mut self) -> bool {
        let mut state = self.file_mentions.borrow_mut();
        if active_token(&self.input, self.cursor_pos).is_none()
            || state.root != commands::active_working_dir(self)
            || self.pending_login.is_some()
            || self.pending_account_input.is_some()
            || self.pending_ssh_remote_name.is_some()
            || self.inline_interactive_state.is_some()
        {
            state.close();
            return false;
        }
        let Some(rx) = state.pending.as_ref() else {
            return false;
        };
        match rx.try_recv() {
            Ok(listing) => {
                state.files = listing.files;
                state.notice = listing.notice;
            }
            Err(mpsc::TryRecvError::Empty) => return false,
            Err(mpsc::TryRecvError::Disconnected) => {}
        }
        state.pending = None;
        drop(state);
        self.advance_command_suggestions_epoch();
        self.force_full_redraw = true;
        true
    }

    pub(super) fn file_mention_suggestions(&self) -> Option<Vec<(String, &'static str)>> {
        let mut state = self.file_mentions.borrow_mut();
        if crate::tui::is_ssh_remote() || self.input.trim_start().starts_with('!') {
            state.close();
            return None;
        }
        if state
            .dismissed
            .as_ref()
            .is_some_and(|(range, text)| self.input.get(range.clone()) != Some(text.as_str()))
        {
            state.dismissed = None;
        }
        let Some(token) = active_token(&self.input, self.cursor_pos) else {
            state.close();
            return None;
        };
        let root = commands::active_working_dir(self);
        if state.root != root {
            state.close();
            state.dismissed = None;
            state.root = root.clone();
        }
        let identity = (token.range.clone(), token.text.clone());
        if state.dismissed.as_ref() == Some(&identity) {
            return Some(Vec::new());
        }
        state.dismissed = None;
        let Some(root) = root else {
            return Some(vec![("File listing unavailable".into(), STATUS_LABEL)]);
        };
        if !state.open {
            state.start(root);
            return Some(vec![("Loading repository files…".into(), STATUS_LABEL)]);
        }
        if let Some(rx) = state.pending.as_ref() {
            match rx.try_recv() {
                Ok(listing) => {
                    state.files = listing.files;
                    state.notice = listing.notice;
                    state.pending = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    state.pending = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if state.pending.is_some() {
            return Some(vec![("Loading repository files…".into(), STATUS_LABEL)]);
        }
        let query = token.query.to_lowercase();
        let mut matches: Vec<_> = state
            .files
            .iter()
            .filter_map(|path| {
                let lower = path.to_lowercase();
                lower.find(&query).map(|position| (position, path))
            })
            .collect();
        matches.sort_unstable_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1)));
        let mut rows: Vec<_> = matches
            .into_iter()
            .take(100)
            .map(|(_, path)| (reference(path), FILE_LABEL))
            .collect();
        if let Some(notice) = state.notice {
            rows.push((notice.into(), STATUS_LABEL));
        } else if rows.is_empty() {
            rows.push(("No matching nonignored files".into(), STATUS_LABEL));
        }
        Some(rows)
    }

    pub(super) fn handle_file_mention_key(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> bool {
        if !modifiers.is_empty()
            || self.pending_login.is_some()
            || self.pending_account_input.is_some()
            || self.pending_ssh_remote_name.is_some()
            || self.inline_interactive_state.is_some()
        {
            return false;
        }
        let Some(rows) = self.file_mention_suggestions() else {
            return false;
        };
        if rows.is_empty() {
            // A dismissed reference still owns Escape, not the agent or draft.
            return code == KeyCode::Esc;
        }
        match code {
            KeyCode::Esc => {
                if let Some(token) = active_token(&self.input, self.cursor_pos) {
                    let mut state = self.file_mentions.borrow_mut();
                    state.close();
                    state.dismissed = Some((token.range, token.text));
                }
            }
            KeyCode::Up | KeyCode::Down => {
                let len = rows
                    .iter()
                    .filter(|(_, label)| *label == FILE_LABEL)
                    .count();
                if len == 0 {
                    return true;
                }
                let selected = self.command_suggestion_selected.min(len - 1);
                self.command_suggestion_selected = if code == KeyCode::Down {
                    (selected + 1) % len
                } else {
                    (selected + len - 1) % len
                };
            }
            KeyCode::Enter | KeyCode::Tab => {
                let selectable = rows
                    .iter()
                    .filter(|(_, label)| *label == FILE_LABEL)
                    .count();
                if selectable == 0 {
                    return true;
                }
                let selected = self.command_suggestion_selected.min(selectable - 1);
                if rows[selected].1 == FILE_LABEL {
                    let Some(token) = active_token(&self.input, self.cursor_pos) else {
                        return false;
                    };
                    let mut insertion = rows[selected].0.clone();
                    if self.input[token.range.end..]
                        .chars()
                        .next()
                        .is_none_or(|ch| !ch.is_whitespace())
                    {
                        insertion.push(' ');
                    }
                    self.remember_input_undo_state();
                    self.cursor_pos = token.range.start + insertion.len();
                    self.input.replace_range(token.range, &insertion);
                    self.tab_completion_state = None;
                    self.command_suggestion_selected = 0;
                    let mut state = self.file_mentions.borrow_mut();
                    state.close();
                    state.dismissed = active_token(&self.input, self.cursor_pos)
                        .map(|token| (token.range, token.text));
                }
            }
            _ => return false,
        }
        self.advance_command_suggestions_epoch();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn file_mention_tokens_are_cursor_and_utf8_safe() {
        assert!(active_token("hi a@b.com", 10).is_none());
        assert!(active_token("é @src tail", 1).is_none());
        let token = active_token("é @src tail", 6).unwrap();
        assert_eq!(token.range, 3..7);
        assert_eq!(token.query, "sr");
        assert!(active_token("@src tail", 9).is_none());
        assert_eq!(active_token("@\"a b\" rest", 5).unwrap().range, 0..6);
        assert_eq!(reference("a b.rs"), "@\"a b.rs\"");
    }
    #[test]
    fn file_mention_walker_respects_ignore_and_subtree() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".gitignore"), "ignored\n").unwrap();
        std::fs::write(dir.path().join("ignored"), "").unwrap();
        std::fs::write(dir.path().join("untracked.rs"), "").unwrap();
        std::fs::write(dir.path().join(".git/secret"), "").unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/a b.rs"), "").unwrap();
        let cancel = AtomicBool::new(false);
        let files = list_files(dir.path(), &cancel).files;
        assert!(files.contains(&"untracked.rs".into()));
        assert!(
            !files
                .iter()
                .any(|p| p.contains("ignored") || p.contains("secret"))
        );
        assert_eq!(
            list_files(&dir.path().join("sub"), &cancel).files,
            vec!["a b.rs"]
        );
        cancel.store(true, Ordering::Relaxed);
        assert!(list_files(dir.path(), &cancel).files.is_empty());
    }
}

#[cfg(test)]
mod app_tests {
    use super::*;

    fn app_with_files(input: &str, files: &[&str]) -> App {
        let mut app = super::super::tests::create_test_app();
        app.session.working_dir = Some("/file-mention-fixture".into());
        app.set_input_for_test(input);
        {
            let mut state = app.file_mentions.borrow_mut();
            state.root = commands::active_working_dir(&app);
            state.open = true;
            state.files = files.iter().map(|p| (*p).to_owned()).collect();
        }
        app
    }

    #[test]
    fn file_mention_same_input_new_cursor_invalidates_frame_cache() {
        let mut app = app_with_files("look @src @tests", &["src/main.rs", "tests/check.rs"]);
        let epoch = app.command_suggestions_epoch.get();
        assert_eq!(
            app.command_suggestions(),
            vec![("@tests/check.rs".into(), FILE_LABEL)]
        );
        app.cursor_pos = "look @src".len();
        assert_eq!(app.command_suggestions_epoch.get(), epoch);
        assert_eq!(
            app.command_suggestions(),
            vec![("@src/main.rs".into(), FILE_LABEL)]
        );
        assert_eq!(
            app.command_suggestions(),
            vec![("@src/main.rs".into(), FILE_LABEL)]
        );
    }

    #[test]
    fn file_mention_enter_and_tab_preserve_draft_and_undo_once() {
        for key in [KeyCode::Enter, KeyCode::Tab] {
            let original = "é inspect @src/old trailing text";
            let mut app = app_with_files(original, &["src/a b.rs"]);
            app.cursor_pos = "é inspect @src/".len();
            app.handle_key(key, KeyModifiers::empty()).unwrap();
            assert_eq!(app.input, "é inspect @\"src/a b.rs\" trailing text");
            assert_eq!(app.cursor_pos, "é inspect @\"src/a b.rs\"".len());
            assert!(!app.is_processing);
            assert!(
                app.command_suggestions().is_empty(),
                "accepted reference must not trap the next Enter"
            );
            app.undo_input_change();
            assert_eq!(app.input, original);
            assert_eq!(app.cursor_pos, "é inspect @src/".len());
        }
    }

    #[test]
    fn file_mention_selection_arrows_and_disconnected_tab() {
        let mut app = app_with_files("look @", &["a.rs", "b.rs"]);
        app.handle_key(KeyCode::Down, KeyModifiers::empty())
            .unwrap();
        assert_eq!(app.command_suggestion_selected, 1);
        app.handle_key(KeyCode::Up, KeyModifiers::empty()).unwrap();
        assert_eq!(app.command_suggestion_selected, 0);
        app.handle_key(KeyCode::Down, KeyModifiers::empty())
            .unwrap();
        super::super::remote::handle_disconnected_key(
            &mut app,
            KeyCode::Tab,
            KeyModifiers::empty(),
        )
        .unwrap();
        assert_eq!(app.input, "look @b.rs ");
        assert!(!app.is_processing);
    }

    #[test]
    fn file_mention_escape_suppresses_until_token_changes_without_cancel() {
        let mut app = app_with_files("keep @a", &["a.rs"]);
        app.is_processing = true;
        app.handle_key(KeyCode::Esc, KeyModifiers::empty()).unwrap();
        assert_eq!(app.input, "keep @a");
        assert!(!app.cancel_requested);
        assert!(app.command_suggestions().is_empty());
        app.handle_key(KeyCode::Esc, KeyModifiers::empty()).unwrap();
        assert_eq!(app.input, "keep @a");
        assert!(!app.cancel_requested);
        app.cursor_pos = 0;
        assert!(app.file_mention_suggestions().is_none());
        app.cursor_pos = app.input.len();
        assert_eq!(app.file_mention_suggestions(), Some(vec![]));
        app.set_input_for_test("keep @a.");
        assert!(!app.command_suggestions().is_empty());
    }

    #[test]
    fn file_mention_pending_and_empty_rows_never_submit_or_insert() {
        for pending in [false, true] {
            for key in [KeyCode::Enter, KeyCode::Tab] {
                let mut app = app_with_files("keep @missing", &[]);
                let (_tx, rx) = mpsc::channel();
                if pending {
                    app.file_mentions.borrow_mut().pending = Some(rx);
                }
                app.handle_key(key, KeyModifiers::empty()).unwrap();
                assert_eq!(app.input, "keep @missing");
                assert!(!app.is_processing);
            }
        }
    }

    #[test]
    fn file_mention_pending_credentials_suppress_listing() {
        let mut app = app_with_files("@secret", &["secret.rs"]);
        app.pending_login = Some(super::super::PendingLogin::ClaudeAccount {
            verifier: "test-verifier".into(),
            label: "test-account".into(),
            redirect_uri: None,
        });
        assert!(app.command_suggestions().is_empty());
        assert!(!app.handle_file_mention_key(KeyCode::Enter, KeyModifiers::empty()));
        assert_eq!(app.input, "@secret");
    }

    #[test]
    fn file_mention_partial_notice_is_not_selectable_or_inserted() {
        let mut app = app_with_files("@", &["a.rs"]);
        app.file_mentions.borrow_mut().notice =
            Some("Partial file listing: work or time limit reached");
        let rows = app.command_suggestions();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].1, STATUS_LABEL);
        app.handle_key(KeyCode::Down, KeyModifiers::empty())
            .unwrap();
        assert_eq!(app.command_suggestion_selected, 0);
        app.handle_key(KeyCode::Tab, KeyModifiers::empty()).unwrap();
        assert_eq!(app.input, "@a.rs ");

        let mut app = app_with_files("@missing", &[]);
        app.file_mentions.borrow_mut().notice =
            Some("Partial file listing: some paths were inaccessible");
        app.handle_key(KeyCode::Enter, KeyModifiers::empty())
            .unwrap();
        assert_eq!(app.input, "@missing");
        assert!(!app.is_processing);
        assert!(
            app.command_suggestions()[0]
                .0
                .starts_with("Partial file listing")
        );
    }

    #[test]
    fn file_mention_missing_root_is_unavailable_not_process_cwd() {
        let mut app = app_with_files("@", &["stale.rs"]);
        app.session.working_dir = None;
        assert_eq!(
            app.command_suggestions(),
            vec![("File listing unavailable".into(), STATUS_LABEL)]
        );
        app.handle_key(KeyCode::Enter, KeyModifiers::empty())
            .unwrap();
        assert_eq!(app.input, "@");
    }

    #[test]
    fn file_mention_async_root_switch_and_idle_poll() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        std::fs::write(first.path().join("first.rs"), "").unwrap();
        std::fs::write(second.path().join("second.rs"), "").unwrap();
        let mut app = app_with_files("@", &[]);
        app.session.working_dir = Some(first.path().to_string_lossy().into_owned());
        assert_eq!(app.command_suggestions()[0].1, STATUS_LABEL);
        app.session.working_dir = Some(second.path().to_string_lossy().into_owned());
        assert_eq!(app.command_suggestions()[0].1, STATUS_LABEL);
        let deadline = Instant::now() + Duration::from_secs(5);
        while !app.poll_file_mentions() {
            assert!(Instant::now() < deadline, "catalog did not complete");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(app.force_full_redraw);
        assert_eq!(
            app.command_suggestions(),
            vec![("@second.rs".into(), FILE_LABEL)]
        );
    }

    #[cfg(unix)]
    #[test]
    fn file_mention_walker_never_follows_symlinks() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), "").unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("linked")).unwrap();
        std::os::unix::fs::symlink(outside.path().join("secret"), root.path().join("file-link"))
            .unwrap();
        let cancel = AtomicBool::new(false);
        assert!(list_files(root.path(), &cancel).files.is_empty());
        assert!(
            list_files(&root.path().join("linked"), &cancel)
                .files
                .is_empty()
        );
    }
}

#[cfg(test)]
mod remote_tests {
    use super::*;

    #[test]
    fn file_mention_daemon_client_enter_and_tab_are_not_submission() {
        for key in [KeyCode::Enter, KeyCode::Tab] {
            let mut app = super::super::tests::create_test_app();
            app.runtime_mode = super::super::AppRuntimeMode::RemoteClient;
            app.session.working_dir = Some("/mention-client".into());
            app.set_input_for_test("review @src");
            {
                let mut state = app.file_mentions.borrow_mut();
                state.root = commands::active_working_dir(&app);
                state.open = true;
                state.files = vec!["src/main.rs".into()];
            }
            let rt = tokio::runtime::Runtime::new().unwrap();
            let _guard = rt.enter();
            let mut remote = crate::tui::backend::RemoteConnection::dummy();
            rt.block_on(super::super::remote::handle_remote_key(
                &mut app,
                key,
                KeyModifiers::empty(),
                &mut remote,
            ))
            .unwrap();
            assert_eq!(app.input, "review @src/main.rs ");
            assert!(!app.is_processing);
            assert!(!app.cancel_requested);
        }
    }
}
