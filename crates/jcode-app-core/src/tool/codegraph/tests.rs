//! Tests for the live code-graph core (`jcode-rey`).
//!
//! Bead obligations: per-language import fixtures (rs/ts/py/go), co-change on
//! a fixture git repo, path traversal rejection, rg-missing degraded path,
//! binary skip, timeout partials, cache TTL behavior.

use super::cache::DepCache;
use super::live::*;
use std::fs;
use std::path::PathBuf;

fn fixture_root(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "jcode-codegraph-test-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(root: &std::path::Path, rel: &str, content: &str) -> PathBuf {
    let p = root.join(rel);
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&p, content).unwrap();
    p
}

#[test]
fn rust_imports_detected() {
    let root = fixture_root("rust");
    write(&root, "a.rs", "pub fn alpha() {}\n");
    write(
        &root,
        "b.rs",
        "use crate::a;\nmod c;\nuse std::collections::HashMap;\n",
    );
    let (deps, source, partial) = dependencies_live(&root, "b.rs");
    assert_eq!(source, GraphSource::Live);
    assert!(!partial);
    let specs: Vec<_> = deps.iter().map(|d| d.path.as_str()).collect();
    assert!(
        specs.iter().any(|s| s.contains("crate") || s.contains('a')),
        "specs={specs:?}"
    );
    assert!(
        specs.iter().any(|s| s.starts_with("external:")),
        "specs={specs:?}"
    );
}

#[test]
fn ts_imports_detected() {
    let root = fixture_root("ts");
    write(&root, "util.ts", "export function help() {}\n");
    write(
        &root,
        "main.ts",
        "import { help } from './util';\nconst x = require('./util');\nimport fs from 'node:fs';\n",
    );
    let (deps, _, _) = dependencies_live(&root, "main.ts");
    let specs: Vec<_> = deps.iter().map(|d| d.path.as_str()).collect();
    assert!(
        specs.iter().any(|s| s.ends_with("util.ts")),
        "specs={specs:?}"
    );
    assert!(
        specs.iter().any(|s| *s == "external:node:fs"),
        "specs={specs:?}"
    );
}

#[test]
fn python_imports_detected() {
    let root = fixture_root("py");
    write(&root, "pkg/__init__.py", "");
    write(&root, "pkg/mod.py", "import os\nfrom . import other\n");
    let (deps, _, _) = dependencies_live(&root, "pkg/mod.py");
    assert!(!deps.is_empty());
}

#[test]
fn go_imports_detected() {
    let root = fixture_root("go");
    write(
        &root,
        "main.go",
        "package main\nimport \"fmt\"\nimport \"./lib\"\nfunc main() {}\n",
    );
    let (deps, _, _) = dependencies_live(&root, "main.go");
    assert!(!deps.is_empty());
}

#[test]
fn rust_symbols_extracted() {
    let root = fixture_root("sym-rs");
    write(
        &root,
        "lib.rs",
        "pub fn alpha() {}\npub struct Beta;\npub enum Gamma { A }\npub trait Delta {}\npub const E: u8 = 1;\nfn private() {}\n",
    );
    let syms = exported_symbols_live(&root, "lib.rs");
    let names: Vec<_> = syms.iter().map(|s| s.name.as_str()).collect();
    for want in ["alpha", "Beta", "Gamma", "Delta", "E"] {
        assert!(names.contains(&want), "names={names:?}");
    }
    assert!(!names.contains(&"private"));
}

#[test]
fn ts_symbols_extracted() {
    let root = fixture_root("sym-ts");
    write(
        &root,
        "a.ts",
        "export function f() {}\nexport class C {}\nexport interface I {}\nexport type T = string;\nexport const v = 1;\n",
    );
    let syms = exported_symbols_live(&root, "a.ts");
    assert!(syms.len() >= 5, "syms={syms:?}");
}

#[test]
fn path_traversal_rejected() {
    let root = fixture_root("traversal");
    write(&root, "a.rs", "pub fn a() {}\n");
    assert!(resolve_within_root(&root, "../escape.rs").is_err());
    assert!(resolve_within_root(&root, "a.rs").is_ok());
    let (deps, _, _) = dependencies_live(&root, "../escape.rs");
    assert!(deps.is_empty());
}

#[test]
fn binary_files_skipped() {
    let root = fixture_root("binary");
    let p = write(&root, "blob.o", "");
    fs::write(&p, vec![0u8; 100]).unwrap();
    assert!(is_binary_file(&p));
    let (deps, _, _) = dependencies_live(&root, "blob.o");
    assert!(deps.is_empty());
    let (dents, _, _) = dependents_live(&root, "blob.o");
    assert!(dents.is_empty());
}

#[test]
fn cochange_on_fixture_git_repo() {
    let root = fixture_root("git");
    let git = |args: &[&str]| {
        let st = std::process::Command::new("git")
            .args(args)
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(st.status.success(), "git {args:?} failed");
    };
    git(&["init"]);
    git(&["config", "user.email", "t@t.t"]);
    git(&["config", "user.name", "t"]);
    write(&root, "a.txt", "a1\n");
    write(&root, "b.txt", "b1\n");
    git(&["add", "."]);
    git(&["commit", "-m", "one"]);
    write(&root, "a.txt", "a2\n");
    write(&root, "b.txt", "b2\n");
    git(&["add", "."]);
    git(&["commit", "-m", "two"]);
    let (co, source, partial) = cochanges_live(&root, "a.txt");
    assert_eq!(source, GraphSource::Live);
    assert!(!partial);
    assert!(
        co.iter().any(|c| c.path == "b.txt" && c.weight >= 1.0),
        "co={co:?}"
    );
}

#[test]
fn cochange_degrades_outside_git() {
    let root = fixture_root("nogit");
    write(&root, "a.txt", "hi\n");
    let (co, source, partial) = cochanges_live(&root, "a.txt");
    assert!(co.is_empty());
    assert_eq!(source, GraphSource::Degraded);
    assert!(partial);
}

#[test]
fn dependents_finds_importers() {
    let root = fixture_root("dents");
    write(&root, "core.ts", "export function core() {}\n");
    write(
        &root,
        "user.ts",
        "import { core } from './core';\ncore();\n",
    );
    write(&root, "other.ts", "console.log('nothing');\n");
    let (dents, _, _) = dependents_live(&root, "core.ts");
    let paths: Vec<_> = dents.iter().map(|d| d.path.as_str()).collect();
    assert!(paths.contains(&"user.ts"), "paths={paths:?}");
    assert!(!paths.contains(&"other.ts"));
}

#[test]
fn cache_serves_second_call() {
    DepCache::shared().clear_for_test();
    let root = fixture_root("cache");
    write(&root, "a.rs", "pub fn a() {}\n");
    write(&root, "b.rs", "use crate::a;\n");
    let canon = root.canonicalize().unwrap_or(root.clone());
    let (_, first_cached) = DepCache::shared().dependencies(&canon, "b.rs");
    assert!(!first_cached);
    let (_, second_cached) = DepCache::shared().dependencies(&canon, "b.rs");
    assert!(second_cached);
}

#[test]
fn repo_root_walks_up_to_git() {
    let root = fixture_root("rootwalk");
    let sub = root.join("a").join("b");
    fs::create_dir_all(&sub).unwrap();
    fs::create_dir_all(root.join(".git")).unwrap();
    assert_eq!(resolve_repo_root(&sub), root);
}
