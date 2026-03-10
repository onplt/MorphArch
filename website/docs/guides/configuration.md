# Configuration Guide

MorphArch is designed to work **out of the box** with zero configuration. Its topological analysis automatically detects cycles, boundary violations, and god modules without any manual setup.

When you need to fine-tune the scoring engine for your specific project, place a `morpharch.toml` file at the root of your Git repository. This file is designed to be version-controlled alongside your code so your entire team shares the same architectural health standards.

:::tip Zero-Config First
If you don't create a `morpharch.toml`, MorphArch uses sensible defaults that work well for most monorepos. Start without a config file and add one only when you need to customize behavior.
:::

---

## Config File Location

MorphArch looks for `morpharch.toml` at the root of the repository you pass to `scan`, `watch`, or `analyze`:

```
my-monorepo/
  morpharch.toml   <-- loaded automatically
  src/
  packages/
  ...
```

If the file is missing, all defaults apply. If the file exists but contains invalid TOML, MorphArch will exit with a clear error message.

---

## Ignore Rules

Exclude paths from AST parsing and scoring. Ignored paths are skipped during the Git tree walk, which also improves scan performance for large repositories.

```toml
[ignore]
paths = ["tests/**", "benches/**", "**/generated_*.rs", "vendor/**"]
```

Patterns use standard glob syntax:
- `*` matches any sequence of characters within a path segment
- `**` matches any number of path segments (including zero)
- `?` matches any single character

:::info Performance Benefit
Ignore rules are applied at the Git tree-walk level, meaning entire subtrees are skipped before any file I/O or parsing occurs. This can significantly speed up scans for repositories with large test or vendor directories.
:::

---

## Scoring Weights

Control how much each debt component contributes to the final health score. Values are relative and normalized internally to sum to 1.0, so they don't need to add up to any specific number.

```toml
[scoring.weights]
cycle = 30         # Circular dependencies (SCC analysis)
layering = 25      # Back-edges violating layered architecture
hub = 15           # God modules (high fan-in AND fan-out)
coupling = 12      # Edge weight density (import concentration)
cognitive = 10     # Graph complexity (edge excess + degree excess)
instability = 8    # Brittle modules (Martin instability metric)
```

The values above are the **defaults**. Adjust them to match your project's priorities:

| Project Type | Recommended Adjustment |
|---|---|
| **Legacy codebase** | Lower `cycle` weight to make adoption gradual |
| **Microservices** | Raise `coupling` weight to catch tight bindings early |
| **Monolith** | Raise `hub` weight to catch god modules early |
| **Library/SDK** | Raise `instability` weight to catch fragile public APIs |

### How normalization works

MorphArch normalizes your weights so they always sum to 1.0 internally. For example, if you set:

```toml
[scoring.weights]
cycle = 50
layering = 50
```

The other four components default to their standard values (hub=15, coupling=12, cognitive=10, instability=8), giving a total of 145. Each weight is divided by 145, so cycles would be ~34.5% and layering ~34.5%.

If all weights are set to zero, MorphArch falls back to the default weights automatically.

---

## Scoring Thresholds

Fine-tune when exemptions and penalties kick in.

```toml
[scoring.thresholds]
hub_exemption_ratio = 0.3
entry_point_max_fan_in = 2
brittle_instability_ratio = 0.8
```

### `hub_exemption_ratio` (default: `0.3`)

Modules with `fan_out / (fan_in + 1)` below this ratio are treated as **legitimate shared cores** rather than god modules. These are modules that many packages import but that themselves import relatively few things (e.g., a shared `core` or `utils` package).

Lower this value to be stricter about what counts as a legitimate hub.

### `entry_point_max_fan_in` (default: `2`)

Modules with fan-in at or below this value are treated as **entry-point composition roots** and exempt from hub debt. Entry points like `main.rs` or `cli/tools.ts` naturally wire many packages together but have few (or no) incoming dependencies.

Raise this value if your project has entry points that are imported by a few test or build files.

### `brittle_instability_ratio` (default: `0.8`)

Modules with an instability index `I` greater than this threshold are flagged as brittle. The instability index is calculated as:

```
I = Ce / (Ca + Ce)
```

Where `Ca` = fan-in (afferent coupling) and `Ce` = fan-out (efferent coupling). A module with `I = 1.0` depends on others but nothing depends on it, making it maximally unstable.

Raise this value (e.g., `0.9`) to be more lenient, or lower it (e.g., `0.7`) to catch fragility earlier.

---

## Boundary Rules

Define explicit architectural boundaries: forbidden dependency directions. These are checked during `morpharch analyze` and reported as violations.

```toml
[[scoring.boundaries]]
from = "packages/**"
deny = ["apps/**", "cmd/**"]

[[scoring.boundaries]]
from = "libs/shared/**"
deny = ["libs/feature_*/**"]

[[scoring.boundaries]]
from = "modules/billing/**"
deny = ["modules/auth/**"]
```

Each rule means: modules matching `from` must **NOT** depend on modules matching `deny`. Matching uses prefix comparison (glob wildcards in the pattern are stripped for matching purposes).

:::info Legacy Fallback
When no `[[scoring.boundaries]]` are configured, MorphArch falls back to its built-in topological layering analysis. Boundary rules give you explicit control on top of the automatic detection.
:::

### Common patterns

| Rule | Meaning |
|---|---|
| `from = "packages/**"`, `deny = ["apps/**"]` | Shared packages cannot depend on application code |
| `from = "runtime/"`, `deny = ["cli/"]` | Runtime library cannot depend on CLI tooling |
| `from = "core/"`, `deny = ["plugins/**"]` | Core cannot depend on plugins |

---

## Exemptions

Exempt specific modules from certain debt calculations. Useful for intentional design decisions that would otherwise trigger false positives.

```toml
[scoring.exemptions]
hub_exempt = ["src/utils.rs", "libs/core/index.ts"]
instability_exempt = ["packages/ui-kit/src/index.ts"]
entry_point_stems = ["main", "index", "app", "lib", "mod"]
```

### `hub_exempt` (default: `[]`)

Modules listed here are completely exempt from hub/god-module debt calculation. Use this for intentional utility modules or framework entry points that you know will have high fan-in and fan-out.

### `instability_exempt` (default: `[]`)

Modules listed here are exempt from instability debt calculation. Use this for barrel/re-export files or UI kit entry points that naturally have high fan-out.

### `entry_point_stems` (default: `["main", "index", "app", "lib", "mod"]`)

File stems (filename without extension) that are treated as entry points. Entry points are automatically exempt from fragility penalties because they naturally have high fan-out and low fan-in.

Add custom stems if your project uses non-standard entry point names:

```toml
entry_point_stems = ["main", "index", "app", "lib", "mod", "server", "worker", "cli"]
```

---

## Zero-Config Topology

Even without a `morpharch.toml`, MorphArch provides powerful analysis through automatic detection:

### Topological Layering
Instead of requiring manual boundary rules, MorphArch uses **Topological Sorting** to analyze the natural dependency flow. It detects **back-edges** (when a lower-level module unexpectedly imports a higher-level module) algorithmically.

### Entry Point Detection
MorphArch automatically recognizes composition roots. Any module with a stem matching the `entry_point_stems` list (default: `main`, `index`, `app`, `lib`, `mod`) is forgiven for high fan-out.

### Scale-Aware Scoring
The scoring engine dynamically adjusts baseline expectations based on repository size. Larger monorepos are given more leniency for natural coupling, while smaller projects are held to stricter standards.

---

## Full Example

Here is a complete `morpharch.toml` for a Deno-like monorepo:

```toml
[ignore]
paths = ["tests/**", "cli/tests/**", "tools/**", "bench_util/**"]

[scoring.weights]
cycle = 35
layering = 25
hub = 20
coupling = 10
cognitive = 5
instability = 5

[scoring.thresholds]
entry_point_max_fan_in = 3
hub_exemption_ratio = 0.25
brittle_instability_ratio = 0.85

[[scoring.boundaries]]
from = "runtime/"
deny = ["cli/"]

[[scoring.boundaries]]
from = "ext/"
deny = ["cli/"]

[scoring.exemptions]
hub_exempt = ["deno_core"]
entry_point_stems = ["main", "index", "app", "lib", "mod", "tools"]
```

See also: [`morpharch.example.toml`](https://github.com/onplt/morpharch/blob/main/morpharch.example.toml) in the repository root for a fully commented reference.

---

## Environment Variables

For CI/CD environments, you can override system-level settings:

- `MORPHARCH_DB_PATH`: Path to the SQLite database (default: `~/.morpharch/morpharch.db`).
