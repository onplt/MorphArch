//! Tree-sitter based import and dependency extractor.

use std::cell::RefCell;
use std::path::{Component, Path};
use tracing::debug;
use tree_sitter::{Language as TsLanguage, Node, Parser as TsParser};

/// Supported programming languages
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Rust,
    TypeScript,
    Python,
    Go,
}

const MAX_FILE_SIZE: usize = 512 * 1024;

// Thread-local parser storage to reuse parser instances across files in the same thread.
// This avoids expensive re-allocation and re-setting the language per file.
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
        "ts" | "tsx" => Some(Language::TypeScript),
        "py" => Some(Language::Python),
        "go" => Some(Language::Go),
        _ => None,
    }
}

pub fn extract_package_name(file_path: &Path) -> String {
    let components: Vec<_> = file_path
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect();

    if components.len() <= 1 {
        return file_path
            .file_stem()
            .map_or_else(|| "".to_string(), |s| s.to_string_lossy().to_string());
    }

    let dirs = &components[..components.len() - 1];

    if let Some(pos) = dirs.iter().position(|&c| c == "packages" || c == "apps") {
        if pos + 1 < dirs.len() {
            return dirs[pos + 1].to_string();
        }
    }

    const SKIP_ROOTS: &[&str] = &["src", "lib", "internal"];
    let m_start = if !dirs.is_empty() && SKIP_ROOTS.contains(&dirs[0]) {
        1
    } else {
        0
    };
    let meaningful = &dirs[m_start..];

    match meaningful.len() {
        0 => file_path
            .file_stem()
            .map_or_else(|| "".to_string(), |s| s.to_string_lossy().to_string()),
        1 => meaningful[0].to_string(),
        _ => format!("{}/{}", meaningful[0], meaningful[1]),
    }
}

pub fn parse_imports(content: &str, lang: Language, _file_path: &Path) -> Vec<String> {
    if content.len() > MAX_FILE_SIZE {
        debug!(size = content.len(), "File too large, skipping");
        return Vec::new();
    }

    let source = content.as_bytes();

    // Use thread-local parser to avoid re-allocation
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
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        match child.kind() {
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
                if let Ok(text) = child.utf8_text(source) {
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

fn extract_typescript_imports(root: Node, source: &[u8]) -> Vec<String> {
    let mut imports = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "import_statement" {
            if let Some(module) = find_string_literal_in_node(child, source) {
                if module.starts_with('.') {
                    if let Some(stem) = Path::new(&module).file_stem() {
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

fn extract_python_imports(root: Node, source: &[u8]) -> Vec<String> {
    let mut imports = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        match child.kind() {
            "import_statement" => {
                if let Ok(text) = child.utf8_text(source) {
                    if let Some(module_part) = text.trim().strip_prefix("import ") {
                        for module in module_part.split(',') {
                            let m = module.trim().split(" as ").next().unwrap_or("").trim();
                            let top = m.split('.').next().unwrap_or(m);
                            if !top.is_empty() {
                                imports.push(top.to_string());
                            }
                        }
                    }
                }
            }
            "import_from_statement" => {
                if let Ok(text) = child.utf8_text(source) {
                    if let Some(rest) = text.trim().strip_prefix("from ") {
                        let m_path = rest.split_whitespace().next().unwrap_or("");
                        if !m_path.starts_with('.') {
                            let top = m_path.split('.').next().unwrap_or(m_path);
                            if !top.is_empty() {
                                imports.push(top.to_string());
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

fn find_string_literal_in_node(node: Node, source: &[u8]) -> Option<String> {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if (child.kind() == "string" || child.kind() == "string_literal")
                && let Ok(text) = child.utf8_text(source)
            {
                return Some(text.trim_matches(|c| c == '\'' || c == '"').to_string());
            }
        }
    }
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            for j in 0..child.child_count() {
                if let Some(gc) = child.child(j) {
                    if (gc.kind() == "string"
                        || gc.kind() == "string_literal"
                        || gc.kind() == "string_fragment")
                        && let Ok(text) = gc.utf8_text(source)
                    {
                        let m = text.trim_matches(|c| c == '\'' || c == '"');
                        if !m.is_empty() {
                            return Some(m.to_string());
                        }
                    }
                }
            }
        }
    }
    None
}
