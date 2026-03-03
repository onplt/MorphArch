# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
