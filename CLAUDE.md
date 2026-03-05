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
4. **scoring.rs** — Computes a 0–100 absolute health score (100 is best). Penalizes cycles (-25), boundary violations (-15), and excessive coupling density (>3.5).
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

- **Absolute Health**: Scoring is based on structural correctness (0 debt = 100% health).
- **No git CLI dependency**: All operations use pure-Rust `gix`.
- **Mission Control UI**: TUI adheres to k9s-style professional terminal aesthetics.
- **Large Repo Support**: Density thresholds are tuned for scale (e.g., Deno).

## CI

GitHub Actions (`ci.yml`) runs formatting, linting, tests, and security audits across all major platforms.
