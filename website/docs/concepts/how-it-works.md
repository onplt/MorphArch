# How it Works: The Pipeline

MorphArch acts as a high-performance engine that transforms raw source code and Git metadata into a structured architectural model.

## The 4-Stage Pipeline

### 1. Discovery (The Scanner)
Powered by the `gix` (gitoxide) library, MorphArch performs an incremental walk of your Git history. 
- It identifies changed files between commits.
- It detects monorepo workspace configurations (Nx, Cargo, etc.).

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
The graph is passed through our mathematical evaluation engine:
- **Debt Calculation**: A 6-component scale-aware algorithm (Cycle, Layering, Hub, Coupling, Cognitive, Instability) computes the absolute health score.
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
