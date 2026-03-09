# Introduction

**MorphArch** is a high-performance monorepo architecture health visualizer. It is designed for developers and architects who need to maintain structural integrity in large-scale codebases.

## The Problem: Architecture Drift

As monorepos grow, architectural boundaries often blur. Developers accidentally introduce circular dependencies or violate layer constraints, leading to:
- **Tightly coupled code** that is hard to test.
- **Slow build times** due to massive dependency chains.
- **Cognitive overload** when trying to understand the system.

## The Solution: MorphArch

MorphArch scans your Git history, builds per-commit dependency graphs using **AST parsing**, and calculates an **Architectural Health Score**. It gives you the visual and quantitative data you need to stop technical debt before it becomes unmanageable.

---

## Why MorphArch?

- **Visual Clarity**: See your architecture as an animated, interactive graph.
- **Quantitative Health**: Get a 0-100 score based on a 6-component scale-aware algorithm (cycles, boundaries, coupling, cognitive load, and fragility).
- **High Performance**: Written in Rust, optimized for monorepos with thousands of files.
- **Git-Native**: Analyze the evolution, not just the current state.

## Next Steps

Ready to dive in? Follow these guides:

1.  **[Installation](./installation)**: Get MorphArch on your machine.
2.  **[Quick Start](./quick-start)**: Analyze your repo in 30 seconds.
3.  **[Scoring Engine](./concepts/scoring)**: Understand how we calculate health.
