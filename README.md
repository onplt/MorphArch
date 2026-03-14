<p align="center">
  <img src="assets/logo.svg" alt="MorphArch logo" width="600">
</p>

<p align="center">
  <strong>Inspect repository structure, drift, and hotspots from the terminal.</strong>
</p>

<p align="center">
  <a href="https://crates.io/crates/morpharch"><img src="https://img.shields.io/crates/v/morpharch" alt="Crates.io"></a>
  <a href="https://docs.rs/morpharch"><img src="https://img.shields.io/docsrs/morpharch" alt="docs.rs"></a>
  <a href="https://morpharch.dev"><img src="https://img.shields.io/badge/website-morpharch.dev-7aa2f7" alt="Website"></a>
  <a href="https://github.com/onplt/morpharch/actions/workflows/ci.yml"><img src="https://github.com/onplt/morpharch/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/onplt/morpharch#license"><img src="https://img.shields.io/crates/l/morpharch" alt="License"></a>
</p>

MorphArch scans Git history, extracts dependency edges from source code,
computes architectural health, and helps you inspect large repositories through
a terminal UI designed for repeated analysis.

It supports Rust, TypeScript, JavaScript, Python, and Go out of the box, and
works well with Nx, Turborepo, pnpm workspaces, Cargo workspaces, and other
monorepo layouts.

<p align="center">
  <img src="website/static/img/demo.gif" alt="MorphArch TUI demo" width="900">
</p>

---

## Why MorphArch

- **Grouped by default**: large repositories open on a cluster map instead of
  a full raw dependency graph.
- **Git-native**: scan history, not only `HEAD`, and replay changes in the TUI.
- **Language-aware**: import extraction uses safe fast paths with AST fallback
  instead of plain regex matching.
- **Operational**: the TUI is built for triage, inspection, drift review, and
  focused debugging inside the terminal.
- **Configurable**: ignore presets, scoring rules, boundaries, clustering, and
  presentation can all be tuned in `morpharch.toml`.

---

## Features

- **First-parent history scanning**: walks a deterministic Git history stream
  with `gix` and avoids merge-order ambiguity.
- **Repo-scoped local cache**: stores commit frames, checkpoints, and scan
  state in SQLite for fast replay and incremental updates.
- **Language-aware dependency extraction**: parses Rust, TypeScript,
  JavaScript, Python, and Go with comment/string-safe fast paths and AST
  fallback.
- **Terminal UI built for inspection**: cluster map, cluster details, focused
  inspect lens, timeline, and contextual insights.
- **Scale-aware health scoring**: cycle, layering, hub, coupling, cognitive,
  and instability debt combine into a 0-100 health score.
- **Blast radius analysis**: inspect likely downstream impact for high-risk
  modules without leaving the terminal.
- **Config-driven clustering**: semantic families, rules, constraints, aliases,
  kind hints, and color mode can all be customized per repo.
- **Incremental performance**: subtree caching, blob caching, delta frames, and
  parallel parsing reduce repeated scan cost substantially.

---

## Installation

### Quick install

```bash
cargo install morpharch
```

Linux / macOS:

```bash
curl -fsSL https://raw.githubusercontent.com/onplt/morpharch/main/install.sh | sh
```

Windows PowerShell:

```powershell
irm https://raw.githubusercontent.com/onplt/morpharch/main/install.ps1 | iex
```

### Other install methods

| Platform | Command |
|----------|---------|
| `cargo-binstall` | `cargo binstall morpharch` |
| Homebrew | `brew install onplt/morpharch` |
| npm | `npm install -g morpharch` |
| Scoop | `scoop bucket add morpharch https://github.com/onplt/scoop-morpharch` then `scoop install morpharch` |
| AUR | `yay -S morpharch-bin` |
| Docker | `docker run --rm -v .:/repo ghcr.io/onplt/morpharch scan . -n 1` |

From source:

```bash
git clone https://github.com/onplt/morpharch.git
cd morpharch
cargo build --release
```

---

## Quick Start

```bash
# Scan a repository and open the TUI
morpharch watch .

# Static report for HEAD
morpharch analyze --path .

# Historical drift table
morpharch list-drift --path .

# Recent cached graph frames
morpharch list-graphs --path .
```

If you are exploring a large repo for the first time, start with a commit
limit:

```bash
morpharch watch . -n 150 -s 200
```

### TUI mental model

1. `Map`: start with a cluster-level view of the repository.
2. `Cluster details`: open one subsystem to inspect members, dependencies, and
   link pressure.
3. `Inspect`: center a single member and use the focused raw graph only
   when you need graph-level detail.

The insights panel then gives you:

- `Overview`: current state, recent trend, risk drivers, and suggested actions
- `Hotspots`: the modules creating the most pressure
- `Blast`: downstream impact for high-risk modules

---

## Commands

### `morpharch scan <path>`

Scan a Git repository, compute per-commit dependency data, and store it in the
local repo-scoped cache.

```bash
morpharch scan .
morpharch scan /path/to/repo -n 100
```

| Flag | Description |
|------|-------------|
| `-n, --max-commits <N>` | Maximum commits to scan. `0` means unlimited. |

Notes:

- history traversal is `first-parent` only
- repeated scans reuse the local cache when the repo and config are unchanged
- increasing `--max-commits` on an already scanned repo can trigger a backfill rebuild

### `morpharch watch <path>`

Scan a repository and launch the TUI.

```bash
morpharch watch .
morpharch watch . -n 150 -s 200
```

| Flag | Description |
|------|-------------|
| `-n, --max-commits <N>` | Maximum commits to scan before launching the TUI. |
| `-s, --max-snapshots <N>` | Maximum snapshots loaded into the TUI timeline. Default: `200`. |

### `morpharch analyze [commit]`

Generate a detailed report for one commit.

```bash
morpharch analyze --path .
morpharch analyze HEAD~5 --path .
```

### `morpharch list-drift`

Show recent health drift and graph deltas for one repository.

```bash
morpharch list-drift --path .
```

### `morpharch list-graphs`

Show recently stored graph frames for one repository.

```bash
morpharch list-graphs --path .
```

---

## TUI Navigation

### Global model

MorphArch uses one interaction model everywhere:

- `Tab` / `Shift+Tab`: move panel focus
- `j/k` or arrow keys: move selection inside the active panel
- `h/l` or `[ ]`: switch local views or insight tabs
- `Enter`: drill in
- `Esc`: drill out

### Key shortcuts

| Key | Action |
|-----|--------|
| `Tab` / `Shift+Tab` | Cycle panel focus |
| `1-4` | Jump to Packages / Graph / Insights / Timeline |
| `j/k` | Move selection in the active panel |
| `h/l` or `[ ]` | Switch local views or insight tabs |
| `Enter` | Open cluster, inspect member, or open selected item |
| `Esc` | Back out one semantic level |
| `Space` / `p` | Play or pause timeline auto-advance |
| `/` | Filter sidebar entries or graph context |
| `c` | Reset the current graph viewport |
| `r` | Reheat the raw graph layout |
| `x` | Toggle blast overlay |
| `b` / `i` | Toggle sidebar or detail panel |
| `q` | Quit |

### Mouse support

- Click sidebar entries to select them and click map clusters to open the
  corresponding cluster.
- Scroll on the raw graph to zoom.
- Drag the raw graph background to pan.
- Drag the timeline to scrub history.
- Click insight tabs or hotspot rows directly.

---

## Configuration

MorphArch works with zero config, but a `morpharch.toml` in the repo root lets
you tune both analysis and presentation.

```toml
[ignore]
use_defaults = true
presets = ["repo_noise"]
paths = ["third_party/tmp/**"]

[ignore.custom_presets]
repo_noise = [".circleci/**", "scripts/dev/**"]

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

[scoring.weights]
cycle = 30
layering = 25
hub = 15
coupling = 12
cognitive = 10
instability = 8

[scoring.thresholds]
hub_exemption_ratio = 0.3
entry_point_max_fan_in = 2
brittle_instability_ratio = 0.8
blast_high_impact_threshold = 0.3
blast_max_critical_paths = 5

[[scoring.boundaries]]
from = "packages/**"
deny = ["apps/**", "cmd/**"]

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
name = "runtime"
kind = "infra"
include = ["runtime", "runtime/**"]
split = "never"

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
deps = "third-party"
workspace = "platform"

[clustering.presentation.kinds]
platform = "infra"
third-party = "deps"
```

### Configuration highlights

- `ignore.use_defaults`: enables built-in presets for tooling, build artifacts,
  and generated files
- `ignore.presets` / `ignore.custom_presets`: reusable ignore bundles for large repos
- `scan.package_depth`: controls how many meaningful path segments become one
  package label
- `scan.external_min_importers`: hides low-signal third-party dependencies
  unless they are imported by at least `N` internal packages
- `scan.test_path_patterns`: controls which path fragments are treated as
  non-architectural test or fixture code
- `scoring.boundaries`: explicit architectural rules that feed layering debt
- `clustering.families`: stable semantic grouping for important subsystems
- `clustering.rules`: label-based pattern grouping for dynamic naming cases
- `clustering.presentation.kind_mode`: `explicit_then_heuristic` or `explicit_only`
- `clustering.presentation.color_mode`: `minimal` or `semantic`

### Scan heuristics

- Python relative imports such as `from . import config` and
  `from ..shared import util` are resolved as internal dependencies.
- The default test-path filter is intentionally narrow. Directories like
  `examples/`, `bench/`, and `mocks/` are no longer excluded unless you add
  them explicitly through `scan.test_path_patterns` or `ignore` rules.
- Set `scan.external_min_importers = 0` if you want every third-party
  dependency to stay visible in the TUI and dependency views.

---

## Architecture Health Scoring

MorphArch assigns a health score from `0` to `100`.

| Range | Meaning |
|-------|---------|
| `90-100` | Clean |
| `70-89` | Healthy |
| `40-69` | Warning |
| `0-39` | Critical |

The score is built from six components:

- **Cycle debt**
- **Layering debt**
- **Hub / god module debt**
- **Coupling debt**
- **Cognitive debt**
- **Instability debt**

See [morpharch.dev/docs/concepts/scoring](https://morpharch.dev/docs/concepts/scoring) for
the full explanation.

---

## Documentation

- Website: [morpharch.dev](https://morpharch.dev/)
- Docs home: [morpharch.dev/docs/intro](https://morpharch.dev/docs/intro)
- Intro: [morpharch.dev/docs/intro](https://morpharch.dev/docs/intro)
- Quick start: [morpharch.dev/docs/quick-start](https://morpharch.dev/docs/quick-start)
- CLI reference: [morpharch.dev/docs/cli-reference](https://morpharch.dev/docs/cli-reference)
- Configuration guide: [morpharch.dev/docs/guides/configuration](https://morpharch.dev/docs/guides/configuration)
- How it works: [morpharch.dev/docs/concepts/how-it-works](https://morpharch.dev/docs/concepts/how-it-works)

Docs source lives under [website/docs](website/docs).
The landing page source lives at [website/src/pages/index.tsx](website/src/pages/index.tsx).

---

## Contributing

```bash
cargo fmt
cargo test
cargo clippy -- -D warnings
```

Website:

```bash
cd website
npm install
npm run typecheck
npm run build
```

---

## License

Apache-2.0 OR MIT
