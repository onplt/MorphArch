# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

MorphArch is a monorepo architecture health visualizer with an animated TUI. It scans Git history, builds dependency graphs per commit, calculates absolute health scores, and renders an interactive force-directed graph in the terminal.

- Rust 2024 edition, MSRV 1.85
- Uses `gix` (pure-Rust Git) — no subprocess/shell calls to git
- SQLite via `rusqlite` (bundled) at `~/.morpharch/morpharch.db`
- Tree-sitter for import extraction (Rust, TypeScript, JavaScript, Python, Go)
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

1. **git_scanner.rs** — Walks commits via `gix`, diffs trees, collects changed blobs. Uses subtree caching for incremental scans.
2. **parser.rs** — Extracts imports from source blobs using tree-sitter. Supports Rust, TypeScript/TSX, JavaScript/JSX, Python, and Go.
3. **graph_builder.rs** — Constructs a `petgraph::DiGraph` from dependency edges.
4. **scoring.rs** — Computes a 0–100 absolute health score using a 6-component scale-aware algorithm: Cycle Debt (30%), Layering Debt (25%), Hub Debt (15%), Coupling Debt (12%), Cognitive Debt (10%), and Instability Debt (8%). Automatically exempts entry points (`main`, `index`, `app`) from fragility penalties.
5. **db.rs** — Persists commits and graph snapshots to SQLite (WAL mode).

### Commands (src/commands/)

- **scan** — Orchestrates the full pipeline. Parallel parsing via rayon. Incremental by default.
- **watch** — Runs scan then launches the TUI.
- **analyze** — Generates a per-commit health report with AI-driven recommendations.

### TUI (src/tui/)

- **app.rs** — Main event loop and state. Layout: package list, graph canvas, k9s-style insights, timeline.
- **insight_panel.rs** — High-fidelity dashboard displaying health, debt breakdown, and vulnerable hotspots.
- **graph_renderer.rs** — Verlet physics engine ($O(V \log V)$ with Quadtree repulsion).

## Key Design Decisions

- **Scale-Aware Health Scoring**: The 6-component debt algorithm dynamically adjusts baseline expectations depending on repo size. "God object" limits and cognitive complexity thresholds relax naturally for large monorepos (e.g., Deno).
- **Entry Point Exemption**: Prevents false positive "fragile" warnings for natural composition roots like `main`, `app`, and `index`.
- **No git CLI dependency**: All operations use pure-Rust `gix` for maximum portability and speed.
- **Mission Control UI**: TUI adheres to k9s-style professional terminal aesthetics, optimized to prevent layout stretching on dynamic panels (e.g. fixed-height sparklines).

## CI

GitHub Actions (`ci.yml`) runs formatting, linting, tests, and security audits across all major platforms.
