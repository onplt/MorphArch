# Language-Aware Parsing

MorphArch does not rely on plain regex matching. It uses a hybrid parsing
strategy that keeps scans fast while still protecting dependency accuracy.

## The Core Logic

MorphArch parses source files in two stages:

1. **Safe fast path**
   Comment-aware and string-aware scanners handle the common import forms
   quickly.
2. **AST fallback**
   When the fast path is not reliable, MorphArch falls back to Tree-sitter for
   language-aware parsing.

This design gives you:

1. **Context awareness**: imports inside comments and strings are ignored.
2. **Language specificity**: each supported language has dedicated extraction logic.
3. **Good repeated-scan performance**: unchanged blobs are cached and reused.

---

## TypeScript & JavaScript

Modern JS/TS monorepos use several import styles, and MorphArch supports the
common static forms.

### Supported Patterns

- **ES module imports**: `import {a} from './b'`
- **Dynamic imports with a static literal**: `import('./lazy')`
- **CommonJS**: `require('../legacy')`
- **TypeScript import assignment**: `import fs = require('fs')`
- **Re-exports**: `export * from './internal'`

### Important behavior

- Static literal imports are tracked.
- Template-literal imports with interpolation such as ``import(`./${name}`)``
  are treated as dynamic and are not turned into fake static dependencies.
- Relative imports are normalized into repo-local module labels.
- Third-party packages are kept as external dependency nodes when they are
  meaningful enough to show in the architecture view.

---

## Rust

Rust's module system is rich, so MorphArch focuses on import-level dependency
signals that are stable at the architectural level.

### Supported Patterns

- **External crates**: `use serde::Serialize;`
- **Internal modules**: `mod scanner;` or `use crate::db::Database;`
- **Relative imports**: `use super::utils;`

### Important behavior

- Commented-out `use` lines and string literals do not create edges.
- Relative imports are normalized against the source file path.
- The graph is package-oriented, so imports roll up to repo-local modules rather
  than acting like a compiler-level resolver.

---

## Python

Python's import system is dynamic, but most architectural drift happens at the
package level.

### Supported Patterns

- **Absolute imports**: `import my_package.models`
- **Relative imports**: `from ..utils import helper`
- **Sub-modules**: `from my_package.api import routes`

### Important behavior

- Comments and docstrings do not create false dependencies.
- Relative imports stay relative to the source package and are normalized into
  repo-local module labels when possible.

---

## Go

Go's import system is rigid enough that MorphArch can extract useful package
edges accurately.

### Supported Patterns

- **Internal imports**: `import "github.com/org/repo/pkg/auth"`
- **Alias imports**: `import auth "github.com/org/repo/pkg/security"`

---

## Performance & Caching

Parsing is CPU-intensive. To maintain high performance, MorphArch implements:

- **Subtree caching**: if a directory hasn't changed, we skip all files inside it
- **Blob cache**: file import results are cached across the scan
- **Parallel parsing**: parsing is distributed across available CPU cores
- **Safe fallbacks**: when fast parsing is uncertain, MorphArch falls back to
  AST parsing instead of emitting bad edges
