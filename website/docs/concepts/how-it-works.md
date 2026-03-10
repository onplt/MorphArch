# How it Works: The Pipeline

MorphArch acts as a high-performance engine that transforms raw source code and Git metadata into a structured architectural model.

## The 5-Stage Pipeline

### 0. Configuration (Optional)
MorphArch loads project settings from `morpharch.toml` at the repository root, if present. This controls which paths to ignore, how scoring weights are distributed, and what boundary rules to enforce. If no config file exists, sensible defaults are used.

### 1. Discovery (The Scanner)
Powered by the `gix` (gitoxide) library, MorphArch performs an incremental walk of your Git history.
- It identifies changed files between commits.
- It detects monorepo workspace configurations (Nx, Cargo, etc.).
- Paths matching [ignore rules](../guides/configuration#ignore-rules) are skipped at the tree-walk level, before any file I/O occurs.

### 2. Analysis (AST Parsing)
Instead of regex, we use **Tree-sitter** to build a full Abstract Syntax Tree of your source files.
- **Language Intelligence**: Supports Rust, TS, Python, and Go.
- **Context Awareness**: Distinguishes between a real `import` and a string literal in a comment.

### 3. Synthesis (The Graph)
Extracted imports are mapped to workspace packages.
- **Nodes**: Represent packages/modules.
- **Edges**: Represent dependency relationships.
- **Weights**: Scale based on the number of individual file-level imports between packages.

### 4. Evaluation (Scoring)
The graph is passed through the scoring engine, configured by `morpharch.toml`:
- **Debt Calculation**: A 6-component scale-aware algorithm (Cycle, Layering, Hub, Coupling, Cognitive, Instability) computes the absolute health score using [configurable weights and thresholds](../guides/configuration#scoring-weights).
- **Boundary Rules**: Explicit [architectural boundaries](../guides/configuration#boundary-rules) are checked alongside automatic topological layering analysis.
- **Physics Layout**: Generates initial coordinates for the **Verlet Physics** engine used in the TUI.

---

## Technical Stack

| Component | Technology |
|-----------|------------|
| **Runtime** | Rust (Tokio for Async) |
| **Git Engine** | `gix` (Pure-Rust Git implementation) |
| **Parsing** | `tree-sitter` + Language Grammars |
| **Graph Theory** | `petgraph` |
| **TUI Framework** | `ratatui` + `crossterm` |
| **Persistence** | SQLite (via `rusqlite`) |

## Incremental Performance
Scanning 1,000 commits from scratch is slow. MorphArch solves this by:
1.  **Subtree Caching**: If a directory hash hasn't changed, we skip the entire folder.
2.  **Blob LRU Cache**: We store the results of individual file parses in a 50,000-entry cache.
3.  **Parallelism**: Every CPU core is utilized via `Rayon` during the parsing stage.
