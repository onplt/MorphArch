# Quick Start

Get your first architectural health report in less than 30 seconds.

## 1. Run your first scan

Navigate to your monorepo root and run the `watch` command. This will perform an initial scan and launch the interactive TUI.

```bash
morpharch watch .
```

## 2. Explore the TUI

Once the TUI launches:
- **Arrow Keys / J-K**: Navigate through the commit history on the timeline.
- **Mouse**: Click and drag nodes to rearrange the force-directed graph.
- **Search (/)**: Type a package name to filter the view.
- **R Key**: "Reheat" the graph physics if nodes get stuck.

## 3. Generate a Health Report

If you want a static report of your current HEAD commit, run:

```bash
morpharch analyze
```

This will output a detailed breakdown of your **Architectural Debt**, including specific circular dependencies, God-modules, and topological boundary violations.

## 4. Customize (Optional)

To fine-tune the scoring engine for your project, create a `morpharch.toml` at your repo root:

```toml
[ignore]
paths = ["tests/**", "vendor/**"]

[scoring.weights]
cycle = 35
hub = 20

[[scoring.boundaries]]
from = "packages/"
deny = ["apps/"]
```

All fields are optional. See the [Configuration Guide](./guides/configuration) for the full reference.

---

## Next Steps

- Learn about the [Scoring Engine](./concepts/scoring) to understand your health score.
- Read the [Configuration Guide](./guides/configuration) to customize weights, thresholds, and boundary rules.
