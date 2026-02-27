# MorphArch

Monorepo architecture drift visualizer with animated TUI.

MorphArch scans monorepo Git history (Nx, Turborepo, pnpm workspaces), builds per-commit dependency graphs, calculates architecture drift scores, and visualizes them with an animated force-graph + timeline.

## Sprint 1 — Git History Scanner

```bash
# Scan a repository (last 500 commits)
cargo run -- scan .

# Scan + watch mode (TUI coming in Sprint 2)
cargo run -- watch .

# Help
cargo run -- --help
```

## Build

```bash
cargo build --release
```

## Test

```bash
cargo test
```

## Project Structure

```
src/
├── main.rs          # Entry point, CLI dispatch
├── cli.rs           # Clap derive structs (scan, watch)
├── config.rs        # Default config, DB path (~/.morpharch/)
├── models.rs        # Data structs: CommitInfo, PackageSnapshot, GraphSnapshot
├── db.rs            # SQLite connection, migration, CRUD
├── git_scanner.rs   # gix-based Git commit walker
└── utils.rs         # Logging init, error formatting
```

## License

MIT OR Apache-2.0
