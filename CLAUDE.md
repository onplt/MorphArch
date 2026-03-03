# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

MorphArch is a monorepo architecture drift visualizer with an animated TUI. It scans Git history, builds dependency graphs per commit, calculates drift scores, and renders an interactive force-directed graph in the terminal.

- Rust 2024 edition, MSRV 1.85
- Uses `gix` (pure-Rust Git) — no subprocess/shell calls to git
- SQLite via `rusqlite` (bundled) at `~/.morpharch/morpharch.db`
- Tree-sitter for import extraction (Rust, TypeScript, Python, Go)
- `ratatui` + `crossterm` for TUI, `petgraph` for graph operations

## Build Commands

```bash
cargo build                                    # Debug build
cargo build --release                          # Release build (LTO enabled)
cargo test --locked                            # Run all tests
cargo test <test_name>                         # Run a single test
cargo clippy --all-targets -- -D warnings      # Lint (warnings are errors in CI)
cargo fmt --all -- --check                     # Check formatting
cargo fmt                                      # Auto-format
cargo doc --no-deps --document-private-items   # Build docs
```

Formatting: max_width=100 (see `.rustfmt.toml`).

## Architecture

The codebase follows a pipeline: **Git → Parse → Graph → Score → Store → Display**.

### Core Pipeline (executed per commit during scan)

1. **git_scanner.rs** — Walks commits via `gix`, diffs trees, collects changed blobs. Uses subtree caching for incremental scans (O(changed_dirs) not O(all_dirs)).
2. **parser.rs** — Extracts imports from source blobs using tree-sitter. Supports Rust (`use`/`extern crate`), TypeScript (`import from`), Python (`import`/`from`), Go (`import`). Files >512KB are skipped. LRU cache bounded at 50K entries.
3. **graph_builder.rs** — Constructs a `petgraph::DiGraph` from dependency edges.
4. **scoring.rs** — Computes a 0–100 drift score with sub-metrics: fan-in/out delta, cycle count (Kosaraju SCC), boundary violations, cognitive complexity. Boundary rules define forbidden cross-layer deps (e.g., `packages→apps`, `libs→apps`).
5. **db.rs** — Persists commits and graph snapshots to SQLite (WAL mode). Two tables: `commits` and `graph_snapshots` (stores serialized JSON).

### Commands (src/commands/)

- **scan** — Orchestrates the full pipeline. Supports incremental scanning (skips already-processed commits). Parallel parsing via rayon.
- **watch** — Runs scan then launches the TUI.
- **analyze** — Generates a per-commit drift report with recommendations.

### TUI (src/tui/)

- **app.rs** — Main event loop, state management, ratatui rendering. Layout: package list (left), graph canvas (center), drift panel (right), timeline (bottom).
- **graph_renderer.rs** — Verlet physics engine for force-directed layout. Adaptive step count for large graphs. 30fps target.
- **insight_panel.rs** — Displays drift metrics, hotspots, and recommendations.
- **widgets.rs** — Shared widget helpers.

### Supporting Modules

- **cli.rs** — Clap derive definitions for all subcommands.
- **models.rs** — Core types: `CommitInfo`, `DependencyEdge`, `GraphSnapshot`, `DriftScore`, `TemporalDelta`.
- **config.rs** — Manages `~/.morpharch/` directory and database path.
- **utils.rs** — Tracing/logging initialization.

## Key Design Decisions

- **No git CLI dependency**: All Git operations use `gix` (pure Rust). No `Command::new("git")`.
- **Incremental scanning**: Subtree caching + commit deduplication makes rescans 5–20x faster.
- **Parameterized SQL everywhere**: All `rusqlite` queries use `?N` placeholders via `params![]`.
- **Package name extraction**: Recognizes monorepo patterns (`packages/X/`, `apps/X/`, `libs/X/`), skips common roots (`src`, `lib`, `internal`), falls back to first 2 meaningful path components.
- **Test/fixture filtering**: The scanner excludes directories like `test/`, `__tests__/`, `fixtures/`, `examples/` to reduce noise.

## CI

GitHub Actions (`.github/workflows/ci.yml`) runs on Ubuntu, macOS, Windows across stable, MSRV (1.85), and beta Rust. Jobs: check, fmt, clippy, test, doc, audit.
