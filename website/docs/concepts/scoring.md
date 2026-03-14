# Architectural Health Scoring

MorphArch translates dependency structure into a `0-100` health score.

## How the Score Works

MorphArch starts at `100` and subtracts debt when the graph shows structural
problems that make the repository harder to change.

The score combines six debt components and scales expectations based on
repository size, so larger systems are not penalized for every bit of normal
complexity.

All component weights, thresholds, and exemptions are
[configurable](../guides/configuration) via `morpharch.toml`. The percentages
below are the defaults.

---

## 1. Cycle Debt (default: 30%)

Cycles are one of the clearest signs that module boundaries are no longer
clean.

- **Detection**: uses strongly connected component analysis.
- **Impact**: accounts for up to 30% of total architectural debt by default.
- **Why it matters**: cycles make local changes harder to reason about.

---

## 2. Layering Debt (default: 25%)

Boundaries define the intended flow of your architecture. High-level modules
should depend on low-level modules, not the reverse.

- **Detection**: combines structural layering violations with explicit
  [boundary rules](../guides/configuration#boundary-rules) from
  `morpharch.toml`.
- **Impact**: accounts for up to 25% of total debt by default.
- **Why it matters**: boundary violations create ripple effects across the repo.

---

## 3. Hub / God Module Debt (default: 15%)

Hub modules are modules that have accumulated too much coordination pressure.

- **Detection**: penalizes modules with abnormally high incoming and outgoing
  dependency counts relative to the graph.
- **Entry-point exemption**: MorphArch ignores natural entry points such as
  `main`, `index`, `app`, `lib`, and `mod` by default.
- **Shared core exemption**: modules with a low
  `fan_out / (fan_in + 1)` ratio can be treated as legitimate shared cores.
- **Impact**: accounts for up to 15% of total debt by default.

---

## 4. Coupling Debt (default: 12%)

Large systems are naturally complex, but excessive connections lead to
fragility.

- **Detection**: measures weighted coupling intensity based on import counts
  between modules.
- **Impact**: accounts for up to 12% of total debt by default.
- **Scale grace**: larger monorepos are given more leniency for natural
  coupling than smaller ones.

---

## 5. Cognitive Debt (default: 10%)

Can a developer still reason about the graph without tracing too many links?

- **Detection**: evaluates graph density and distribution complexity.
- **Impact**: accounts for up to 10% of total debt by default.
- **Why it matters**: an architecture can compile and still be too dense to
  reason about safely.

---

## 6. Instability Debt (default: 8%)

Fragile modules are a risk. A module is fragile if it depends on many other
modules, but few things depend on it.

- **Detection**: uses instability-oriented dependency ratios and thresholding.
- **Exception**: leaf nodes, entry points, and items listed in
  `instability_exempt` are excluded from this penalty.
- **Impact**: accounts for up to 8% of total debt by default.

---

## Customizing the Scoring Engine

All defaults above can be overridden in `morpharch.toml`:

```toml
[scoring.weights]
cycle = 35
hub = 20
instability = 5

[scoring.thresholds]
hub_exemption_ratio = 0.25
brittle_instability_ratio = 0.85

[scoring.exemptions]
hub_exempt = ["deno_core"]
entry_point_stems = ["main", "index", "app", "lib", "mod"]
```

Weights are normalized internally so they always sum to 100%. See the
[Configuration Guide](../guides/configuration) for the full reference.

---

## How to Improve Your Score

1. **Break cycles**: use interfaces, traits, or extracted lower-level modules.
2. **Reduce boundary violations**: push dependencies back in the intended
   direction.
3. **Split oversized hubs**: refactor large coordinators into smaller modules.
4. **Review hotspots and blast radius**: MorphArch surfaces the modules creating
   the most risk and the widest downstream impact.
5. **Tune your config carefully**: prefer exemptions and explicit boundaries
   over simply ignoring problematic areas.
