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

#[tokio::test]
async fn budget_top_k_caps_blocks_with_disclosure() {
    let tool = CodeQueryTool::new();
    let dir = fixture("budget-n");
    for i in 0..5 {
        fs::write(dir.join(format!("f{i}.txt")), format!("needle {i}\n")).unwrap();
    }
    let full = tool
        .execute(
            json!({"pipeline": [
                {"op": "search", "pattern": "needle"},
                {"op": "read"}
            ]}),
            ctx_with_dir(Some(dir.clone())),
        )
        .await
        .unwrap();
    assert!(!full.output.contains("capped="), "out={}", full.output);
    assert!(!full.output.contains("est_tokens"), "out={}", full.output);
    let cut = tool
        .execute(
            json!({"pipeline": [
                {"op": "search", "pattern": "needle"},
                {"op": "read"},
                {"op": "budget", "n": 2}
            ]}),
            ctx_with_dir(Some(dir)),
        )
        .await
        .unwrap();
    assert!(
        cut.output.contains("capped=3 block(s)"),
        "out={}",
        cut.output
    );
    assert!(cut.output.contains("est_tokens"), "out={}", cut.output);
}

#[tokio::test]
async fn budget_max_tokens_caps_at_block_boundary() {
    let tool = CodeQueryTool::new();
    let dir = fixture("budget-tok");
    fs::write(dir.join("a.txt"), "alpha needle\n").unwrap();
    fs::write(dir.join("b.txt"), "beta needle\n").unwrap();
    let out = tool
        .execute(
            json!({"pipeline": [
                {"op": "search", "pattern": "needle"},
                {"op": "read"},
                {"op": "budget", "max_tokens": 1}
            ]}),
            ctx_with_dir(Some(dir)),
        )
        .await
        .unwrap();
    // At least one block survives (never empty when input non-empty); cap disclosed.
    assert!(out.output.contains("capped="), "out={}", out.output);
    assert!(out.output.contains("est_tokens"), "out={}", out.output);
}

#[tokio::test]
async fn budget_signatures_only_drops_read_bodies() {
    let tool = CodeQueryTool::new();
    let dir = fixture("budget-sig");
    fs::write(dir.join("core.ts"), "export function core() {}\n").unwrap();
    let out = tool
        .execute(
            json!({"pipeline": [
                {"op": "search", "pattern": "core"},
                {"op": "outline"},
                {"op": "read"},
                {"op": "budget", "signatures_only": true}
            ]}),
            ctx_with_dir(Some(dir)),
        )
        .await
        .unwrap();
    assert!(!out.output.contains("── "), "out={}", out.output);
    assert!(out.output.contains("capped="), "out={}", out.output);
}
