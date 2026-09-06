//! Tests for `code_impact` (`jcode-nd5`): tool execute paths, traversal guard,
//! missing working dir, advisory line helper.

use super::CodeImpactTool;
use crate::tool::{Tool, ToolContext, codegraph};
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
        execution_mode: crate::tool::ToolExecutionMode::Direct,
    }
}

fn fixture(name: &str) -> PathBuf {
    // Unique per test-name AND thread (tests run parallel in one process;
    // shared dirs race: one test's remove_dir_all wipes another's files).
    let dir = std::env::temp_dir().join(format!(
        "jcode-impact-test-{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[tokio::test]
async fn missing_working_dir_errors() {
    let tool = CodeImpactTool::new();
    let out = tool
        .execute(
            json!({"action": "blast_radius", "file": "a.rs"}),
            ctx_with_dir(None),
        )
        .await;
    assert!(out.is_err());
}

#[tokio::test]
async fn unknown_action_errors() {
    let tool = CodeImpactTool::new();
    let dir = fixture("action");
    let out = tool
        .execute(
            json!({"action": "explode", "file": "a.rs"}),
            ctx_with_dir(Some(dir)),
        )
        .await;
    assert!(out.is_err());
}

#[tokio::test]
async fn traversal_rejected() {
    let tool = CodeImpactTool::new();
    let dir = fixture("traverse");
    fs::write(dir.join("a.rs"), "pub fn a() {}\n").unwrap();
    let out = tool
        .execute(
            json!({"action": "blast_radius", "file": "../escape.rs"}),
            ctx_with_dir(Some(dir)),
        )
        .await;
    assert!(out.is_err());
}

#[tokio::test]
async fn blast_radius_reports_dependents() {
    let tool = CodeImpactTool::new();
    let dir = fixture("blast");
    fs::write(dir.join("core.ts"), "export function core() {}\n").unwrap();
    fs::write(
        dir.join("user.ts"),
        "import { core } from './core';\ncore();\n",
    )
    .unwrap();
    let out = tool
        .execute(
            json!({"action": "blast_radius", "file": "core.ts"}),
            ctx_with_dir(Some(dir)),
        )
        .await
        .unwrap();
    assert!(out.output.contains("Blast radius"), "out={}", out.output);
    assert!(out.output.contains("user.ts"), "out={}", out.output);
    assert!(out.output.contains("source:"), "out={}", out.output);
}

#[tokio::test]
async fn unknown_file_reports_not_found() {
    let tool = CodeImpactTool::new();
    let dir = fixture("missing");
    fs::write(dir.join("a.rs"), "pub fn a() {}\n").unwrap();
    let out = tool
        .execute(
            json!({"action": "blast_radius", "file": "nope.rs"}),
            ctx_with_dir(Some(dir)),
        )
        .await
        .unwrap();
    assert!(
        out.output.contains("not found in graph"),
        "out={}",
        out.output
    );
}

#[test]
fn advisory_line_present_for_imported_file() {
    let dir = fixture("advisory");
    fs::write(dir.join("core.ts"), "export function core() {}\n").unwrap();
    fs::write(
        dir.join("user.ts"),
        "import { core } from './core';\ncore();\n",
    )
    .unwrap();
    let ctx = ctx_with_dir(Some(dir.clone()));
    let line = super::advisory_blast_line(&ctx, &dir.join("core.ts"));
    assert!(line.is_some(), "expected advisory line");
    assert!(line.unwrap().contains("blast radius"));
}

#[test]
fn advisory_line_absent_for_leaf() {
    let dir = fixture("leaf");
    fs::write(dir.join("leaf.ts"), "console.log(1);\n").unwrap();
    let ctx = ctx_with_dir(Some(dir.clone()));
    assert!(super::advisory_blast_line(&ctx, &dir.join("leaf.ts")).is_none());
}

#[test]
fn codegraph_module_reexported() {
    let _ = codegraph::GraphSource::Live;
}
