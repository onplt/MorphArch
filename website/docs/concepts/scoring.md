# Architectural Health Scoring

The **Scoring Engine** is the core of MorphArch. It translates complex graph relationships into a single, actionable **0-100 Health Score**.

## The Philosophy: Debt vs. Health

MorphArch views architecture through the lens of **Technical Debt**. We start with a perfect score of **100** and subtract points for structural flaws that hinder maintainability.

The engine uses a **6-component scale-aware algorithm** that calculates "Architectural Debt" and subtracts it from a base of 100. It dynamically scales its expectations based on the size of your repository, forgiving necessary complexity while harshly penalizing true anti-patterns.

All component weights, thresholds, and exemptions are [fully configurable](../guides/configuration) via `morpharch.toml`. The percentages below are the defaults.

---

## 1. Cycle Debt (default: 30%)
Cycles are the most severe architectural flaw. They break modularity, making it impossible to test or deploy packages in isolation.

- **Detection**: Uses **Kosaraju's Algorithm** to find **Strongly Connected Components (SCC)**.
- **Impact**: Accounts for up to 30% of total architectural debt by default.
- **Why it matters**: Cycles lead to "Spaghetti Code" where changing one line in a library might break an entire unrelated application. Break cycles using dependency inversion or by extracting shared logic into a lower-level package.

---

## 2. Layering Debt (default: 25%)
Boundaries define the "flow" of your architecture. High-level modules should depend on low-level modules, never the other way around.

- **Detection**: Measures back-edges in the topological ordering of the dependency graph. You can also define explicit [boundary rules](../guides/configuration#boundary-rules) in `morpharch.toml`.
- **Impact**: Accounts for up to 25% of total debt by default.
- **Why it matters**: Dependencies that violate layer constraints (e.g., shared libs depending on application code) create massive ripple effects.

---

## 3. Hub / God Module Debt (default: 15%)
"God modules" are those that do too much, knowing everything and being known by everyone.

- **Detection**: Penalizes modules that have abnormally high incoming (Fan-in) and outgoing (Fan-out) dependencies relative to the graph's median.
- **Exception (Entry Points)**: MorphArch ignores natural entry points (`main`, `index`, `app`, `lib`, `mod` by default). It understands that a `main` module is *supposed* to wire everything together and won't penalize it as a God module. The entry point stems and hub exemption thresholds are [configurable](../guides/configuration#exemptions).
- **Shared Cores**: Modules with `fan_out / (fan_in + 1)` below the `hub_exemption_ratio` (default: 0.3) are treated as legitimate shared cores and exempt from hub debt.
- **Impact**: Accounts for up to 15% of total debt by default.

---

## 4. Coupling Debt (default: 12%)
Large systems are naturally complex, but excessive connections lead to fragility.

- **Detection**: Calculates weighted coupling intensity based on the exact count of import statements between modules.
- **Impact**: Accounts for up to 12% of total debt by default.
- **Scale Grace**: Larger monorepos are given more leniency for natural coupling than smaller ones.

---

## 5. Cognitive Debt (default: 10%)
Can a human developer actually understand this graph?

- **Detection**: Evaluates graph **Shannon entropy** and edge excess ratios.
- **Impact**: Accounts for up to 10% of total debt by default.
- **Why it matters**: Penalizes structures where the sheer density of connections makes the system impossible for a human to reason about, even if it technically compiles.

---

## 6. Instability Debt (default: 8%)
Fragile modules are a risk. A module is fragile if it depends on many other modules, but few (or none) depend on it.

- **Detection**: Based on Robert C. Martin's Abstractness/Instability metrics. Flags modules with instability index `I > 0.8` (configurable via `brittle_instability_ratio`).
- **Exception**: Leaf nodes (packages with no outgoing dependencies), entry points, and modules listed in `instability_exempt` are excluded from this penalty.
- **Impact**: Accounts for up to 8% of total debt by default.

---

## Customizing the Scoring Engine

All defaults above can be overridden in `morpharch.toml`:

```toml
[scoring.weights]
cycle = 35        # Increase cycle penalty
hub = 20          # Increase god-module penalty
instability = 5   # Decrease instability penalty

[scoring.thresholds]
hub_exemption_ratio = 0.25       # Stricter hub exemption
brittle_instability_ratio = 0.85 # More lenient instability threshold

[scoring.exemptions]
hub_exempt = ["deno_core"]                                   # Exempt known hubs
entry_point_stems = ["main", "index", "app", "lib", "mod"]   # Customize entry points
```

Weights are normalized internally so they always sum to 100%. See the [Configuration Guide](../guides/configuration) for the full reference.

---

## How to Improve Your Score

1.  **Break Cycles**: Use interfaces or traits to invert dependencies.
2.  **Split Responsibilities**: Refactor "Hub/God Modules" by splitting them into smaller, single-purpose packages.
3.  **Review the TUI Advisory**: The MorphArch TUI provides actionable, senior-architect-level advice on exactly which modules are causing the most debt.
4.  **Tune Your Config**: If a legitimate design decision triggers false positives, use [exemptions](../guides/configuration#exemptions) rather than ignoring the warning.
