// =============================================================================
// parser.rs — Tree-sitter based import/dependency extractor
// =============================================================================
//
// Supported languages and patterns:
//
//   Rust         → use xxx::yyy, extern crate xxx
//   TypeScript   → import ... from 'xxx', import 'xxx'
//   Python       → import xxx, from xxx import yyy
//   Go           → import "xxx", import ( "xxx" "yyy" )
//
// For each language, a tree-sitter grammar builds an AST, then relevant
// node types are found and text-based extraction is performed.
//
// Performance:
//   - tree-sitter is C-based GLR parser — very fast (ms-level)
//   - Large files (>512KB) are skipped (likely generated/minified code)
//   - Parser objects are created per call (cheap operation)
// =============================================================================

use std::path::Path;
use tracing::debug;
use tree_sitter::{Node, Parser as TsParser};

/// Supported programming languages
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Rust,
    TypeScript,
    Python,
    Go,
}

/// Maximum file size — files larger than this are skipped (512 KB)
const MAX_FILE_SIZE: usize = 512 * 1024;

/// Detects the programming language from a file extension.
///
/// # Returns
/// - `Some(Language)` — recognized extension
/// - `None` — unsupported extension
///
/// # Supported Extensions
/// - `.rs` → Rust
/// - `.ts`, `.tsx` → TypeScript
/// - `.py` → Python
/// - `.go` → Go
pub fn detect_language(path: &str) -> Option<Language> {
    let ext = path.rsplit('.').next()?;
    match ext {
        "rs" => Some(Language::Rust),
        "ts" | "tsx" => Some(Language::TypeScript),
        "py" => Some(Language::Python),
        "go" => Some(Language::Go),
        _ => None,
    }
}

/// Extracts a **directory-level** package name from a file path.
///
/// Instead of returning individual file stems (which creates thousands of
/// unique nodes for large monorepos), this function groups files by their
/// first two meaningful directory levels.
///
/// # Logic
/// 1. Monorepo patterns: `packages/X/...` or `apps/X/...` → returns `X`
/// 2. Skip common root dirs (`src`, `lib`, `internal`)
/// 3. Take first 2 meaningful directory components → `dir1/dir2`
/// 4. Fallback → file stem (for root-level files)
///
/// # Examples
/// - "packages/core/lib.rs" → "core"
/// - "apps/web/src/main.ts" → "web"
/// - "src/commands/mod.rs" → "commands"
/// - "src/main.rs" → "main"
/// - "ext/node/polyfills/path.ts" → "ext/node"
/// - "cli/tools/run.ts" → "cli/tools"
/// - "runtime/ops/fs.rs" → "runtime/ops"
pub fn extract_package_name(file_path: &Path) -> String {
    let path_str = file_path.to_string_lossy().replace('\\', "/");
    let components: Vec<&str> = path_str
        .split('/')
        .filter(|c| !c.is_empty())
        .collect();

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
    if let Some(pos) = dirs
        .iter()
        .position(|c| *c == "packages" || *c == "apps")
    {
        if pos + 1 < dirs.len() {
            return dirs[pos + 1].to_string();
        }
    }

    // Skip common meaningless root directories
    const SKIP_ROOTS: &[&str] = &["src", "lib", "internal"];
    let meaningful_start = if !dirs.is_empty() && SKIP_ROOTS.contains(&dirs[0]) {
        1
    } else {
        0
    };

    let meaningful = &dirs[meaningful_start..];

    match meaningful.len() {
        0 => {
            // All dirs were skipped (e.g. "src/main.rs") → use file stem
            file_path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
        }
        1 => meaningful[0].to_string(),
        _ => format!("{}/{}", meaningful[0], meaningful[1]),
    }
}

/// Extracts imported module/package names from source code.
///
/// Uses tree-sitter to build an AST, then extracts import names from
/// language-specific node types.
///
/// # Parameters
/// - `content`: UTF-8 source file content
/// - `lang`: The file's programming language
/// - `file_path`: Path of the file being processed (for package name extraction)
///
/// # Returns
/// Non-unique import list. The same module may appear multiple times;
/// deduplication is done in graph_builder.
///
/// # Safety
/// - Large files (>512KB) return empty list
/// - Parse errors are silently skipped (empty list)
pub fn parse_imports(content: &str, lang: Language, file_path: &Path) -> Vec<String> {
    // Skip very large files (likely generated code)
    if content.len() > MAX_FILE_SIZE {
        debug!(size = content.len(), "File too large, skipping");
        return Vec::new();
    }

    let package_name = extract_package_name(file_path);
    debug!(
        "Parsed file: {:?} → package: {}",
        file_path, package_name
    );

    // Get the tree-sitter Language object for the detected language
    let ts_lang = match lang {
        Language::Rust => tree_sitter_rust::language(),
        Language::TypeScript => tree_sitter_typescript::language_typescript(),
        Language::Python => tree_sitter_python::language(),
        Language::Go => tree_sitter_go::language(),
    };

    // Create parser and set language
    let mut parser = TsParser::new();
    if parser.set_language(&ts_lang).is_err() {
        return Vec::new();
    }

    // Parse source code
    let tree = match parser.parse(content, None) {
        Some(t) => t,
        None => return Vec::new(),
    };

    let source = content.as_bytes();
    let root = tree.root_node();

    // Call language-specific extraction function
    match lang {
        Language::Rust => extract_rust_imports(root, source),
        Language::TypeScript => extract_typescript_imports(root, source),
        Language::Python => extract_python_imports(root, source),
        Language::Go => extract_go_imports(root, source),
    }
}

// =============================================================================
// Rust import extraction
// =============================================================================
//
// Node types searched:
//   use_declaration     → use std::collections::HashMap;
//   extern_crate_declaration → extern crate serde;
//
// Extraction: First path segment (crate name), e.g. "std", "serde"
// self/crate/super are skipped (internal references)
// =============================================================================

/// Extracts imported crate names from Rust source code.
fn extract_rust_imports(root: Node, source: &[u8]) -> Vec<String> {
    let mut imports = Vec::new();
    let mut cursor = root.walk();

    for child in root.children(&mut cursor) {
        match child.kind() {
            "use_declaration" => {
                if let Ok(text) = child.utf8_text(source) {
                    // "use std::collections::HashMap;" → "std"
                    let text = text.trim();
                    if let Some(path) = text.strip_prefix("use ") {
                        let path = path.trim_end_matches(';').trim();
                        // First segment: crate name
                        let first = path.split("::").next().unwrap_or(path);
                        // Skip internal references
                        if !matches!(first, "self" | "crate" | "super") {
                            imports.push(first.to_string());
                        }
                    }
                }
            }
            "extern_crate_declaration" => {
                if let Ok(text) = child.utf8_text(source) {
                    // "extern crate serde;" → "serde"
                    if let Some(name) = text.strip_prefix("extern crate ") {
                        let name = name.trim_end_matches(';').trim();
                        if !name.is_empty() {
                            imports.push(name.to_string());
                        }
                    }
                }
            }
            _ => {}
        }
    }
    imports
}

// =============================================================================
// TypeScript import extraction
// =============================================================================
//
// Node types searched:
//   import_statement → import { x } from 'react';
//                    → import React from 'react';
//                    → import './styles.css';
//
// Extraction: string literal after `from` (quotes removed)
// Relative imports "../core" → resolved to "core"
// =============================================================================

/// Extracts imported module names from TypeScript/TSX source code.
fn extract_typescript_imports(root: Node, source: &[u8]) -> Vec<String> {
    let mut imports = Vec::new();
    let mut cursor = root.walk();

    for child in root.children(&mut cursor) {
        if child.kind() == "import_statement" {
            // Find the string node inside import_statement
            if let Some(module) = find_string_literal_in_node(child, source) {
                if module.starts_with('.') {
                    // Relative path resolution: ../core → "core"
                    let path = Path::new(&module);
                    if let Some(stem) = path.file_stem() {
                        imports.push(stem.to_string_lossy().to_string());
                    }
                } else {
                    imports.push(module);
                }
            }
        }
    }
    imports
}

// =============================================================================
// Python import extraction
// =============================================================================
//
// Node types searched:
//   import_statement      → import os
//   import_from_statement → from datetime import datetime
//
// Extraction: Top-level module name (first segment of dotted path)
// Relative imports (from . import ...) are skipped
// =============================================================================

/// Extracts imported top-level module names from Python source code.
fn extract_python_imports(root: Node, source: &[u8]) -> Vec<String> {
    let mut imports = Vec::new();
    let mut cursor = root.walk();

    for child in root.children(&mut cursor) {
        match child.kind() {
            "import_statement" => {
                // "import os" or "import os, sys"
                if let Ok(text) = child.utf8_text(source) {
                    let text = text.trim();
                    if let Some(module_part) = text.strip_prefix("import ") {
                        for module in module_part.split(',') {
                            let module = module.trim().split(" as ").next().unwrap_or("").trim();
                            let top_level = module.split('.').next().unwrap_or(module);
                            if !top_level.is_empty() {
                                imports.push(top_level.to_string());
                            }
                        }
                    }
                }
            }
            "import_from_statement" => {
                // "from os.path import join" → "os"
                if let Ok(text) = child.utf8_text(source) {
                    let text = text.trim();
                    if let Some(rest) = text.strip_prefix("from ") {
                        let module_path = rest.split_whitespace().next().unwrap_or("");
                        // Skip relative imports
                        if !module_path.starts_with('.') {
                            let top_level = module_path.split('.').next().unwrap_or(module_path);
                            if !top_level.is_empty() {
                                imports.push(top_level.to_string());
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    imports
}

// =============================================================================
// Go import extraction
// =============================================================================
//
// Node types searched:
//   import_declaration → import "fmt"
//                      → import ( "fmt" \n "os" )
//
// Extraction: Package paths from string literals (quotes removed)
//   - "fmt" → "fmt"
//   - "github.com/gin-gonic/gin" → "github.com/gin-gonic/gin"
// =============================================================================

/// Extracts imported package paths from Go source code.
fn extract_go_imports(root: Node, source: &[u8]) -> Vec<String> {
    let mut imports = Vec::new();
    let mut cursor = root.walk();

    for child in root.children(&mut cursor) {
        if child.kind() == "import_declaration" {
            collect_go_import_strings(child, source, &mut imports);
        }
    }
    imports
}

/// Collects all string literals under an import_declaration (recursive).
///
/// Go imports can be single-line or multi-line parenthesized:
///   import "fmt"
///   import ( "fmt" \n "os" )
///
/// We walk the AST recursively to support both forms.
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

    // Recursively scan child nodes
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            collect_go_import_strings(child, source, imports);
        }
    }
}

// =============================================================================
// Helper functions
// =============================================================================

/// Finds the first string literal in a node's subtree.
///
/// In TypeScript imports, the module name is inside a string node.
/// This function scans the node and its children, returning the first string found.
fn find_string_literal_in_node(node: Node, source: &[u8]) -> Option<String> {
    // First check direct children for a string
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i)
            && (child.kind() == "string" || child.kind() == "string_literal")
            && let Ok(text) = child.utf8_text(source)
        {
            let module = text.trim_matches(|c| c == '\'' || c == '"');
            return Some(module.to_string());
        }
    }
    // If not found, look one level deeper
    for i in 0..node.child_count() {
        let Some(child) = node.child(i) else {
            continue;
        };
        for j in 0..child.child_count() {
            if let Some(grandchild) = child.child(j)
                && (grandchild.kind() == "string"
                    || grandchild.kind() == "string_literal"
                    || grandchild.kind() == "string_fragment")
                && let Ok(text) = grandchild.utf8_text(source)
            {
                let module = text.trim_matches(|c| c == '\'' || c == '"');
                if !module.is_empty() {
                    return Some(module.to_string());
                }
            }
        }
    }
    None
}

// =============================================================================
// Tests
// =============================================================================
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_language() {
        assert_eq!(detect_language("src/main.rs"), Some(Language::Rust));
        assert_eq!(detect_language("app/index.ts"), Some(Language::TypeScript));
        assert_eq!(detect_language("component.tsx"), Some(Language::TypeScript));
        assert_eq!(detect_language("script.py"), Some(Language::Python));
        assert_eq!(detect_language("main.go"), Some(Language::Go));
        assert_eq!(detect_language("style.css"), None);
        assert_eq!(detect_language("README.md"), None);
    }

    #[test]
    fn test_extract_package_name() {
        assert_eq!(
            extract_package_name(Path::new("packages/core/lib.rs")),
            "core"
        );
        assert_eq!(
            extract_package_name(Path::new("apps/web/src/main.ts")),
            "web"
        );
        assert_eq!(extract_package_name(Path::new("src/main.rs")), "main");
    }

    #[test]
    fn test_extract_package_name_mod_files() {
        // mod.rs should use parent directory name (directory-level grouping)
        assert_eq!(
            extract_package_name(Path::new("src/commands/mod.rs")),
            "commands"
        );
        assert_eq!(
            extract_package_name(Path::new("src/tui/mod.rs")),
            "tui"
        );
        // index.ts should use parent directory name
        assert_eq!(
            extract_package_name(Path::new("packages/ui/src/index.ts")),
            "ui" // monorepo pattern takes priority
        );
        // __init__.py should use parent directory name
        assert_eq!(
            extract_package_name(Path::new("utils/__init__.py")),
            "utils"
        );
        // lib.rs in src/ should fall back to stem since parent is "src"
        assert_eq!(
            extract_package_name(Path::new("src/lib.rs")),
            "lib"
        );
    }

    #[test]
    fn test_extract_package_name_directory_grouping() {
        // Deep paths → first 2 meaningful directory levels
        assert_eq!(
            extract_package_name(Path::new("ext/node/polyfills/path.ts")),
            "ext/node"
        );
        assert_eq!(
            extract_package_name(Path::new("cli/tools/run.ts")),
            "cli/tools"
        );
        assert_eq!(
            extract_package_name(Path::new("runtime/ops/fs.rs")),
            "runtime/ops"
        );
        // src/ is skipped as meaningless root
        assert_eq!(
            extract_package_name(Path::new("src/commands/scan.rs")),
            "commands"
        );
        assert_eq!(
            extract_package_name(Path::new("src/tui/graph_renderer.rs")),
            "tui"
        );
        // Deep path under src/ → 2 levels after skipping src
        assert_eq!(
            extract_package_name(Path::new("src/commands/watch/handler.rs")),
            "commands/watch"
        );
        // Root-level file → file stem
        assert_eq!(
            extract_package_name(Path::new("main.rs")),
            "main"
        );
        // Single directory → just that dir
        assert_eq!(
            extract_package_name(Path::new("cli/main.ts")),
            "cli"
        );
    }

    #[test]
    fn test_parse_rust_imports() {
        let source = r#"
use std::collections::HashMap;
use serde::{Serialize, Deserialize};
use crate::models::CommitInfo;
use super::utils;
extern crate anyhow;

fn main() {}
"#;
        let imports = parse_imports(source, Language::Rust, Path::new("src/main.rs"));
        assert!(imports.contains(&"std".to_string()), "should contain std");
        assert!(imports.contains(&"serde".to_string()), "should contain serde");
        assert!(imports.contains(&"anyhow".to_string()), "should contain anyhow");
        // crate and super are internal — should not be present
        assert!(!imports.contains(&"crate".to_string()), "should not contain crate");
        assert!(!imports.contains(&"super".to_string()), "should not contain super");
    }

    #[test]
    fn test_parse_typescript_imports() {
        let source = r#"
import { useState } from 'react';
import axios from 'axios';
import './styles.css';
import { helper } from '../utils';

const x = 1;
"#;
        let imports = parse_imports(source, Language::TypeScript, Path::new("src/index.ts"));
        assert!(imports.contains(&"react".to_string()), "should contain react");
        assert!(imports.contains(&"axios".to_string()), "should contain axios");
        // Relative imports should be resolved
        assert!(imports.contains(&"utils".to_string()), "should contain utils");
    }

    #[test]
    fn test_parse_python_imports() {
        let source = r#"
import os
import sys
from datetime import datetime
from os.path import join
from . import utils

def main():
    pass
"#;
        let imports = parse_imports(source, Language::Python, Path::new("main.py"));
        assert!(imports.contains(&"os".to_string()), "should contain os");
        assert!(imports.contains(&"sys".to_string()), "should contain sys");
        assert!(imports.contains(&"datetime".to_string()), "should contain datetime");
    }

    #[test]
    fn test_parse_go_imports() {
        let source = r#"
package main

import (
    "fmt"
    "os"
    "github.com/gin-gonic/gin"
)

func main() {}
"#;
        let imports = parse_imports(source, Language::Go, Path::new("main.go"));
        assert!(imports.contains(&"fmt".to_string()), "should contain fmt");
        assert!(imports.contains(&"os".to_string()), "should contain os");
        assert!(
            imports.iter().any(|i| i.contains("gin")),
            "should contain gin package"
        );
    }

    #[test]
    fn test_empty_and_invalid_content() {
        // Empty file
        let imports = parse_imports("", Language::Rust, Path::new("lib.rs"));
        assert!(imports.is_empty(), "empty file should have no imports");

        // Invalid syntax — tree-sitter is tolerant, still parses
        let imports = parse_imports("fn {{{", Language::Rust, Path::new("lib.rs"));
        assert!(imports.is_empty(), "broken syntax should have no imports");
    }
}
