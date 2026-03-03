
<p align="center">
  <img src="assets/logo.svg" alt="morpharch animated logo" width="600">
</p>

<p align="center">
  <strong>Monorepo architecture drift visualizer with animated TUI.</strong>
</p>

<p align="center">
  <a href="https://crates.io/crates/morpharch"><img src="https://img.shields.io/crates/v/morpharch" alt="Crates.io"></a>
  <a href="https://docs.rs/morpharch"><img src="https://img.shields.io/docsrs/morpharch" alt="docs.rs"></a>
  <a href="https://github.com/onplt/morpharch/actions/workflows/ci.yml"><img src="https://github.com/onplt/morpharch/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/onplt/morpharch#license"><img src="https://img.shields.io/crates/l/morpharch" alt="License"></a>
</p>

MorphArch scans monorepo Git history, builds per-commit dependency graphs using
tree-sitter AST parsing, calculates architecture drift scores, and renders the
results as an animated force-directed graph in your terminal.

It supports Nx, Turborepo, pnpm workspaces, and Cargo workspaces out of the
box, with language-level import extraction for Rust, TypeScript, Python, and Go.

---

## Features

- **Git history scanning** -- walk commit history with gix, extract file trees,
  and detect workspace configurations automatically.
- **Tree-sitter AST parsing** -- extract real import/dependency edges from
  source files in Rust, TypeScript, Python, and Go.
- **Architecture drift scoring** -- quantify structural health on a 0--100
  scale using fan-in/out deltas, cycle detection (Kosaraju SCC), boundary
  violation analysis, and cognitive complexity metrics.
- **Animated TUI** -- Verlet physics force-directed graph layout rendered with
  ratatui and crossterm, featuring a timeline slider, drift insight panel, and
  Catppuccin Mocha color theme.
- **Incremental scanning** -- subtree-cached tree walks (`O(changed_dirs)`)
  and an LRU blob import cache (50K entries) deliver 5--20x speedups on
  subsequent runs.
- **Parallel parsing** -- rayon-powered data-parallel import extraction across
  all workspace packages.
- **Mouse interaction** -- click and drag graph nodes to rearrange the layout
  in real time.
- **Search filtering** -- press `/` in the TUI to filter nodes by name.
- **SQLite persistence** -- all scan data is stored in
  `~/.morpharch/morpharch.db` for instant replay and historical analysis.
- **Cross-platform** -- runs on Linux, macOS, and Windows.

---

## Installation

### From crates.io

```bash
cargo install morpharch
```

### From source

```bash
git clone https://github.com/onplt/morpharch.git
cd morpharch
cargo build --release
# Binary is at target/release/morpharch
```

> **Note:** SQLite is bundled via the `rusqlite` `bundled` feature, so no
> system SQLite library is required.

---

## Quick Start

```bash
# Scan a monorepo and view drift scores
morpharch scan /path/to/monorepo

# Scan and launch the animated TUI
morpharch watch /path/to/monorepo

# Analyze drift for the current HEAD commit
morpharch analyze

# View the drift trend over recent commits
morpharch list-drift
```

---

## Usage

### `morpharch scan <path>`

Scan a Git repository: walk commit history, build per-commit dependency graphs,
calculate drift scores, and persist everything to the SQLite database.

```bash
# Scan the current directory (all commits)
morpharch scan .

# Scan a specific repo, limit to last 100 commits
morpharch scan /path/to/repo -n 100
```

| Flag | Description |
|------|-------------|
| `-n, --max-commits <N>` | Maximum commits to scan. `0` (default) means unlimited. |

### `morpharch watch <path>`

Perform a scan and then launch the animated TUI. The TUI displays a
force-directed graph of the dependency structure, a timeline slider for
navigating commit history, and a drift insight panel with per-commit metrics.

```bash
# Watch the current directory
morpharch watch .

# Watch with limited scan depth and timeline snapshots
morpharch watch /path/to/repo -n 200 -s 100
```

| Flag | Description |
|------|-------------|
| `-n, --max-commits <N>` | Maximum commits to scan. `0` (default) means unlimited. |
| `-s, --max-snapshots <N>` | Maximum graph snapshots loaded into the TUI timeline. Default: `200`. When the database contains more, snapshots are sampled at even intervals so the timeline covers the full history. |

### `morpharch analyze [commit]`

Display a detailed drift report for a specific commit, including the drift
score, sub-metrics, boundary violations, circular dependencies, and improvement
recommendations.

```bash
# Analyze HEAD
morpharch analyze

# Analyze a specific commit
morpharch analyze main~5

# Analyze with explicit repo path
morpharch analyze abc1234 -p /path/to/repo
```

### `morpharch list-graphs`

List the 10 most recent dependency graph snapshots stored in the database.

```bash
morpharch list-graphs
```

### `morpharch list-drift`

Display the drift score trend for the last 20 commits as a table, including
node/edge counts and delta changes relative to the previous commit.

```bash
morpharch list-drift
```

---

## TUI Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `j` / `Down` | Navigate to the next (older) commit |
| `k` / `Up` | Navigate to the previous (newer) commit |
| `p` / `Space` | Play / pause auto-play through the timeline |
| `r` | Reheat the graph (re-energize Verlet temperature) |
| `/` | Enter search mode to filter nodes by name |
| `Esc` | Exit search mode, or quit the TUI |
| `q` | Quit the TUI |

### Mouse Controls

| Action | Effect |
|--------|--------|
| Click + Drag | Move a graph node (pinned during drag, released on drop) |

---

## Drift Score

MorphArch assigns each commit a drift score between 0 and 100. Lower scores
indicate healthier architecture; higher scores indicate growing structural
problems.

| Range | Label |
|-------|-------|
| 0--30 | Healthy |
| 31--50 | Stable |
| 51--70 | Drifting |
| 71--100 | Critical |

The score is computed from five sub-metrics:

- **Fan-in / fan-out delta** -- change in the maximum incoming and outgoing
  edge counts across all nodes compared to the previous commit.
- **Cyclic dependencies** -- number of strongly connected components with more
  than one node, detected via Kosaraju's algorithm.
- **Boundary violations** -- dependencies that cross architectural layer
  boundaries (for example, a shared library importing from an application
  package).
- **Cognitive complexity** -- a proxy metric based on the edge-to-node ratio
  and cycle count.

A baseline score of 50 is assigned to the first scanned commit. Subsequent
commits are scored relative to their predecessor: removing dependencies and
resolving cycles pulls the score down, while adding new dependencies, cycles,
or boundary violations pushes it up.

---

## Architecture

MorphArch is structured as 20 Rust source files across four logical layers:

```
src/
  main.rs              Entry point and CLI dispatch
  cli.rs               Clap derive definitions for all subcommands
  config.rs            Configuration management and default paths
  models.rs            Core data structures (commits, graphs, scores, deltas)
  utils.rs             Logging initialization and error formatting

  git_scanner.rs       Git commit walking and tree diffing via gix
  parser.rs            Tree-sitter import extraction (Rust, TS, Python, Go)
  graph_builder.rs     petgraph directed graph construction from parsed edges
  scoring.rs           Drift score engine with SCC, fan metrics, boundaries
  db.rs                SQLite layer for persistence (~/.morpharch/morpharch.db)

  commands/
    mod.rs             Command module re-exports
    scan.rs            `scan` subcommand implementation
    watch.rs           `watch` subcommand: scan then launch TUI
    analyze.rs         `analyze` subcommand: per-commit drift report

  tui/
    mod.rs             TUI module re-exports
    app.rs             Main TUI application state, event loop, render loop
    graph_renderer.rs  Verlet physics engine and ratatui Canvas rendering
    timeline.rs        Commit timeline slider widget
    insight_panel.rs   Drift metrics and recommendations panel
    widgets.rs         Shared widget helpers
```

### Key dependencies

| Crate | Purpose |
|-------|---------|
| `gix` | Pure-Rust Git operations (commit walking, tree diffing, blob reading) |
| `tree-sitter` | Incremental parsing framework for import extraction |
| `petgraph` | Directed graph data structure and SCC algorithm |
| `rusqlite` | SQLite storage with bundled library |
| `ratatui` | Terminal UI framework |
| `crossterm` | Cross-platform terminal backend |
| `tokio` | Async runtime for the TUI event loop |
| `rayon` | Data-parallel iteration for parsing |
| `lru` | Bounded LRU cache for blob import results |
| `clap` | CLI argument parsing with derive macros |

---

## Performance

MorphArch is designed for large monorepos with thousands of commits and
hundreds of packages.

- **Incremental scanning** -- only changed directories are re-parsed on
  subsequent runs, using subtree hash caching at `O(changed_dirs)` cost.
- **LRU blob cache** -- a 50,000-entry cache avoids redundant tree-sitter
  parsing of unchanged files across commits.
- **Parallel parsing** -- rayon distributes import extraction across all
  available CPU cores.
- **Graph physics** -- the Verlet integration loop runs at O(V^2 + E) per
  frame, which stays under 4ms for graphs with up to 500 nodes.
- **Release profile** -- LTO, single codegen unit, and symbol stripping are
  enabled for optimized builds.

---

## Minimum Supported Rust Version

The current MSRV is **1.85** (Rust edition 2024).

---

## Contributing

Contributions are welcome. To get started:

1. Fork the repository and create a feature branch.
2. Make your changes and ensure all tests pass:
   ```bash
   cargo test
   ```
3. Run clippy and check formatting:
   ```bash
   cargo clippy -- -D warnings
   cargo fmt --check
   ```
4. Open a pull request against `main`.

Please open an issue first for large changes or new features so the design can
be discussed before implementation begins.

---

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT License ([LICENSE-MIT](LICENSE-MIT) or
  <https://opensource.org/licenses/MIT>)

at your option.

---

## Changelog

See [CHANGELOG.md](CHANGELOG.md) for release history and migration notes.
