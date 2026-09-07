//! Regex import + symbol extraction per language family (plan 5.1/5.3).
//!
//! Advisory precision (not AST). Shared with the live core's semantics:
//! unresolved bare imports surface as `external:<spec>`, never silently.

#[derive(Debug, Clone, PartialEq)]
pub struct ExportedSymbol {
    pub name: String,
    pub kind: String,
    pub line: usize,
    pub end_line: Option<usize>,
}

pub fn detect_language(path: &str) -> &'static str {
    if path.ends_with(".rs") {
        "rust"
    } else if path.ends_with(".ts") || path.ends_with(".tsx") || path.ends_with(".mts") {
        "ts"
    } else if path.ends_with(".js") || path.ends_with(".jsx") || path.ends_with(".mjs") {
        "js"
    } else if path.ends_with(".py") {
        "python"
    } else if path.ends_with(".go") {
        "go"
    } else {
        "generic"
    }
}

/// Raw import specifiers found in `content` for the file's language.
pub fn import_specs(path: &str, content: &str) -> Vec<String> {
    let lang = detect_language(path);
    let pats: &[&str] = match lang {
        "rust" => &[
            r"(?m)^\s*(?:pub\s+)?use\s+([^;]+);",
            r"(?m)^\s*mod\s+(\w+)\s*;",
        ],
        "ts" | "js" => &[
            r#"(?m)^\s*import\s+(?:[^'"]*from\s+)?['"]([^'"]+)['"]"#,
            r#"require\(\s*['"]([^'"]+)['"]\s*\)"#,
        ],
        "python" => &[r"(?m)^\s*(?:from\s+(\S+)\s+import|import\s+(.+))"],
        "go" => &[r#"(?m)^\s*(?:import\s+(?:\(\s*)?(?:"([^"]+)"|`([^`]+)`))"#],
        _ => &[
            r#"(?m)^\s*import\s+['"]?([^'"\s;]+)['"]?"#,
            r"(?m)^\\s*use\\s+([^;]+);",
        ],
    };
    let mut out = vec![];
    let mut seen = std::collections::HashSet::new();
    for pat in pats {
        let Ok(re) = regex::Regex::new(pat) else {
            continue;
        };
        for cap in re.captures_iter(content) {
            for i in 1..cap.len() {
                if let Some(m) = cap.get(i) {
                    let spec = m
                        .as_str()
                        .split_whitespace()
                        .next()
                        .unwrap_or("")
                        .trim_end_matches(',');
                    if !spec.is_empty() && seen.insert(spec.to_string()) {
                        out.push(spec.to_string());
                    }
                }
            }
        }
    }
    out
}

/// All-items symbols with line numbers (`line` is 1-based).
///
/// Advisory precision (not AST): regex-based, all visibilities. Kinds: `function`
/// (free fns incl. `async`/`unsafe`/`extern`), `method` (fns inside `impl` blocks),
/// `test` (`#[test]` fns), `class`/`enum`/`interface`/`module`/`constant`/`type`.
/// Dedup key is (name, line): methods like `new`/`execute` repeat across impl blocks.
pub fn exported_symbols(path: &str, content: &str) -> Vec<ExportedSymbol> {
    let lang = detect_language(path);
    let pats: &[(&str, &str)] = match lang {
        "rust" => &[
            // Test fns first so #[test] wins the kind over plain function.
            (
                r"(?m)^\s*#\[test\]\s*\n\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+(\w+)",
                "test",
            ),
            (
                r"(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+|unsafe\s+|extern\s+)*fn\s+(\w+)",
                "function",
            ),
            (r"(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?struct\s+(\w+)", "class"),
            (r"(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?enum\s+(\w+)", "enum"),
            (
                r"(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?trait\s+(\w+)",
                "interface",
            ),
            (
                r"(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?(?:const|static)\s+(?:mut\s+)?(\w+)",
                "constant",
            ),
            (r"(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)", "module"),
            (r"(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?type\s+(\w+)", "type"),
        ],
        "ts" | "js" => &[
            (
                r"(?m)^\s*export\s+(?:async\s+)?function\s+(\w+)",
                "function",
            ),
            (r"(?m)^\s*export\s+(?:default\s+)?class\s+(\w+)", "class"),
            (
                r"(?m)^\s*export\s+(?:default\s+)?interface\s+(\w+)",
                "interface",
            ),
            (r"(?m)^\s*export\s+type\s+(\w+)", "type"),
            (r"(?m)^\s*export\s+(?:const|let|var)\s+(\w+)", "variable"),
            (r"(?m)^\s*export\s+enum\s+(\w+)", "enum"),
        ],
        "python" => &[
            (r"(?m)^\s*def\s+(\w+)", "function"),
            (r"(?m)^\s*class\s+(\w+)", "class"),
        ],
        "go" => &[
            (
                r"(?m)^\s*func\s+(?:\(\w+\s+\*?\w+\)\s+)?([A-Z]\w*)",
                "function",
            ),
            (r"(?m)^\s*type\s+([A-Z]\w*)", "type"),
        ],
        _ => &[
            (r"(?m)^\s*(?:function\s+|fn\s+|def\s+)(\w+)", "function"),
            (r"(?m)^\s*(?:class|struct)\s+(\w+)", "class"),
        ],
    };
    // Brace-depth tracking for Rust impl blocks: a fn whose enclosing scope is
    // an `impl` is a method, not a free function (regex-only, advisory).
    let impl_spans: Vec<(usize, usize)> = if lang == "rust" {
        impl_block_spans(content)
    } else {
        vec![]
    };
    let mut out = vec![];
    let mut seen = std::collections::HashSet::new();
    for (pat, kind) in pats {
        let Ok(re) = regex::Regex::new(pat) else {
            continue;
        };
        for cap in re.captures_iter(content) {
            if let Some(m) = cap.get(1) {
                // Line number: count newlines before match start.
                let line = content[..m.start()].matches('\n').count() + 1;
                let name = m.as_str().to_string();
                if lang == "python" && name.starts_with('_') {
                    continue;
                }
                // Dedup on (name, line): same-named methods repeat across impls.
                // First pattern wins the kind (test patterns precede function).
                if seen.insert((name.clone(), line)) {
                    let mut final_kind = kind.to_string();
                    if lang == "rust" && final_kind == "function" && in_impl(&impl_spans, m.start())
                    {
                        final_kind = "method".to_string();
                    }
                    out.push(ExportedSymbol {
                        name,
                        kind: final_kind,
                        line,
                        end_line: None,
                    });
                }
            }
        }
    }
    out
}

/// Byte spans of `impl ... { ... }` bodies (brace-matched, advisory).
fn impl_block_spans(content: &str) -> Vec<(usize, usize)> {
    let bytes = content.as_bytes();
    let mut spans = vec![];
    let mut i = 0;
    while i < bytes.len() {
        if starts_impl_at(bytes, i) {
            if let Some(open) = content[i..].find('{') {
                let body_start = i + open;
                let mut depth = 0;
                let mut j = body_start;
                while j < bytes.len() {
                    match bytes[j] {
                        b'{' => depth += 1,
                        b'}' => {
                            depth -= 1;
                            if depth == 0 {
                                spans.push((body_start, j));
                                i = j;
                                break;
                            }
                        }
                        _ => {}
                    }
                    j += 1;
                }
            }
        }
        i += 1;
    }
    spans
}

fn starts_impl_at(bytes: &[u8], i: usize) -> bool {
    if i > 0 && bytes[i - 1] != b'\n' {
        return false;
    }
    let mut j = i;
    while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
        j += 1;
    }
    if bytes.len() < j + 4 || &bytes[j..j + 4] != b"impl" {
        return false;
    }
    let after = j + 4;
    if after < bytes.len() {
        let c = bytes[after];
        if c.is_ascii_alphanumeric() || c == b'_' {
            return false;
        }
    }
    true
}

fn in_impl(spans: &[(usize, usize)], pos: usize) -> bool {
    spans.iter().any(|(s, e)| pos >= *s && pos <= *e)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_specs_and_symbols() {
        let c = "use crate::a;\nmod b;\nuse std::collections::HashMap;\npub fn f() {}\n";
        let specs = import_specs("x.rs", c);
        assert!(specs.iter().any(|s| s.contains("crate")));
        let syms = exported_symbols("x.rs", c);
        assert!(syms.iter().any(|s| s.name == "f" && s.line == 4));
    }

    #[test]
    fn ts_specs() {
        let c = "import { h } from './util';\nimport fs from 'node:fs';\n";
        let specs = import_specs("m.ts", c);
        assert!(specs.contains(&"./util".to_string()));
        assert!(specs.contains(&"node:fs".to_string()));
    }

    #[test]
    fn rust_private_and_scoped_fn() {
        let c = "fn hidden() {}\nasync fn areq() {}\npub(crate) fn scoped() {}\npub fn open() {}\n";
        let syms = exported_symbols("x.rs", c);
        for name in ["hidden", "areq", "scoped", "open"] {
            assert!(
                syms.iter().any(|s| s.name == name && s.kind == "function"),
                "missing {name}"
            );
        }
        let lines: Vec<usize> = ["hidden", "areq", "scoped", "open"]
            .iter()
            .map(|n| syms.iter().find(|s| s.name == *n).unwrap().line)
            .collect();
        assert_eq!(lines, vec![1, 2, 3, 4]);
    }

    #[test]
    fn rust_methods_in_impl_blocks() {
        let c = "pub struct Foo;\nimpl Foo {\n    pub fn new() -> Self { Foo }\n    fn helper(&self) {}\n}\nfn free() {}\n";
        let syms = exported_symbols("x.rs", c);
        let methods: Vec<&ExportedSymbol> = syms.iter().filter(|s| s.kind == "method").collect();
        assert_eq!(methods.len(), 2, "methods: {methods:?}");
        assert!(
            syms.iter()
                .any(|s| s.name == "free" && s.kind == "function")
        );
        assert!(syms.iter().any(|s| s.name == "Foo" && s.kind == "class"));
    }

    #[test]
    fn rust_same_named_methods_not_collapsed() {
        let c = "struct A;\nimpl A {\n    fn new() -> Self { A }\n}\nstruct B;\nimpl B {\n    fn new() -> Self { B }\n}\n";
        let syms = exported_symbols("x.rs", c);
        let news: Vec<&ExportedSymbol> = syms.iter().filter(|s| s.name == "new").collect();
        assert_eq!(news.len(), 2, "both new() kept: {news:?}");
        assert!(news.iter().all(|s| s.kind == "method"));
        assert_ne!(news[0].line, news[1].line);
    }

    #[test]
    fn rust_test_fns_classified() {
        let c =
            "#[cfg(test)]\nmod tests {\n    #[test]\n    fn my_case() {}\n    fn helper() {}\n}\n";
        let syms = exported_symbols("x.rs", c);
        assert!(syms.iter().any(|s| s.name == "my_case" && s.kind == "test"));
        assert!(
            syms.iter()
                .any(|s| s.name == "helper" && s.kind == "function")
        );
    }

    #[test]
    fn rust_all_item_kinds_any_visibility() {
        let c = "struct S;\nenum E { A }\ntrait T {}\nmod m {}\nconst C: u8 = 1;\nstatic S2: u8 = 2;\ntype Alias = u8;\n";
        let syms = exported_symbols("x.rs", c);
        for (name, kind) in [
            ("S", "class"),
            ("E", "enum"),
            ("T", "interface"),
            ("m", "module"),
            ("C", "constant"),
            ("S2", "constant"),
            ("Alias", "type"),
        ] {
            assert!(
                syms.iter().any(|s| s.name == name && s.kind == kind),
                "missing {name}/{kind}"
            );
        }
    }
}
