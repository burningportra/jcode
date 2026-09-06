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

/// Exported symbols with line numbers (`line` is 1-based).
pub fn exported_symbols(path: &str, content: &str) -> Vec<ExportedSymbol> {
    let lang = detect_language(path);
    let pats: &[(&str, &str)] = match lang {
        "rust" => &[
            (r"(?m)^\s*pub\s+fn\s+(\w+)", "function"),
            (r"(?m)^\s*pub\s+struct\s+(\w+)", "class"),
            (r"(?m)^\s*pub\s+enum\s+(\w+)", "enum"),
            (r"(?m)^\s*pub\s+trait\s+(\w+)", "interface"),
            (r"(?m)^\s*pub\s+(?:const|static)\s+(\w+)", "constant"),
            (r"(?m)^\s*pub\s+mod\s+(\w+)", "module"),
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
                if seen.insert(name.clone()) {
                    out.push(ExportedSymbol {
                        name,
                        kind: kind.to_string(),
                        line,
                        end_line: None,
                    });
                }
            }
        }
    }
    out
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
}
