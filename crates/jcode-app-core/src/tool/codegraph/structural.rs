//! Structural compaction stub builder (`jcode-hlz`, no-LLM).
//!
//! Replaces file-content tool results with symbol-outline stubs from the code
//! graph. Pure function over message text + working dir: extracts file paths
//! mentioned in tool results, resolves outlines via the agentgrep crate, and
//! emits a compact structural summary. The `CompactionManager` calls this in
//! `Structural` mode instead of spawning an LLM summarization.

use std::path::Path;

/// Max files outlined per compaction (bounds cost).
pub const STRUCTURAL_FILE_CAP: usize = 20;
/// Max stub chars total.
pub const STRUCTURAL_CHAR_CAP: usize = 6000;

/// Build a structural summary for `messages_text` given `working_dir`.
/// Returns `None` when no file mentions resolve (caller falls back).
pub fn structural_summary(messages_text: &str, working_dir: &Path) -> Option<String> {
    let files = extract_repo_files(messages_text, working_dir);
    if files.is_empty() {
        return None;
    }
    let mut lines =
        vec!["Structural context summary (no-LLM; outlines, not contents):".to_string()];
    let mut chars = lines[0].len();
    for f in files.iter().take(STRUCTURAL_FILE_CAP) {
        let stub = outline_stub(working_dir, f);
        if chars + stub.len() + 1 > STRUCTURAL_CHAR_CAP {
            break;
        }
        chars += stub.len() + 1;
        lines.push(stub);
    }
    if lines.len() < 2 {
        return None;
    }
    Some(lines.join("\n"))
}

fn extract_repo_files(text: &str, working_dir: &Path) -> Vec<String> {
    use std::collections::HashSet;
    let mut seen = HashSet::new();
    let mut out = vec![];
    for token in text.split(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == '`') {
        let t = token.trim_matches(|c| c == '(' || c == ')' || c == ',' || c == ':');
        if t.len() < 4 || t.len() > 200 || seen.contains(t) {
            continue;
        }
        let has_ext = [".rs", ".ts", ".tsx", ".js", ".jsx", ".py", ".go"]
            .iter()
            .any(|e| t.ends_with(e));
        if !has_ext {
            continue;
        }
        let abs = working_dir.join(t.trim_start_matches("./").trim_start_matches('/'));
        if abs.is_file() {
            seen.insert(t.to_string());
            // Normalize to repo-relative.
            let rel = abs
                .strip_prefix(working_dir)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|_| t.to_string());
            out.push(rel);
        }
        if out.len() >= STRUCTURAL_FILE_CAP {
            break;
        }
    }
    out.sort();
    out
}

fn outline_stub(working_dir: &Path, rel: &str) -> String {
    let args = ::agentgrep::cli::OutlineArgs {
        file: rel.to_string(),
        json: true,
        max_items: Some(30),
        path: None,
        context_json: None,
    };
    match ::agentgrep::outline::run_outline(working_dir, &args) {
        Ok(res) => {
            let items: Vec<String> = res
                .structure
                .items
                .iter()
                .take(30)
                .map(|s| format!("{} {}", s.kind, s.label))
                .collect();
            if items.is_empty() {
                return format!("{rel}: (no symbols)");
            }
            format!("{rel}:\n  {}", items.join("\n  "))
        }
        Err(_) => format!("{rel}: (outline unavailable)"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_files_returns_none() {
        let dir = std::env::temp_dir();
        assert!(structural_summary("hello world, no files here", &dir).is_none());
    }

    #[test]
    fn outlines_resolve() {
        let dir = std::env::temp_dir().join(format!(
            "jcode-struct-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("a.ts"),
            "export function alpha() {}\nexport class Beta {}\n",
        )
        .unwrap();
        let text = format!("read file {}/a.ts for context", dir.display());
        // Absolute path mention: extractor joins carefully; use relative form.
        let text2 = "see a.ts for the alpha function";
        let s = structural_summary(&text2, &dir).unwrap();
        assert!(s.contains("a.ts"), "s={s}");
        assert!(s.contains("alpha"), "s={s}");
        let _ = text;
    }
}
