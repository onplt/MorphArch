# AST-Native Parsing

MorphArch distinguishes itself by performing deep **Abstract Syntax Tree (AST)** parsing instead of relying on simple text searches. This page details how we handle different programming languages.

## The Core Logic: Tree-sitter

We use **Tree-sitter**, an incremental parsing library, to build a full syntax tree of every source file. This allows us to:
1.  **Differentiate Context**: We know the difference between an `import` statement and a string inside a comment.
2.  **Handle Aliases**: We correctly resolve renamed imports (e.g., `import { X as Y }`).
3.  **Language Specificity**: Each language has a dedicated grammar and resolution strategy.

---

## 🟦 TypeScript & JavaScript

Modern frontend and backend JS/TS monorepos often have complex import structures.

### Supported Patterns
- **ES6 Imports**: `import { a } from './b'`
- **Dynamic Imports**: `import('./lazy')`
- **CommonJS**: `require('../legacy')`
- **Re-exports**: `export * from './internal'`

### Path Mapping & Aliases
MorphArch understands `paths` defined in `tsconfig.json` or `jsconfig.json`.
- **Scenario**: If you use `@core/auth` which points to `packages/auth/src/index.ts`, MorphArch resolves this alias to the correct package boundary.
- **Resolution**: We recursively walk up the directory tree to find the nearest config file and apply the mapping rules.

---

## 🦀 Rust

Rust's module system is powerful but non-trivial to parse without a full compiler.

### Supported Patterns
- **External Crates**: `use serde::Serialize;`
- **Internal Modules**: `mod scanner;` or `use crate::db::Database;`
- **Relative Imports**: `use super::utils;`

### Resolution Strategy
- **Workspace Awareness**: MorphArch reads your root `Cargo.toml` to identify workspace members.
- **Dependency Mapping**: If `app-a` has a `[dependencies]` entry for `lib-b`, any `use lib_b::...` statement is correctly mapped as an edge in the graph.

---

## 🐍 Python

Python's import system is dynamic, but most architectural drift happens at the package level.

### Supported Patterns
- **Absolute Imports**: `import my_package.models`
- **Relative Imports**: `from ..utils import helper`
- **Sub-modules**: `from my_package.api import routes`

### Resolution Strategy
MorphArch identifies the Python package boundaries by looking for `__init__.py` files or `pyproject.toml` definitions.

---

## 🐹 Go

Go enforces a very strict package structure which makes analysis highly accurate.

### Supported Patterns
- **Internal Imports**: `import "github.com/org/repo/pkg/auth"`
- **Alias Imports**: `import auth "github.com/org/repo/pkg/security"`

### Resolution Strategy
- **Go Mod Support**: MorphArch parses `go.mod` to determine the module's root name.
- **Boundary Detection**: Any import path that starts with the module root name is identified as an internal dependency edge.

---

## Performance & Caching

AST parsing is CPU-intensive. To maintain high performance, MorphArch implements:
- **Subtree Caching**: If a directory hasn't changed (based on Git tree hash), we skip all files inside it.
- **Blob Cache**: Individual file AST results are stored in an LRU cache (default size: 50,000 files).
- **Rayon Parallelism**: Parsing is distributed across all available CPU cores.
