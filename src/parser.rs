// =============================================================================
// parser.rs — Tree-sitter tabanlı import/bağımlılık çıkarıcı
// =============================================================================
//
// Desteklenen diller ve aranan kalıplar:
//
//   Rust         → use xxx::yyy, extern crate xxx
//   TypeScript   → import ... from 'xxx', import 'xxx'
//   Python       → import xxx, from xxx import yyy
//   Go           → import "xxx", import ( "xxx" "yyy" )
//
// Her dil için tree-sitter grammar'ı kullanılarak AST oluşturulur,
// ardından ilgili düğüm türleri bulunup metin tabanlı çıkarım yapılır.
//
// Performans:
//   - tree-sitter C tabanlı GLR parser — çok hızlı (ms mertebesinde)
//   - Büyük dosyalar (>512KB) atlanır (üretilen/minified kod olabilir)
//   - Parser nesnesi her çağrıda oluşturulur (ucuz işlem)
// =============================================================================

use tracing::debug;
use tree_sitter::{Node, Parser as TsParser};

/// Desteklenen programlama dilleri
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Rust,
    TypeScript,
    Python,
    Go,
}

/// Maksimum dosya boyutu — bundan büyük dosyalar atlanır (512 KB)
const MAX_FILE_SIZE: usize = 512 * 1024;

/// Dosya uzantısından programlama dilini tespit eder.
///
/// # Dönüş
/// - `Some(Language)` — tanınan bir uzantı ise
/// - `None` — desteklenmeyen uzantı
///
/// # Desteklenen Uzantılar
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

/// Kaynak koddan import edilen modül/paket isimlerini çıkarır.
///
/// tree-sitter ile AST oluşturur, ardından dile özgü düğüm türlerini
/// bulup import edilen isimleri metin tabanlı olarak çıkarır.
///
/// # Parametreler
/// - `content`: Kaynak dosyanın UTF-8 içeriği
/// - `lang`: Dosyanın programlama dili
///
/// # Dönüş
/// Benzersiz olmayan import listesi. Aynı modül birden fazla kez dönebilir;
/// deduplication graph_builder'da yapılır.
///
/// # Güvenlik
/// - Büyük dosyalar (>512KB) boş liste döner
/// - Parse hataları sessizce atlanır (boş liste)
pub fn parse_imports(content: &str, lang: Language) -> Vec<String> {
    // Çok büyük dosyaları atla (muhtemelen üretilen kod)
    if content.len() > MAX_FILE_SIZE {
        debug!(size = content.len(), "Dosya çok büyük, atlanıyor");
        return Vec::new();
    }

    // Dile uygun tree-sitter Language nesnesini al
    let ts_lang = match lang {
        Language::Rust => tree_sitter_rust::language(),
        Language::TypeScript => tree_sitter_typescript::language_typescript(),
        Language::Python => tree_sitter_python::language(),
        Language::Go => tree_sitter_go::language(),
    };

    // Parser oluştur ve dili ayarla
    let mut parser = TsParser::new();
    if parser.set_language(&ts_lang).is_err() {
        return Vec::new();
    }

    // Kaynak kodu parse et
    let tree = match parser.parse(content, None) {
        Some(t) => t,
        None => return Vec::new(),
    };

    let source = content.as_bytes();
    let root = tree.root_node();

    // Dile özgü çıkarım fonksiyonunu çağır
    match lang {
        Language::Rust => extract_rust_imports(root, source),
        Language::TypeScript => extract_typescript_imports(root, source),
        Language::Python => extract_python_imports(root, source),
        Language::Go => extract_go_imports(root, source),
    }
}

// =============================================================================
// Rust import çıkarımı
// =============================================================================
//
// Aranan düğüm türleri:
//   use_declaration     → use std::collections::HashMap;
//   extern_crate_declaration → extern crate serde;
//
// Çıkarım: İlk path segmenti (crate adı), örneğin "std", "serde"
// self/crate/super atlanır (dahili referanslar)
// =============================================================================

/// Rust kaynak kodundan import edilen crate isimlerini çıkarır.
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
                        // İlk segment: crate adı
                        let first = path.split("::").next().unwrap_or(path);
                        // Dahili referansları atla
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
// TypeScript import çıkarımı
// =============================================================================
//
// Aranan düğüm türleri:
//   import_statement → import { x } from 'react';
//                    → import React from 'react';
//                    → import './styles.css';
//
// Çıkarım: from sonrasındaki string literal (tırnaklar çıkarılır)
// "./" ve "../" ile başlayan relative import'lar atlanır
// =============================================================================

/// TypeScript/TSX kaynak kodundan import edilen modül isimlerini çıkarır.
fn extract_typescript_imports(root: Node, source: &[u8]) -> Vec<String> {
    let mut imports = Vec::new();
    let mut cursor = root.walk();

    for child in root.children(&mut cursor) {
        if child.kind() == "import_statement" {
            // import_statement içindeki string düğümünü bul
            if let Some(module) = find_string_literal_in_node(child, source) {
                // Relative import'ları atla
                if !module.starts_with('.') {
                    imports.push(module);
                }
            }
        }
    }
    imports
}

// =============================================================================
// Python import çıkarımı
// =============================================================================
//
// Aranan düğüm türleri:
//   import_statement      → import os
//   import_from_statement → from datetime import datetime
//
// Çıkarım: Üst seviye modül adı (noktalı path'in ilk segmenti)
// Relative import'lar (from . import ...) atlanır
// =============================================================================

/// Python kaynak kodundan import edilen üst seviye modül isimlerini çıkarır.
fn extract_python_imports(root: Node, source: &[u8]) -> Vec<String> {
    let mut imports = Vec::new();
    let mut cursor = root.walk();

    for child in root.children(&mut cursor) {
        match child.kind() {
            "import_statement" => {
                // "import os" veya "import os, sys"
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
                        // Relative import'ları atla
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
// Go import çıkarımı
// =============================================================================
//
// Aranan düğüm türleri:
//   import_declaration → import "fmt"
//                      → import ( "fmt" \n "os" )
//
// Çıkarım: String literal'lardaki paket yolları (tırnaklar çıkarılır)
//   - "fmt" → "fmt"
//   - "github.com/gin-gonic/gin" → "github.com/gin-gonic/gin"
// =============================================================================

/// Go kaynak kodundan import edilen paket yollarını çıkarır.
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

/// import_declaration altındaki tüm string literal'ları toplar (recursive).
///
/// Go'da import tek satır veya parantezli çoklu form olabilir:
///   import "fmt"
///   import ( "fmt" \n "os" )
///
/// Her iki formu da desteklemek için AST'yi recursive yürürüz.
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

    // Alt düğümleri recursive olarak tara
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            collect_go_import_strings(child, source, imports);
        }
    }
}

// =============================================================================
// Yardımcı fonksiyonlar
// =============================================================================

/// Verilen düğümün alt ağacındaki ilk string literal'ı bulur.
///
/// TypeScript import'larında modül adı bir string düğümü içinde bulunur.
/// Bu fonksiyon düğümü ve çocuklarını tarar, ilk string'i döner.
fn find_string_literal_in_node(node: Node, source: &[u8]) -> Option<String> {
    // Önce doğrudan çocuklarda string ara
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i)
            && (child.kind() == "string" || child.kind() == "string_literal")
            && let Ok(text) = child.utf8_text(source)
        {
            let module = text.trim_matches(|c| c == '\'' || c == '"');
            return Some(module.to_string());
        }
    }
    // Bulunamazsa bir seviye daha derine bak
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
// Testler
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
    fn test_parse_rust_imports() {
        let source = r#"
use std::collections::HashMap;
use serde::{Serialize, Deserialize};
use crate::models::CommitInfo;
use super::utils;
extern crate anyhow;

fn main() {}
"#;
        let imports = parse_imports(source, Language::Rust);
        assert!(imports.contains(&"std".to_string()), "std olmalı");
        assert!(imports.contains(&"serde".to_string()), "serde olmalı");
        assert!(imports.contains(&"anyhow".to_string()), "anyhow olmalı");
        // crate ve super dahili — olmamalı
        assert!(!imports.contains(&"crate".to_string()), "crate olmamalı");
        assert!(!imports.contains(&"super".to_string()), "super olmamalı");
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
        let imports = parse_imports(source, Language::TypeScript);
        assert!(imports.contains(&"react".to_string()), "react olmalı");
        assert!(imports.contains(&"axios".to_string()), "axios olmalı");
        // Relative import'lar olmamalı
        assert!(
            !imports.iter().any(|i| i.starts_with('.')),
            "relative import olmamalı"
        );
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
        let imports = parse_imports(source, Language::Python);
        assert!(imports.contains(&"os".to_string()), "os olmalı");
        assert!(imports.contains(&"sys".to_string()), "sys olmalı");
        assert!(imports.contains(&"datetime".to_string()), "datetime olmalı");
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
        let imports = parse_imports(source, Language::Go);
        assert!(imports.contains(&"fmt".to_string()), "fmt olmalı");
        assert!(imports.contains(&"os".to_string()), "os olmalı");
        assert!(
            imports.iter().any(|i| i.contains("gin")),
            "gin paketi olmalı"
        );
    }

    #[test]
    fn test_empty_and_invalid_content() {
        // Boş dosya
        let imports = parse_imports("", Language::Rust);
        assert!(imports.is_empty(), "Boş dosyada import olmamalı");

        // Geçersiz syntax — tree-sitter toleranslı, yine de parse edebilir
        let imports = parse_imports("fn {{{", Language::Rust);
        assert!(imports.is_empty(), "Bozuk syntax'ta import olmamalı");
    }
}
