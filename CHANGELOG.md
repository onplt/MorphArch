# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.2.0] - 2026-03-09

### Added

- **6-Component Scale-Aware Scoring Algorithm:** Completely overhauled the architectural health engine. It now calculates debt based on 6 critical components: Cycle Debt (30%), Layering Debt (25%), Hub Debt (15%), Coupling Debt (12%), Cognitive Debt (10%), and Instability Debt (8%).
- **Entry Point Exemption:** The scoring engine is now smart enough to detect natural application entry points (`main`, `index`, `app`, `lib`, `mod`) and automatically exempts them from "God Module" and "Fragility" penalties, eliminating false-positive architectural warnings.
- **Topological Boundary Detection:** Replaced rigid regex-based layer rules with algorithmic Topological Sorting. Boundary violations are now natively detected as back-edges in the dependency flow, creating a truly zero-config experience.

### Changed

- **Responsive TUI Trends Panel:** Re-architected the `ratatui` layout constraints for the Trends tab. The Health Sparkline is now capped at an optimal fixed height, preventing the ugly "infinite vertical stretch" bug on high-resolution terminals.
- **Robust Multibyte String Handling:** Refactored the `truncate_str` helper function to use safe `.chars().take(n)` iterators instead of raw byte slicing. This completely resolves a critical panic (crash) that occurred when navigating the timeline over commits containing Unicode symbols (like `→`) or emojis in their messages.
- **Comprehensive Documentation Rewrite:** The `README.md`, `CLAUDE.md`, and all Docusaurus website contents (`docs/` and `src/pages/`) have been fully refactored by a Senior Architect. The documentation now perfectly reflects the new scale-aware scoring system and zero-config philosophy.
- **Dead Code Eradication:** Conducted a massive cleanup of deprecated functions (`TemporalDelta`, unused `format_timestamp`, legacy tests). The codebase is now 100% clean under `cargo clippy`.

## [1.1.0] - 2026-03-05

### Added

- **JavaScript & JSX Support:** Expanded AST parsing to include `.js` and `.jsx` files, enabling architecture analysis for a wider range of monorepos.
- **k9s-Inspired Dashboard:** Redesigned the TUI insight panel with high-fidelity "Mission Control" aesthetics, featuring key-value alignment, stylized block headers, and clear section separators.
- **Scientific Absolute Health Scoring:** Transitioned from relative drift to a 0-100 absolute health scale. Higher scores now represent better structural integrity.
- **Vulnerable Components Table:** Real-time hotspots analysis in the TUI, listing packages with high instability (Uncle Bob's metric) and coupling risk.

### Changed

- **Fair Complexity Thresholds:** Tuned the coupling density penalty to a base threshold of 3.5 connections per package, providing grace for naturally complex large-scale projects like Deno.
- **Scoring Weight Adjustment:** Prioritized structural correctness (cycles and layer violations) as primary health detractors (-25 and -15 pts respectively).
- **History Trend Visualization:** Fixed sparkline scaling issues to ensure historical trends remain visually meaningful even during periods of architectural stability.

### Fixed

- **Terminology Alignment:** Unified terminology across the TUI and documentation, moving from "Drift" to "Health" and "Architectural Debt".
- **Clippy Type Complexity:** Refactored parallel scanning results into clean type aliases to satisfy strict linting rules.

## [1.0.4] - 2026-03-05

### Added

- **Interactive Hover Labels:** Mouse-over functionality reveals the full name of any node, even when hidden by label density limits, with high-contrast highlighting.

### Changed

- **Physics Stability:** Increased central gravity and implemented soft boundary forces to prevent disconnected nodes from bunching at corners, ensuring a clean "cloud" layout.
- **Improved Hover Logic:** Fine-tuned the mouse-node detection radius for pixel-perfect interaction.

## [1.0.3] - 2026-03-05

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

[1.1.0]: https://github.com/onplt/morpharch/compare/v1.0.4...v1.1.0
[1.0.4]: https://github.com/onplt/morpharch/compare/v1.0.3...v1.0.4
[1.0.3]: https://github.com/onplt/morpharch/compare/v1.0.2...v1.0.3
[1.0.2]: https://github.com/onplt/morpharch/compare/v1.0.1...v1.0.2
[1.0.1]: https://github.com/onplt/morpharch/compare/v1.0.0...v1.0.1
[1.0.0]: https://github.com/onplt/morpharch/releases/tag/v1.0.0
