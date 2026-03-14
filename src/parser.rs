//! Tree-sitter based import and dependency extractor.
#![allow(clippy::items_after_test_module)]

use std::cell::RefCell;
use std::path::Path;
use tracing::debug;
use tree_sitter::{Language as TsLanguage, Node, Parser as TsParser};

/// Supported programming languages
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    Rust,
    TypeScript,
    Python,
    Go,
}

enum FastParseResult {
    Complete(Vec<String>),
    Fallback,
}

const MAX_FILE_SIZE: usize = 512 * 1024;

// Thread-local parser storage to reuse parser instances across files in the same thread.
thread_local! {
    static RUST_PARSER: RefCell<TsParser> = RefCell::new(create_parser(tree_sitter_rust::LANGUAGE.into()));
    static TS_PARSER: RefCell<TsParser> = RefCell::new(create_parser(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()));
    static PY_PARSER: RefCell<TsParser> = RefCell::new(create_parser(tree_sitter_python::LANGUAGE.into()));
    static GO_PARSER: RefCell<TsParser> = RefCell::new(create_parser(tree_sitter_go::LANGUAGE.into()));
}

fn create_parser(lang: TsLanguage) -> TsParser {
    let mut p = TsParser::new();
    p.set_language(&lang)
        .expect("Failed to set tree-sitter language");
    p
}

pub fn detect_language(path: &str) -> Option<Language> {
    let ext = path.rsplit('.').next()?;
    match ext {
        "rs" => Some(Language::Rust),
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" => Some(Language::TypeScript),
        "py" => Some(Language::Python),
        "go" => Some(Language::Go),
        _ => None,
    }
}

/// Original simple package extraction logic from main branch.
pub fn extract_package_name(file_path: &Path) -> String {
    extract_package_name_with_depth(file_path, 2)
}

pub fn extract_package_name_with_depth(file_path: &Path, package_depth: usize) -> String {
    extract_package_name_str_with_depth(&file_path.to_string_lossy(), package_depth)
}

pub fn extract_package_name_str(path_str: &str) -> String {
    extract_package_name_str_with_depth(path_str, 2)
}

pub fn extract_package_name_str_with_depth(path_str: &str, package_depth: usize) -> String {
    let normalized = path_str.replace('\\', "/");
    let file_path = Path::new(&normalized);
    let components: Vec<&str> = normalized.split('/').filter(|c| !c.is_empty()).collect();

    // Single file at root (no directory) → use file stem
    if components.len() <= 1 {
        return file_path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
    }

    // Directory components only (exclude the file name at the end)
    let dirs = &components[..components.len() - 1];

    // Check for monorepo patterns: packages/X/... or apps/X/...
    if let Some(pos) = dirs.iter().position(|c| *c == "packages" || *c == "apps")
        && pos + 1 < dirs.len()
    {
        return dirs[pos + 1].to_string();
    }

    // Skip common meaningless root directories
    const SKIP_ROOTS: &[&str] = &["src", "lib", "internal"];
    let meaningful_start = if !dirs.is_empty() && SKIP_ROOTS.contains(&dirs[0]) {
        1
    } else {
        0
    };

    let meaningful = &dirs[meaningful_start..];
    let depth = package_depth.max(1);

    match meaningful.len() {
        0 => file_path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string(),
        len if len <= depth => meaningful.join("/"),
        _ => meaningful[..depth].join("/"),
    }
}

fn may_contain_imports(content: &str, lang: Language) -> bool {
    match lang {
        Language::Rust => {
            content.contains("use ")
                || content.contains("use\t")
                || content.contains("extern crate")
        }
        Language::TypeScript => {
            content.contains("import ")
                || content.contains("import{")
                || content.contains("import(")
                || content.contains("export ")
                || content.contains("require(")
                || content.contains(" from ")
        }
        Language::Python => content.contains("import "),
        Language::Go => content.contains("import "),
    }
}

pub fn parse_imports(content: &str, lang: Language) -> Vec<String> {
    if content.len() > MAX_FILE_SIZE {
        debug!(size = content.len(), "File too large, skipping");
        return Vec::new();
    }
    if !may_contain_imports(content, lang) {
        return Vec::new();
    }

    let fast_imports = match lang {
        Language::Rust => extract_rust_imports_fast_safe(content),
        Language::TypeScript => extract_typescript_imports_fast_safe(content),
        Language::Python => extract_python_imports_fast_safe(content),
        Language::Go => extract_go_imports_fast_safe(content),
    };
    if let FastParseResult::Complete(fast_imports) = fast_imports {
        return fast_imports;
    }

    let source = content.as_bytes();

    let tree = match lang {
        Language::Rust => RUST_PARSER.with(|p| p.borrow_mut().parse(content, None)),
        Language::TypeScript => TS_PARSER.with(|p| p.borrow_mut().parse(content, None)),
        Language::Python => PY_PARSER.with(|p| p.borrow_mut().parse(content, None)),
        Language::Go => GO_PARSER.with(|p| p.borrow_mut().parse(content, None)),
    };

    let Some(tree) = tree else {
        return Vec::new();
    };
    let root = tree.root_node();

    match lang {
        Language::Rust => extract_rust_imports(root, source),
        Language::TypeScript => extract_typescript_imports(root, source),
        Language::Python => extract_python_imports(root, source),
        Language::Go => extract_go_imports(root, source),
    }
}

fn extract_rust_imports(root: Node, source: &[u8]) -> Vec<String> {
    let mut imports = Vec::new();
    walk_nodes_recursive(root, &mut |child| match child.kind() {
        "use_declaration" => {
            if let Ok(text) = child.utf8_text(source) {
                let text = text.trim();
                if let Some(path) = text.strip_prefix("use ") {
                    let path = path.trim_end_matches(';').trim();
                    let first = path.split("::").next().unwrap_or(path);
                    if !matches!(first, "self" | "crate" | "super") {
                        imports.push(first.to_string());
                    }
                }
            }
        }
        "extern_crate_declaration" => {
            if let Ok(text) = child.utf8_text(source)
                && let Some(name) = text.strip_prefix("extern crate ")
            {
                let name = name.trim_end_matches(';').trim();
                if !name.is_empty() {
                    imports.push(name.to_string());
                }
            }
        }
        _ => {}
    });
    imports
}

fn extract_typescript_imports(root: Node, source: &[u8]) -> Vec<String> {
    let mut imports = Vec::new();
    walk_nodes_recursive(root, &mut |node| match node.kind() {
        "import_statement" | "export_statement" => {
            if let Ok(text) = node.utf8_text(source)
                && let StatementParseResult::Complete(statement_imports) =
                    collect_typescript_statement_imports(text, text)
            {
                for module in statement_imports {
                    push_unique_import(&mut imports, module);
                }
            }
        }
        "call_expression" => {
            if let Ok(text) = node.utf8_text(source) {
                if let Some(module) = parse_static_call_source(text, "require") {
                    push_unique_import(&mut imports, module);
                }
                if let Some(module) = parse_static_call_source(text, "import") {
                    push_unique_import(&mut imports, module);
                }
            }
        }
        _ => {}
    });
    imports
}

fn extract_rust_imports_fast_safe(content: &str) -> FastParseResult {
    let Some(masked) = mask_rust_code(content) else {
        return FastParseResult::Fallback;
    };
    FastParseResult::Complete(extract_rust_imports_fast(&masked))
}

fn extract_rust_imports_fast(content: &str) -> Vec<String> {
    let mut imports = Vec::new();
    let mut statement = String::new();

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }

        let starts_statement = line.starts_with("use ")
            || line.starts_with("pub use ")
            || line.starts_with("extern crate ");
        if !statement.is_empty() || starts_statement {
            if !statement.is_empty() {
                statement.push(' ');
            }
            statement.push_str(line);
            if line.ends_with(';') {
                collect_rust_statement_imports(&statement, &mut imports);
                statement.clear();
            }
        }
    }

    if !statement.is_empty() {
        collect_rust_statement_imports(&statement, &mut imports);
    }

    imports
}

fn collect_rust_statement_imports(statement: &str, imports: &mut Vec<String>) {
    let stmt = statement.trim();
    if let Some(path) = stmt
        .strip_prefix("use ")
        .or_else(|| stmt.strip_prefix("pub use "))
    {
        let path = path.trim_end_matches(';').trim();
        let first = path.split("::").next().unwrap_or(path).trim();
        if !matches!(first, "self" | "crate" | "super") && !first.is_empty() {
            imports.push(first.to_string());
        }
        return;
    }

    if let Some(name) = stmt.strip_prefix("extern crate ") {
        let name = name.trim_end_matches(';').trim();
        if !name.is_empty() {
            imports.push(name.to_string());
        }
    }
}

fn extract_python_imports(root: Node, source: &[u8]) -> Vec<String> {
    let mut imports = Vec::new();
    walk_nodes_recursive(root, &mut |child| match child.kind() {
        "import_statement" => {
            if let Ok(text) = child.utf8_text(source)
                && let Some(module_part) = text.trim().strip_prefix("import ")
            {
                for module in module_part.split(',') {
                    let m = module.trim().split(" as ").next().unwrap_or("").trim();
                    let top = m.split('.').next().unwrap_or(m);
                    if !top.is_empty() {
                        imports.push(top.to_string());
                    }
                }
            }
        }
        "import_from_statement" => {
            if let Ok(text) = child.utf8_text(source)
                && let Some(rest) = text.trim().strip_prefix("from ")
            {
                let m_path = rest.split_whitespace().next().unwrap_or("");
                if m_path.starts_with('.') {
                    imports.extend(extract_python_relative_imports(text.trim(), m_path));
                } else {
                    let top = m_path.split('.').next().unwrap_or(m_path);
                    if !top.is_empty() {
                        imports.push(top.to_string());
                    }
                }
            }
        }
        _ => {}
    });
    imports
}

fn extract_go_imports(root: Node, source: &[u8]) -> Vec<String> {
    let mut imports = Vec::new();
    walk_nodes_recursive(root, &mut |child| {
        if child.kind() == "import_declaration" {
            collect_go_import_strings(child, source, &mut imports);
        }
    });
    imports
}

fn collect_go_import_strings(node: Node, source: &[u8], imports: &mut Vec<String>) {
    if node.kind() == "interpreted_string_literal" || node.kind() == "raw_string_literal" {
        if let Ok(text) = node.utf8_text(source) {
            let path = text.trim_matches(|c| c == '"' || c == '`');
            if !path.is_empty() {
                imports.push(path.to_string());
            }
        }
        return;
    }
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            collect_go_import_strings(child, source, imports);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typescript_relative_imports_preserve_raw_path() {
        let content = "import foo from './core/http';\nimport bar from '../ui/http';";
        let imports = parse_imports(content, Language::TypeScript);

        assert_eq!(imports, vec!["./core/http", "../ui/http"]);
    }

    #[test]
    fn package_name_depth_is_configurable() {
        assert_eq!(
            extract_package_name_str_with_depth("src/auth/strategies/jwt.rs", 1),
            "auth"
        );
        assert_eq!(
            extract_package_name_str_with_depth("src/auth/strategies/jwt.rs", 2),
            "auth/strategies"
        );
    }

    #[test]
    fn javascript_family_extensions_map_to_typescript_parser() {
        assert_eq!(detect_language("src/app.js"), Some(Language::TypeScript));
        assert_eq!(detect_language("src/app.jsx"), Some(Language::TypeScript));
        assert_eq!(detect_language("src/app.mjs"), Some(Language::TypeScript));
        assert_eq!(detect_language("src/app.cjs"), Some(Language::TypeScript));
    }

    #[test]
    fn typescript_fast_path_ignores_comments_and_strings() {
        let content = r#"
// import fake from "commented";
const example = "import also_fake from 'string'";
/* export * from "ignored"; */
import real from "./real";
const later = require("./dep");
"#;

        let imports = parse_imports(content, Language::TypeScript);
        assert_eq!(imports, vec!["./real", "./dep"]);
    }

    #[test]
    fn typescript_import_equals_require_is_not_double_counted() {
        let content = r#"import foo = require("foo");"#;
        let imports = parse_imports(content, Language::TypeScript);
        assert_eq!(imports, vec!["foo"]);
    }

    #[test]
    fn typescript_static_dynamic_import_is_preserved() {
        let content = r#"const mod = import("./stable");"#;
        let imports = parse_imports(content, Language::TypeScript);
        assert_eq!(imports, vec!["./stable"]);
    }

    #[test]
    fn typescript_dynamic_template_import_is_not_treated_as_static_dependency() {
        let content = r#"
import stable from "./stable";
const mod = import(`./${name}`);
"#;

        let imports = parse_imports(content, Language::TypeScript);
        assert_eq!(imports, vec!["./stable"]);
    }

    #[test]
    fn typescript_multiline_require_falls_back_to_ast_and_is_preserved() {
        let content = r#"
const dep = require(
  "./stable"
);
"#;

        let imports = parse_imports(content, Language::TypeScript);
        assert_eq!(imports, vec!["./stable"]);
    }

    #[test]
    fn typescript_multiline_dynamic_import_falls_back_to_ast_and_is_preserved() {
        let content = r#"
const dep = import(
  "./stable"
);
"#;

        let imports = parse_imports(content, Language::TypeScript);
        assert_eq!(imports, vec!["./stable"]);
    }

    #[test]
    fn typescript_non_static_require_expression_is_ignored() {
        let content = r#"const dep = require(cond ? "a" : "b");"#;
        let imports = parse_imports(content, Language::TypeScript);
        assert!(imports.is_empty());
    }

    #[test]
    fn typescript_export_from_is_preserved_via_fallback() {
        let content = r#"export { value } from "./shared";"#;
        let imports = parse_imports(content, Language::TypeScript);
        assert_eq!(imports, vec!["./shared"]);
    }

    #[test]
    fn python_fast_path_ignores_comments_and_docstrings() {
        let content = r#"
"""import fake"""
# import skipped
import os
from pkg.sub import item
"#;

        let imports = parse_imports(content, Language::Python);
        assert_eq!(imports, vec!["os", "pkg"]);
    }

    #[test]
    fn python_relative_imports_are_preserved_for_scan_resolution() {
        let content = r#"
from . import config
from .sub import item
from ..shared import util
"#;

        let imports = parse_imports(content, Language::Python);
        assert_eq!(imports, vec![".config", ".sub", "..shared"]);
    }

    #[test]
    fn go_fast_path_ignores_comments() {
        let content = r#"
// import "fake/comment"
package main

import (
    "fmt"
    /* "ignored/block" */
    "net/http"
)
"#;

        let imports = parse_imports(content, Language::Go);
        assert_eq!(imports, vec!["fmt", "net/http"]);
    }

    #[test]
    fn rust_fast_path_ignores_comments_and_strings() {
        let content = r#"
// use fake::comment;
const EXAMPLE: &str = "use fake::string;";
use real::module;
"#;

        let imports = parse_imports(content, Language::Rust);
        assert_eq!(imports, vec!["real"]);
    }
}

fn extract_typescript_imports_fast_safe(content: &str) -> FastParseResult {
    let Some(masked) = mask_c_like_code(content, true, true, false) else {
        return FastParseResult::Fallback;
    };
    extract_typescript_imports_fast(content, &masked)
}

fn extract_typescript_imports_fast(original: &str, masked: &str) -> FastParseResult {
    let mut imports = Vec::new();
    let mut original_statement = String::new();
    let mut masked_statement = String::new();

    for (raw_original, raw_masked) in original.lines().zip(masked.lines()) {
        let line = raw_masked.trim();
        if line.is_empty() {
            continue;
        }

        let starts_statement = line.starts_with("import ") || line.starts_with("export ");
        if !masked_statement.is_empty() || starts_statement {
            if !masked_statement.is_empty() {
                original_statement.push(' ');
                masked_statement.push(' ');
            }
            original_statement.push_str(raw_original.trim());
            masked_statement.push_str(line);
            let should_flush =
                line.ends_with(';') || line.contains(" from ") || line.contains("require(");
            if should_flush {
                let StatementParseResult::Complete(statement_imports) =
                    collect_typescript_statement_imports(&original_statement, &masked_statement)
                else {
                    return FastParseResult::Fallback;
                };
                imports.extend(statement_imports);
                original_statement.clear();
                masked_statement.clear();
            }
            continue;
        }

        let StatementParseResult::Complete(statement_imports) =
            collect_typescript_statement_imports(raw_original.trim(), line)
        else {
            return FastParseResult::Fallback;
        };
        imports.extend(statement_imports);
    }

    if !masked_statement.is_empty() {
        let StatementParseResult::Complete(statement_imports) =
            collect_typescript_statement_imports(&original_statement, &masked_statement)
        else {
            return FastParseResult::Fallback;
        };
        imports.extend(statement_imports);
    }

    FastParseResult::Complete(imports)
}

enum StatementParseResult {
    Complete(Vec<String>),
    Fallback,
}

fn collect_typescript_statement_imports(original: &str, masked: &str) -> StatementParseResult {
    let mut statement_imports = Vec::new();
    let trimmed_original = original.trim();
    let trimmed_masked = masked.trim();

    if trimmed_masked.is_empty() {
        return StatementParseResult::Complete(statement_imports);
    }

    if let Some(idx) = trimmed_masked.find(" from ") {
        if let Some(module) = parse_module_source_after_from(&trimmed_original[idx + 6..]) {
            push_unique_import(&mut statement_imports, module);
        } else {
            return StatementParseResult::Fallback;
        }
        return StatementParseResult::Complete(statement_imports);
    }

    if trimmed_masked.starts_with("import ")
        && !trimmed_masked.contains("= require(")
        && !trimmed_masked.contains("import(")
        && !trimmed_masked.contains("require(")
    {
        if let Some(module) = parse_bare_import_source(trimmed_original) {
            push_unique_import(&mut statement_imports, module);
            return StatementParseResult::Complete(statement_imports);
        }
        return StatementParseResult::Fallback;
    }

    if trimmed_masked.starts_with("export ")
        && !trimmed_masked.contains("require(")
        && !trimmed_masked.contains("import(")
    {
        return StatementParseResult::Complete(statement_imports);
    }

    if let Some(idx) = trimmed_masked.find("= require(") {
        if let Some(module) = parse_static_call_source(&trimmed_original[idx + 2..], "require") {
            push_unique_import(&mut statement_imports, module);
            return StatementParseResult::Complete(statement_imports);
        }
        return StatementParseResult::Fallback;
    }

    let require_count = trimmed_masked.matches("require(").count();
    if require_count > 0 {
        if require_count > 1 {
            return StatementParseResult::Fallback;
        }
        if let Some(idx) = trimmed_masked.find("require(")
            && let Some(module) = parse_static_call_source(&trimmed_original[idx..], "require")
        {
            push_unique_import(&mut statement_imports, module);
            return StatementParseResult::Complete(statement_imports);
        }
        return StatementParseResult::Fallback;
    }

    let dynamic_import_count = trimmed_masked.matches("import(").count();
    if dynamic_import_count > 0 {
        if dynamic_import_count > 1 {
            return StatementParseResult::Fallback;
        }
        if let Some(idx) = trimmed_masked.find("import(")
            && let Some(module) = parse_static_call_source(&trimmed_original[idx..], "import")
        {
            push_unique_import(&mut statement_imports, module);
            return StatementParseResult::Complete(statement_imports);
        }
        return StatementParseResult::Fallback;
    }

    StatementParseResult::Complete(statement_imports)
}

fn extract_python_imports_fast_safe(content: &str) -> FastParseResult {
    let Some(masked) = mask_python_code(content) else {
        return FastParseResult::Fallback;
    };
    FastParseResult::Complete(extract_python_imports_fast(&masked))
}

fn extract_python_imports_fast(content: &str) -> Vec<String> {
    let mut imports = Vec::new();

    for raw_line in content.lines() {
        let line = raw_line.trim_start();
        if let Some(module_part) = line.strip_prefix("import ") {
            for module in module_part.split(',') {
                let module = module.trim().split(" as ").next().unwrap_or("").trim();
                let top = module.split('.').next().unwrap_or(module);
                if !top.is_empty() {
                    imports.push(top.to_string());
                }
            }
            continue;
        }

        if let Some(rest) = line.strip_prefix("from ") {
            let module = rest.split_whitespace().next().unwrap_or("");
            if module.starts_with('.') {
                imports.extend(extract_python_relative_imports(line, module));
            } else {
                let top = module.split('.').next().unwrap_or(module);
                if !top.is_empty() {
                    imports.push(top.to_string());
                }
            }
        }
    }

    imports
}

fn extract_go_imports_fast_safe(content: &str) -> FastParseResult {
    let Some(masked) = mask_c_comments_only(content) else {
        return FastParseResult::Fallback;
    };
    FastParseResult::Complete(extract_go_imports_fast(content, &masked))
}

fn extract_go_imports_fast(original: &str, masked: &str) -> Vec<String> {
    let mut imports = Vec::new();
    let mut in_block = false;

    for (raw_original, raw_masked) in original.lines().zip(masked.lines()) {
        let line = raw_masked.trim();
        if line.is_empty() {
            continue;
        }

        if in_block {
            if line.starts_with(')') {
                in_block = false;
                continue;
            }
            if let Some(path) = find_quoted_literal(line) {
                imports.push(path);
            }
            continue;
        }

        if line == "import (" {
            in_block = true;
            continue;
        }

        if line.strip_prefix("import ").is_some()
            && let Some(path) = find_quoted_literal(raw_original)
        {
            imports.push(path);
        }
    }

    imports
}

fn mask_rust_code(content: &str) -> Option<String> {
    mask_c_like_code(content, false, false, true)
}

fn mask_c_comments_only(content: &str) -> Option<String> {
    let bytes = content.as_bytes();
    let mut masked = String::with_capacity(content.len());
    let mut idx = 0usize;
    let mut in_line_comment = false;
    let mut block_comment_depth = 0usize;

    while idx < bytes.len() {
        let b = bytes[idx];

        if in_line_comment {
            if b == b'\n' {
                in_line_comment = false;
                masked.push('\n');
            } else {
                masked.push(' ');
            }
            idx += 1;
            continue;
        }

        if block_comment_depth > 0 {
            if b == b'/' && bytes.get(idx + 1) == Some(&b'*') {
                block_comment_depth += 1;
                masked.push(' ');
                masked.push(' ');
                idx += 2;
                continue;
            }
            if b == b'*' && bytes.get(idx + 1) == Some(&b'/') {
                block_comment_depth = block_comment_depth.saturating_sub(1);
                masked.push(' ');
                masked.push(' ');
                idx += 2;
                continue;
            }
            masked.push(if b == b'\n' { '\n' } else { ' ' });
            idx += 1;
            continue;
        }

        if b == b'/' && bytes.get(idx + 1) == Some(&b'/') {
            in_line_comment = true;
            masked.push(' ');
            masked.push(' ');
            idx += 2;
            continue;
        }

        if b == b'/' && bytes.get(idx + 1) == Some(&b'*') {
            block_comment_depth = 1;
            masked.push(' ');
            masked.push(' ');
            idx += 2;
            continue;
        }

        masked.push(b as char);
        idx += 1;
    }

    if in_line_comment || block_comment_depth > 0 {
        return None;
    }

    Some(masked)
}

fn mask_c_like_code(
    content: &str,
    allow_backticks: bool,
    _allow_template_strings: bool,
    allow_raw_strings: bool,
) -> Option<String> {
    let bytes = content.as_bytes();
    let mut masked = String::with_capacity(content.len());
    let mut idx = 0usize;
    let mut in_line_comment = false;
    let mut block_comment_depth = 0usize;
    let mut string_quote: Option<u8> = None;
    let mut raw_string_hashes: Option<usize> = None;

    while idx < bytes.len() {
        let b = bytes[idx];

        if in_line_comment {
            if b == b'\n' {
                in_line_comment = false;
                masked.push('\n');
            } else {
                masked.push(' ');
            }
            idx += 1;
            continue;
        }

        if block_comment_depth > 0 {
            if b == b'/' && bytes.get(idx + 1) == Some(&b'*') {
                block_comment_depth += 1;
                masked.push(' ');
                masked.push(' ');
                idx += 2;
                continue;
            }
            if b == b'*' && bytes.get(idx + 1) == Some(&b'/') {
                block_comment_depth = block_comment_depth.saturating_sub(1);
                masked.push(' ');
                masked.push(' ');
                idx += 2;
                continue;
            }
            masked.push(if b == b'\n' { '\n' } else { ' ' });
            idx += 1;
            continue;
        }

        if let Some(hash_count) = raw_string_hashes {
            if b == b'"' {
                let mut end = idx + 1;
                let mut hashes_seen = 0usize;
                while end < bytes.len() && bytes[end] == b'#' && hashes_seen < hash_count {
                    hashes_seen += 1;
                    end += 1;
                }
                if hashes_seen == hash_count {
                    for _ in idx..end {
                        masked.push(' ');
                    }
                    idx = end;
                    raw_string_hashes = None;
                    continue;
                }
            }
            masked.push(if b == b'\n' { '\n' } else { ' ' });
            idx += 1;
            continue;
        }

        if let Some(quote) = string_quote {
            if b == quote && bytes.get(idx.saturating_sub(1)) != Some(&b'\\') {
                string_quote = None;
            }
            masked.push(if b == b'\n' { '\n' } else { ' ' });
            idx += 1;
            continue;
        }

        if b == b'/' && bytes.get(idx + 1) == Some(&b'/') {
            in_line_comment = true;
            masked.push(' ');
            masked.push(' ');
            idx += 2;
            continue;
        }

        if b == b'/' && bytes.get(idx + 1) == Some(&b'*') {
            block_comment_depth = 1;
            masked.push(' ');
            masked.push(' ');
            idx += 2;
            continue;
        }

        if allow_raw_strings && b == b'r' {
            let mut probe = idx + 1;
            let mut hashes = 0usize;
            while probe < bytes.len() && bytes[probe] == b'#' {
                hashes += 1;
                probe += 1;
            }
            if bytes.get(probe) == Some(&b'"') {
                for _ in idx..=probe {
                    masked.push(' ');
                }
                idx = probe + 1;
                raw_string_hashes = Some(hashes);
                continue;
            }
        }

        if b == b'"' || b == b'\'' || (allow_backticks && b == b'`') {
            string_quote = Some(b);
            masked.push(' ');
            idx += 1;
            continue;
        }

        masked.push(b as char);
        idx += 1;
    }

    if in_line_comment
        || block_comment_depth > 0
        || string_quote.is_some()
        || raw_string_hashes.is_some()
    {
        return None;
    }

    Some(masked)
}

fn mask_python_code(content: &str) -> Option<String> {
    let bytes = content.as_bytes();
    let mut masked = String::with_capacity(content.len());
    let mut idx = 0usize;
    let mut line_comment = false;
    let mut string_delim: Option<(u8, usize)> = None;

    while idx < bytes.len() {
        let b = bytes[idx];

        if line_comment {
            if b == b'\n' {
                line_comment = false;
                masked.push('\n');
            } else {
                masked.push(' ');
            }
            idx += 1;
            continue;
        }

        if let Some((quote, len)) = string_delim {
            if matches_python_string_end(bytes, idx, quote, len) {
                for _ in 0..len {
                    masked.push(' ');
                }
                idx += len;
                string_delim = None;
                continue;
            }
            masked.push(if b == b'\n' { '\n' } else { ' ' });
            idx += 1;
            continue;
        }

        if b == b'#' {
            line_comment = true;
            masked.push(' ');
            idx += 1;
            continue;
        }

        if let Some((prefix_len, quote, len)) = starts_python_string(bytes, idx) {
            for _ in 0..(prefix_len + len) {
                masked.push(' ');
            }
            idx += prefix_len + len;
            string_delim = Some((quote, len));
            continue;
        }

        masked.push(b as char);
        idx += 1;
    }

    if line_comment || string_delim.is_some() {
        return None;
    }

    Some(masked)
}

fn starts_python_string(bytes: &[u8], idx: usize) -> Option<(usize, u8, usize)> {
    let mut probe = idx;
    while probe < bytes.len() {
        let c = bytes[probe];
        if matches!(c, b'r' | b'R' | b'u' | b'U' | b'b' | b'B' | b'f' | b'F') {
            probe += 1;
            continue;
        }
        break;
    }

    let quote = *bytes.get(probe)?;
    if quote != b'\'' && quote != b'"' {
        return None;
    }

    let delim_len = if bytes.get(probe + 1) == Some(&quote) && bytes.get(probe + 2) == Some(&quote)
    {
        3
    } else {
        1
    };
    Some((probe - idx, quote, delim_len))
}

fn matches_python_string_end(bytes: &[u8], idx: usize, quote: u8, len: usize) -> bool {
    if len == 3 {
        bytes.get(idx) == Some(&quote)
            && bytes.get(idx + 1) == Some(&quote)
            && bytes.get(idx + 2) == Some(&quote)
    } else {
        bytes.get(idx) == Some(&quote) && bytes.get(idx.saturating_sub(1)) != Some(&b'\\')
    }
}

fn push_unique_import(imports: &mut Vec<String>, module: String) {
    if !imports.iter().any(|existing| existing == &module) {
        imports.push(module);
    }
}

fn extract_python_relative_imports(statement: &str, module: &str) -> Vec<String> {
    let imported = statement
        .split_once(" import ")
        .map(|(_, names)| names)
        .unwrap_or("");
    let imported_names: Vec<String> = imported
        .split(',')
        .map(|name| name.trim())
        .filter(|name| !name.is_empty())
        .map(|name| name.split(" as ").next().unwrap_or("").trim().to_string())
        .filter(|name| !name.is_empty() && name != "*")
        .collect();

    if module.chars().all(|c| c == '.') {
        imported_names
            .into_iter()
            .map(|name| format!("{module}{name}"))
            .collect()
    } else {
        let dots = module.chars().take_while(|c| *c == '.').count();
        let remainder = &module[dots..];
        if remainder.is_empty() {
            Vec::new()
        } else {
            vec![format!(
                "{}{}",
                ".".repeat(dots),
                remainder.split('.').next().unwrap_or(remainder)
            )]
        }
    }
}

fn walk_nodes_recursive(node: Node, visitor: &mut impl FnMut(Node)) {
    visitor(node);
    for idx in 0..node.child_count() {
        if let Some(child) = node.child(idx) {
            walk_nodes_recursive(child, visitor);
        }
    }
}

fn parse_bare_import_source(statement: &str) -> Option<String> {
    let rest = statement.trim_start().strip_prefix("import ")?;
    parse_static_string_literal_with_trailing(rest)
}

fn parse_module_source_after_from(input: &str) -> Option<String> {
    parse_static_string_literal_with_trailing(input)
}

fn parse_static_call_source(input: &str, callee: &str) -> Option<String> {
    let trimmed = input.trim_start();
    let rest = trimmed.strip_prefix(callee)?.trim_start();
    let rest = rest.strip_prefix('(')?.trim_start();
    let (module, consumed) = parse_static_string_literal_prefix(rest)?;
    let trailing = rest[consumed..].trim_start();
    let trailing = trailing.strip_prefix(')')?.trim();
    if trailing.is_empty() || trailing == ";" {
        Some(module)
    } else {
        None
    }
}

fn parse_static_string_literal_with_trailing(input: &str) -> Option<String> {
    let trimmed = input.trim_start();
    let (module, consumed) = parse_static_string_literal_prefix(trimmed)?;
    let trailing = trimmed[consumed..].trim();
    if trailing.is_empty() || trailing == ";" {
        Some(module)
    } else {
        None
    }
}

fn parse_static_string_literal_prefix(input: &str) -> Option<(String, usize)> {
    let bytes = input.as_bytes();
    let quote = *bytes.first()?;
    if quote != b'\'' && quote != b'"' && quote != b'`' {
        return None;
    }

    let mut idx = 1usize;
    while idx < bytes.len() {
        if bytes[idx] == quote && bytes.get(idx.saturating_sub(1)) != Some(&b'\\') {
            let literal = &input[1..idx];
            if quote == b'`' && literal.contains("${") {
                return None;
            }
            return Some((literal.to_string(), idx + 1));
        }
        idx += 1;
    }
    None
}

fn find_quoted_literal(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut idx = 0usize;
    while idx < bytes.len() {
        let quote = bytes[idx];
        if quote == b'\'' || quote == b'"' || quote == b'`' {
            let start = idx + 1;
            let mut end = start;
            while end < bytes.len() {
                if bytes[end] == quote && bytes.get(end.saturating_sub(1)) != Some(&b'\\') {
                    let literal = &input[start..end];
                    if quote == b'`' && literal.contains("${") {
                        return None;
                    }
                    return Some(literal.to_string());
                }
                end += 1;
            }
            return None;
        }
        idx += 1;
    }
    None
}
