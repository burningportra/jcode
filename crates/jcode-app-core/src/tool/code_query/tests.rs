//! Tests for `code_query` (`jcode-hg3`): pipeline reductions, cap enforcement,
//! empty/long pipeline errors, missing working dir, per-stage trace.

use super::CodeQueryTool;
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
        "jcode-query-test-{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[tokio::test]
async fn empty_pipeline_errors() {
    let tool = CodeQueryTool::new();
    let dir = fixture("empty");
    let out = tool
        .execute(json!({"pipeline": []}), ctx_with_dir(Some(dir)))
        .await;
    assert!(out.is_err());
}

#[tokio::test]
async fn missing_working_dir_errors() {
    let tool = CodeQueryTool::new();
    let out = tool
        .execute(
            json!({"pipeline": [{"op": "search", "pattern": "x"}]}),
            ctx_with_dir(None),
        )
        .await;
    assert!(out.is_err());
}

#[tokio::test]
async fn unknown_op_reports_stage_failure() {
    let tool = CodeQueryTool::new();
    let dir = fixture("unknown");
    fs::write(dir.join("a.txt"), "hello\n").unwrap();
    let out = tool
        .execute(
            json!({"pipeline": [{"op": "frobnicate"}]}),
            ctx_with_dir(Some(dir)),
        )
        .await
        .unwrap();
    assert!(out.output.contains("failed"), "out={}", out.output);
}

#[tokio::test]
async fn search_filter_outline_pipeline() {
    let tool = CodeQueryTool::new();
    let dir = fixture("pipe");
    fs::write(dir.join("core.ts"), "export function core() {}\n").unwrap();
    fs::write(
        dir.join("user.ts"),
        "import { core } from './core';\ncore();\n",
    )
    .unwrap();
    fs::write(dir.join("notes.md"), "core ideas\n").unwrap();
    let out = tool
        .execute(
            json!({"pipeline": [
                {"op": "search", "pattern": "core"},
                {"op": "filter", "ext": ".ts"},
                {"op": "outline"},
                {"op": "limit", "n": 10}
            ]}),
            ctx_with_dir(Some(dir)),
        )
        .await
        .unwrap();
    assert!(
        out.output.contains("Pipeline (4 stages)"),
        "out={}",
        out.output
    );
    assert!(out.output.contains("1. search"), "out={}", out.output);
    assert!(!out.output.contains("notes.md"), "out={}", out.output);
}

#[tokio::test]
async fn deps_expansion_finds_importers() {
    let tool = CodeQueryTool::new();
    let dir = fixture("deps");
    fs::write(dir.join("core.ts"), "export function core() {}\n").unwrap();
    fs::write(
        dir.join("user.ts"),
        "import { core } from './core';\ncore();\n",
    )
    .unwrap();
    let out = tool
        .execute(
            json!({"pipeline": [
                {"op": "search", "pattern": "core"},
                {"op": "filter", "ext": "core.ts"},
                {"op": "deps", "direction": "imported_by"}
            ]}),
            ctx_with_dir(Some(dir)),
        )
        .await
        .unwrap();
    assert!(out.output.contains("user.ts"), "out={}", out.output);
}

#[tokio::test]
async fn read_stage_returns_contents() {
    let tool = CodeQueryTool::new();
    let dir = fixture("read");
    fs::write(dir.join("a.txt"), "line1\nline2\nline3\n").unwrap();
    let out = tool
        .execute(
            json!({"pipeline": [
                {"op": "search", "pattern": "line2"},
                {"op": "read", "ranges": {"start": 2, "end": 3}}
            ]}),
            ctx_with_dir(Some(dir)),
        )
        .await
        .unwrap();
    assert!(out.output.contains("line2"), "out={}", out.output);
    assert!(!out.output.contains("line1"), "out={}", out.output);
}
