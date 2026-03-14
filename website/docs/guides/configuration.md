# Configuration Guide

MorphArch works without a config file, but `morpharch.toml` lets you tune both
analysis and presentation for your repository.

Use config when you need to:

- ignore generated or irrelevant paths
- enable or disable built-in ignore presets
- tune health scoring for your architecture standards
- define explicit dependency boundaries
- exempt known entry points or shared cores
- improve semantic grouping and clustering stability for your monorepo

:::tip Zero-config first
Start without a config file. Add overrides only when the defaults stop matching
how your repo is organized.
:::

---

## Config File Location

Put `morpharch.toml` at the repository root:

```text
my-monorepo/
  morpharch.toml
  apps/
  packages/
  crates/
```

MorphArch loads it automatically for `scan`, `watch`, and `analyze`.

---

## Ignore Rules

Skip paths entirely during repository discovery and parsing.

```toml
[ignore]
use_defaults = true
presets = ["repo_noise"]
paths = ["tests/**", "vendor/**", "benchmarks/**"]

[ignore.custom_presets]
repo_noise = [".circleci/**", "scripts/dev/**"]
```

Use this for:

- generated code
- vendored dependencies
- benchmark fixtures
- directories that are not part of the architecture you want to reason about

### Built-in presets

By default MorphArch enables built-in ignore presets for:

- tooling directories such as `.github/**` and `.vscode/**`
- build artifacts such as `dist/**`, `build/**`, and `target/**`
- generated code such as `**/__generated__/**` and `**/*.d.ts`

If you want full manual control:

```toml
[ignore]
use_defaults = false
paths = ["tests/**"]
```

---

## Scan Heuristics

Tune how MorphArch converts file paths and imports into package-level graph
nodes before scoring and clustering.

```toml
[scan]
package_depth = 2
external_min_importers = 3
test_path_patterns = [
  "/test/",
  "/tests/",
  "/testdata/",
  "/__tests__/",
  "/fixtures/",
  "/e2e/",
]
```

### What these mean

- `package_depth`
  Controls how many meaningful path segments are collapsed into one package
  label. With `1`, paths like `src/auth/utils.rs` and
  `src/auth/strategies/jwt.rs` both resolve to `auth`. With `2`, the second
  path becomes `auth/strategies`.
- `external_min_importers`
  Hides low-signal third-party dependencies from overview-style views unless at
  least that many internal packages import them. Set this to `0` if you want
  every external dependency to stay visible.
- `test_path_patterns`
  Path fragments treated as non-architectural test or fixture code during scan
  discovery. These are normalized fragments, not glob patterns.

### Defaults and behavior

- `package_depth = 2`
- `external_min_importers = 3`
- `test_path_patterns` defaults to a narrow set of canonical test paths:
  - `/test/`
  - `/tests/`
  - `/testdata/`
  - `/test_data/`
  - `/__tests__/`
  - `/spec/`
  - `/fixtures/`
  - `/fixture/`
  - `/snapshots/`
  - `/e2e/`

Notably, `examples/`, `bench/`, `benchmarks/`, `mock/`, and `mocks/` are no
longer filtered by default. If those paths are not part of your architecture,
add them explicitly here or under `[ignore]`.

### Python relative imports

MorphArch now resolves Python relative imports into internal dependencies when
possible:

- `from . import config`
- `from .sub import item`
- `from ..shared import util`

This means Python-first repositories no longer lose large portions of their
internal dependency graph just because they use relative imports.

---

## Scoring Weights

Control how strongly each debt component affects the health score.

```toml
[scoring.weights]
cycle = 30
layering = 25
hub = 15
coupling = 12
cognitive = 10
instability = 8
```

These values are normalized internally, so they do not need to sum to `100`.

### Recommended tuning patterns

| Repo Type | Consider |
| --- | --- |
| Large monolith | Raise `hub` and `coupling` |
| Plugin architecture | Raise `layering` |
| Shared library platform | Raise `instability` |
| Legacy migration | Lower `cycle` at first to reduce noise |

---

## Scoring Thresholds

Control when a module is treated as a legitimate shared core, entry point, or
unstable dependency sink.

```toml
[scoring.thresholds]
hub_exemption_ratio = 0.3
entry_point_max_fan_in = 2
brittle_instability_ratio = 0.8
blast_high_impact_threshold = 0.3
blast_max_critical_paths = 5
```

### What these mean

- `hub_exemption_ratio`
  Treats low-fan-out shared cores as legitimate instead of god modules.
- `entry_point_max_fan_in`
  Prevents main entry points and composition roots from being penalized like
  ordinary modules.
- `brittle_instability_ratio`
  Flags modules that depend heavily on others while few things depend on them.
- `blast_high_impact_threshold`
  Controls which modules count as high impact in blast analysis.
- `blast_max_critical_paths`
  Limits how many critical dependency chains MorphArch keeps in the blast view.

---

## Boundary Rules

Define dependency directions that are not allowed in your architecture.

```toml
[[scoring.boundaries]]
from = "packages/**"
deny = ["apps/**", "cmd/**"]

[[scoring.boundaries]]
from = "runtime/**"
deny = ["cli/**"]
```

Use boundary rules when you want MorphArch to enforce business rules such as:

- shared packages must not depend on apps
- runtime layers must not depend on tooling
- domain modules must not depend on presentation code

If no explicit boundaries are configured, MorphArch still uses its built-in
layering analysis.

---

## Exemptions

Exempt intentional design choices from specific debt calculations.

```toml
[scoring.exemptions]
hub_exempt = ["libs/core", "deno_core"]
instability_exempt = ["packages/ui-kit/src/index.ts"]
entry_point_stems = ["main", "index", "app", "lib", "mod", "server"]
```

Use exemptions sparingly. They are most valuable when you know a module is
supposed to behave like a facade, barrel file, or composition root.

---

## Clustering

Clustering controls how MorphArch turns a raw dependency graph into the
`Map -> Cluster details -> Inspect` navigation model.

The modern config model is layered:

- `semantic`: naming-driven grouping and fallback behavior
- `structural`: split/merge refinement after semantic grouping
- `families`: explicit semantic family definitions
- `rules`: label-based fallback grouping for dynamic naming cases
- `constraints`: hard grouping / separation rules
- `presentation`: user-facing aliases, kind mode, and color mode

```toml
[clustering]
strategy = "hybrid"

[clustering.semantic]
collapse_external = true
fallback_family = "workspace"
root_token_min_repeats = 2
include_exact_roots_for_known_heads = true

[clustering.structural]
enabled = true
min_cluster_size = 2
split_threshold = 6
max_cluster_share = 0.45
preserve_family_purity = true
post_merge_small_clusters = true
disambiguate_duplicate_names = true
```

### Strategy

| Key | Meaning |
| --- | --- |
| `hybrid` | Best default: semantic grouping first, structural cleanup second |
| `namespace` | Prefer names and paths over graph structure |
| `structural` | Prefer graph structure when naming is weak |

### Semantic options

| Key | Meaning |
| --- | --- |
| `collapse_external` | Collapse third-party dependency families when possible |
| `fallback_family` | Name used for generic leftovers that do not fit a better family |
| `root_token_min_repeats` | Minimum repeated token count before a root token becomes a semantic hint |
| `include_exact_roots_for_known_heads` | Merge exact roots like `serde_v8` with `serde_v8/**` when appropriate |

### Structural options

| Key | Meaning |
| --- | --- |
| `enabled` | Turn structural refinement on or off |
| `min_cluster_size` | Smallest cluster MorphArch tries to preserve before merging |
| `split_threshold` | Re-split a dominant cluster when it grows too large |
| `max_cluster_share` | Maximum share of internal nodes one generic cluster should absorb |
| `preserve_family_purity` | Avoid splitting a semantically pure family into noisy subclusters |
| `post_merge_small_clusters` | Re-merge tiny artifacts after split passes |
| `disambiguate_duplicate_names` | Add suffixes when multiple clusters resolve to the same display name |

---

## Semantic Families

Define explicit semantic families when default grouping is too generic.

```toml
[[clustering.families]]
name = "git"
kind = "infra"
include = ["git", "git/**", "git_*"]
split = "never"

[[clustering.families]]
name = "frontend"
kind = "entry"
include = ["apps/web/**", "website/**"]
priority = 20
```

Useful family fields:

| Key | Meaning |
| --- | --- |
| `name` | Family name before presentation aliases are applied |
| `include` | Labels or globs to match into this family |
| `exclude` | Optional negative patterns |
| `kind` | Optional presentation hint for the family |
| `split` | `never`, `allow`, or `prefer` |
| `priority` | Higher-priority families win when patterns overlap |
| `merge_small_into_family` | Prefer merging tiny leftovers back into this family |

Families are the best place to:

- group exact roots with namespaced members
- protect stable subsystems from over-splitting
- override weak default naming in one place

---

## Label-Based Rules

Use clustering rules when you want to group labels by glob without defining a
full semantic family.

```toml
[[clustering.rules]]
name = "node_compat"
kind = "domain"
match = ["node", "node/**", "node_*"]
```

Rules are useful for:

- crate families such as `deno_*`
- compatibility layers such as `node_*`
- repos where exact-root and prefix matching matters more than path hierarchy

---

## Clustering Constraints

Use hard constraints when a few architectural groups must stay together or must
never land in the same cluster.

```toml
[[clustering.constraints]]
type = "must_group"
members = ["core", "core/**"]

[[clustering.constraints]]
type = "must_separate"
left = ["apps/**"]
right = ["packages/**"]
```

Constraint types:

- `must_group`: keep matching labels together
- `must_separate`: force the left and right groups apart

`must_group` is best for shared cores and exact-root-plus-namespace families.
`must_separate` is best for project-specific edge cases where topology alone is
not enough.

---

## Presentation Overrides

Rename cluster labels or override their badge/kind without changing the
underlying clustering logic.

```toml
[clustering.presentation]
kind_mode = "explicit_only"
color_mode = "minimal"

[clustering.presentation.aliases]
workspace = "platform"
deps = "third-party"

[clustering.presentation.kinds]
platform = "infra"
third-party = "deps"
frontend = "entry"
```

Presentation options:

| Key | Meaning |
| --- | --- |
| `kind_mode` | `explicit_then_heuristic` or `explicit_only` |
| `color_mode` | `minimal` or `semantic` |
| `aliases` | rename cluster labels in the TUI |
| `kinds` | assign presentation kinds without changing clustering |

Supported kinds:

- `workspace`
- `deps`
- `entry`
- `external`
- `infra`
- `domain`
- `group`

Cluster kinds are presentation hints. They affect labels and badges in the TUI,
not the underlying dependency graph.

---

## Full Example

```toml
[ignore]
use_defaults = true
paths = ["tests/**", "vendor/**", "dist/**", "tools/**"]

[scan]
package_depth = 2
external_min_importers = 3
test_path_patterns = ["/tests/", "/__tests__/", "/fixtures/"]

[scoring.weights]
cycle = 30
layering = 25
hub = 15
coupling = 12
cognitive = 10
instability = 8

[scoring.thresholds]
hub_exemption_ratio = 0.25
entry_point_max_fan_in = 3
brittle_instability_ratio = 0.85

[[scoring.boundaries]]
from = "runtime/**"
deny = ["cli/**"]

[[scoring.boundaries]]
from = "packages/**"
deny = ["apps/**"]

[scoring.exemptions]
hub_exempt = ["deno_core"]
entry_point_stems = ["main", "index", "app", "lib", "mod", "server"]

[clustering]
strategy = "hybrid"

[clustering.semantic]
collapse_external = true
fallback_family = "workspace"
root_token_min_repeats = 2
include_exact_roots_for_known_heads = true

[clustering.structural]
enabled = true
min_cluster_size = 2
split_threshold = 6
max_cluster_share = 0.45
preserve_family_purity = true
post_merge_small_clusters = true
disambiguate_duplicate_names = true

[[clustering.families]]
name = "git"
kind = "infra"
include = ["git", "git/**", "git_*"]
split = "never"

[[clustering.families]]
name = "frontend"
kind = "entry"
include = ["apps/web/**", "website/**"]

[[clustering.rules]]
name = "node_compat"
kind = "domain"
match = ["node", "node/**", "node_*"]

[[clustering.constraints]]
type = "must_group"
members = ["core", "core/**"]

[clustering.presentation]
kind_mode = "explicit_only"
color_mode = "minimal"

[clustering.presentation.aliases]
workspace = "platform"
deps = "third-party"

[clustering.presentation.kinds]
platform = "infra"
third-party = "deps"
frontend = "entry"
```

---

## Environment Variables

- `MORPHARCH_DB_PATH`: custom path to the local SQLite database

---

## Guidance

If the architecture map feels too generic:

1. add a few explicit `clustering.families`
2. add `clustering.rules` for naming-heavy leftovers
3. add `clustering.constraints` for true edge cases
4. rename things with `clustering.presentation.aliases`
5. only then tune `clustering.structural`

That usually produces a better result than jumping straight to a fully custom
structural strategy.
