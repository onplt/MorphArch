# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.0.2] - 2026-03-05

### Added

- **Interactive Timeline Scrubbing:** Support for clicking and dragging the timeline slider with the mouse for real-time history exploration.
- **Lazy-Loading Snapshots:** Full graph snapshots are now loaded on-demand from the database, significantly reducing memory footprint for large project histories.

### Changed

- **Performance Optimization:** Replaced linear node lookups with a HashMap ($O(E+N)$), eliminating latency during commit navigation.
- **Physics Optimization:** Implemented Barnes-Hut algorithm with a Quadtree structure for physics repulsion, improving performance from $O(V^2)$ to $O(V \log V)$.

### Fixed

- **Stack Overflow:** Added depth limits to the Quadtree partitioning to prevent infinite recursion on overlapping node coordinates.
- **TUI Stability:** Resolved various borrow checker and argument count issues in the graph rendering engine.

## [0.4.0] - 2025-03-03

### Added

- Animated TUI graph renderer with Verlet physics simulation
- Mouse drag interaction for repositioning graph nodes
- Search filtering to locate and highlight specific nodes
- Weighted edges reflecting dependency strength
- Drift scoring engine for quantifying architectural deviation
- Incremental scanning to avoid re-processing unchanged files
- Subtree-cached tree walks for faster AST traversal
- LRU blob cache to reduce redundant I/O during analysis
- SQLite persistence for scan results and drift history

## [0.3.0] - 2025-02-15

### Added

- Drift scoring system for measuring architecture degradation over time
- Temporal analysis of dependency changes across commits
- Boundary violation detection for module-level architecture rules

## [0.2.0] - 2025-02-01

### Added

- Dependency graph construction using tree-sitter for multi-language parsing
- Graph data model backed by petgraph with stable node indices
- Support for Rust, TypeScript, Python, and Go import extraction

## [0.1.0] - 2025-01-15

### Added

- Initial git repository scanner using gitoxide
- SQLite storage layer for persisting scan metadata
- CLI interface with clap for command-line argument parsing

[Unreleased]: https://github.com/onplt/morpharch/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/onplt/morpharch/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/onplt/morpharch/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/onplt/morpharch/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/onplt/morpharch/releases/tag/v0.1.0
