# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.0.3] - 2026-03-05

### Added

- **Commit Date Visibility:** The timeline now displays the commit date (`YYYY-MM-DD`) alongside the hash and message for better temporal context.

## [1.0.2] - 2026-03-05

### Added

- **Interactive Timeline Scrubbing:** Support for clicking and dragging the timeline slider with the mouse for real-time history exploration.
- **Commit Date Visibility:** The timeline now displays the commit date (`YYYY-MM-DD`) alongside the hash and message for better temporal context.
- **Lazy-Loading Snapshots:** Full graph snapshots are now loaded on-demand from the database, significantly reducing memory footprint for large project histories.

### Changed

- **Performance Optimization:** Replaced linear node lookups with a HashMap ($O(E+N)$), eliminating latency during commit navigation.
- **Physics Optimization:** Implemented Barnes-Hut algorithm with a Quadtree structure for physics repulsion, improving performance from $O(V^2)$ to $O(V \log V)$.

### Fixed

- **Stack Overflow:** Added depth limits to the Quadtree partitioning to prevent infinite recursion on overlapping node coordinates.
- **TUI Stability:** Resolved various borrow checker and argument count issues in the graph rendering engine.

## [1.0.1] - 2026-03-04

### Added

- **Docs.rs Support:** Added `lib.rs` and proper module structure to support documentation generation.
- **Enhanced Documentation:** Improved README and in-code documentation for better developer onboarding.

### Fixed

- **Dependency Security:** Upgraded `gix` and `lru` to resolve audited vulnerabilities.
- **CI/CD Fixes:** Corrected invalid GitHub Actions configurations for cargo-audit.

## [1.0.0] - 2026-03-04

### Added

- **Initial Stable Release:** Complete core functionality for architecture drift visualization.
- **Animated TUI:** Interactive terminal UI with Verlet physics-based graph rendering.
- **Drift Scoring:** sophisticated algorithm to quantify architectural deviation across Git history.
- **SQLite Persistence:** Durable storage for scan results, graph snapshots, and drift metrics.
- **Multi-Language Support:** Import extraction for Rust, TypeScript, Python, and Go via tree-sitter.

[Unreleased]: https://github.com/onplt/morpharch/compare/v1.0.3...HEAD
[1.0.3]: https://github.com/onplt/morpharch/compare/v1.0.2...v1.0.3
[1.0.2]: https://github.com/onplt/morpharch/compare/v1.0.1...v1.0.2
[1.0.1]: https://github.com/onplt/morpharch/compare/v1.0.0...v1.0.1
[1.0.0]: https://github.com/onplt/morpharch/releases/tag/v1.0.0
