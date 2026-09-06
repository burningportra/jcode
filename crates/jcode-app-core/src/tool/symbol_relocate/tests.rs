//! Tests for `symbol_relocate` (`jcode-rqk`): rename success, injected-failure
//! restore, traversal guards, mandatory blast preview.

use super::SymbolRelocateTool;
use crate::tool::{Tool, ToolContext, ToolExecutionMode};
use jcode_agent_runtime::InterruptSignal;
use serde_json::json;
use std::fs;
use std::path::PathBuf;

fn ctx_with_dir(dir: Option<PathBuf>) -> ToolContext {
    ToolContext {
        session_id: "test".to_string(),
        message_id: "m".to_string(),
        tool_call_id: "t".to_string(),
        working_dir: dir,
        stdin_request_tx: None,
        graceful_shutdown_signal: Some(InterruptSignal::new()),
        execution_mode: ToolExecutionMode::Direct,
    }
}

fn fixture(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "jcode-reloc-test-{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[tokio::test]
async fn rename_across_files_with_preview() {
    let tool = SymbolRelocateTool::new();
    let dir = fixture("rename");
    fs::write(dir.join("a.ts"), "export function alpha() {}\n").unwrap();
    fs::write(dir.join("b.ts"), "import { alpha } from './a';\nalpha();\n").unwrap();
    let out = tool
        .execute(
            json!({"file": "a.ts", "symbol": "alpha", "new_name": "beta"}),
            ctx_with_dir(Some(dir.clone())),
        )
        .await
        .unwrap();
    assert!(out.output.contains("Blast preview"), "out={}", out.output);
    assert!(out.output.contains("Per-file status"), "out={}", out.output);
    let b = fs::read_to_string(dir.join("b.ts")).unwrap();
    assert!(b.contains("beta") && !b.contains("alpha"), "b={b}");
}

#[tokio::test]
async fn injected_failure_restores_nothing() {
    let tool = SymbolRelocateTool::new();
    let dir = fixture("restore");
    fs::write(dir.join("a.ts"), "export function alpha() {}\n").unwrap();
    fs::write(dir.join("b.ts"), "import { alpha } from './a';\nalpha();\n").unwrap();
    let before_b = fs::read_to_string(dir.join("b.ts")).unwrap();
    let out = tool
        .execute(
            json!({"file": "a.ts", "symbol": "alpha", "new_name": "beta", "fail_after": 0}),
            ctx_with_dir(Some(dir.clone())),
        )
        .await
        .unwrap();
    assert!(out.output.contains("ABORTED"), "out={}", out.output);
    assert_eq!(fs::read_to_string(dir.join("b.ts")).unwrap(), before_b);
}

#[tokio::test]
async fn traversal_guards_hold() {
    let tool = SymbolRelocateTool::new();
    let dir = fixture("guards");
    fs::write(dir.join("a.ts"), "export function alpha() {}\n").unwrap();
    assert!(
        tool.execute(
            json!({"file": "../x.ts", "symbol": "a", "new_name": "b"}),
            ctx_with_dir(Some(dir.clone())),
        )
        .await
        .is_err()
    );
    assert!(
        tool.execute(
            json!({"file": "a.ts", "symbol": "alpha", "new_name": "beta", "dst": "../y.ts"}),
            ctx_with_dir(Some(dir)),
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn missing_working_dir_errors() {
    let tool = SymbolRelocateTool::new();
    assert!(
        tool.execute(
            json!({"file": "a.ts", "symbol": "a", "new_name": "b"}),
            ctx_with_dir(None),
        )
        .await
        .is_err()
    );
}

#[test]
fn word_boundary_replacement_is_conservative() {
    let (out, n) = super::replace_symbol_text("foo foobar foo_bar foo", "foo", "baz");
    assert_eq!(n, 2, "out={out}");
    assert!(out.contains("foobar") && out.contains("foo_bar"));
    // F5: `$` is an identifier char (JS `$foo` must not match `foo`).
    let (out2, n2) = super::replace_symbol_text("$foo foo foo$bar", "foo", "baz");
    assert_eq!(n2, 1, "out2={out2}");
    assert!(out2.contains("$foo") && out2.contains("foo$bar"));
}
