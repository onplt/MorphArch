# How It Works

MorphArch turns source code and Git history into a repository-level dependency
model that you can inspect from the terminal.

The basic idea is simple:

- raw dependency graphs are useful for debugging
- grouped views are better for understanding large systems

MorphArch builds both, but exposes them at different levels of detail.

---

## The Pipeline

### 1. Configuration

MorphArch loads `morpharch.toml` if present.

This can change:

- ignore paths and presets
- scoring weights and thresholds
- boundary rules
- clustering strategy
- semantic families, rules, and clustering constraints
- presentation aliases, kind mode, and color mode

If no config file exists, defaults are used.

### 2. Repository Discovery

MorphArch walks Git history using `gix`.

During discovery it:

- follows the repository's first-parent history
- enumerates commits and file changes
- detects workspace structure
- skips ignored subtrees early

This keeps repeated scans and history replay deterministic and practical.

### 3. Parsing

MorphArch uses language-aware import extraction.

In practice that means:

- safe fast paths that ignore comments and strings
- AST fallback when the fast path is not reliable
- accurate dependency edges for supported languages without plain regex matching

Supported languages include Rust, TypeScript, JavaScript, Python, and Go.

### 4. Dependency Graph Synthesis

Parsed imports are mapped into a repository-level dependency graph.

- nodes represent packages or modules
- edges represent dependency relationships
- weights represent how many concrete imports sit behind a higher-level edge

This graph becomes the basis for scoring, grouping, and inspect mode.

### 5. Architecture Evaluation

MorphArch computes health with six debt dimensions:

- cycle
- layering
- hub
- coupling
- cognitive
- instability

It also applies:

- explicit boundary rules
- scale-aware expectations
- hotspot and blast radius analysis

### 6. Semantic Grouping and Clustering

This is what keeps the TUI usable on large repositories.

MorphArch groups the raw graph into clusters using a hybrid approach:

- semantic grouping from names and paths
- structural grouping when naming is weak
- quality passes that split overly generic fallback clusters
- optional collapsing of external dependency families

Users can override semantic families, rules, hard grouping constraints, and
presentation labels through `morpharch.toml`.

### 7. Persistence and Replay

MorphArch stores scan data in a repo-scoped local cache.

That cache includes:

- snapshot frames for each scanned commit
- checkpoints for efficient reconstruction
- saved scan state for incremental updates

This is what makes repeated scans and timeline replay practical without
starting from scratch every time.

### 8. Presentation Surfaces

The TUI is built from three semantic surfaces.

#### `Map`

Cluster-level repository overview.

- major subsystems
- strongest links
- readable repo shape

#### `Cluster details`

Subsystem detail view.

- diagnosis
- top members or dependencies
- incoming/outgoing link pressure
- selected member or dependency lens

#### `Inspect`

Focused debug view.

- selected member centered
- one-hop inbound/outbound graph context
- raw graph rendering reserved for debugging

This is why MorphArch does not need to keep the full raw graph on screen all
the time.

---

## Why Raw Graphs Are Not the Default

Large node-link graphs become noisy quickly, especially in a terminal.

MorphArch avoids this by:

- starting with clusters instead of individual modules
- summarizing link pressure before drawing it
- using text-first cluster views when geometry would be noisy
- keeping raw graph rendering for inspect mode

That tradeoff is deliberate. The goal is to make repository structure easier to
review without removing graph-level debugging when it is needed.

---

## Technical Stack

| Component | Technology |
| --- | --- |
| Runtime | Rust |
| Git engine | `gix` |
| Parsing | fast paths + `tree-sitter` fallback |
| Graph algorithms | `petgraph` |
| TUI | `ratatui` + `crossterm` |
| Persistence | SQLite via `rusqlite` |

---

## Performance Characteristics

MorphArch is optimized for repeated scans and historical navigation.

Important techniques:

- subtree-level skipping for unchanged directories
- blob parse caching
- parallel parsing and graph processing
- repo-scoped SQLite checkpoint + delta storage for fast replay
- saved scan state for incremental updates

That is what makes timeline scrubbing and repeated `watch` sessions responsive.
