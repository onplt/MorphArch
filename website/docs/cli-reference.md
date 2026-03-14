# CLI Reference

MorphArch supports two main workflows:

- interactive repository inspection with `watch`
- static analysis and reporting with `scan`, `analyze`, and repo-scoped cache commands

The interactive UI is organized into three levels:

- `Map`: understand repo shape
- `Cluster details`: inspect one subsystem
- `Inspect`: debug one member or module

---

## Global Options

| Flag | Long Flag | Description |
| --- | --- | --- |
| `-v` | `--verbose` | Enable INFO-level logging |
| `-h` | `--help` | Show help |
| `-V` | `--version` | Show the installed version |

---

## Commands

### `scan`

Scan a repository, extract dependency graphs from Git history, compute health,
and store snapshots locally.

```bash
morpharch scan [path] [flags]
```

**Arguments**

- `path`: repository path, default `.`.

**Flags**

- `-n, --max-commits <N>`: limit how much history is scanned. `0` means no
  limit.

**Notes**

- history traversal is `first-parent` only
- scan data is cached per repository in the local SQLite database
- repeated runs reuse cached state when the repo and config are unchanged

### `watch`

Scan a repository and launch the terminal UI.

```bash
morpharch watch [path] [flags]
```

**Arguments**

- `path`: repository path, default `.`.

**Flags**

- `-n, --max-commits <N>`: limit how much history is scanned before launch
- `-s, --max-snapshots <N>`: limit how many snapshots are loaded into the
  timeline

### What `watch` opens

MorphArch opens on the architecture map whenever clustering is available.

From there the expected flow is:

1. open a cluster from `Map`
2. inspect cluster details and the selected-item lens
3. open a centered raw graph lens in `Inspect`

The insights panel is organized into:

- `Overview`: current state, recent trend, risk drivers, and suggested actions
- `Hotspots`: the modules creating the most pressure
- `Blast`: downstream impact for high-risk modules

### `analyze`

Generate a static architecture report for one commit.

```bash
morpharch analyze [commit] [flags]
```

**Arguments**

- `commit`: commit reference such as `HEAD`, `main~5`, or `abc1234`. Defaults
  to `HEAD`.

**Flags**

- `-p, --path <PATH>`: repository path, default `.`.

### `list-graphs`

List recently stored graph snapshots.

```bash
morpharch list-graphs --path .
```

**Flags**

- `-p, --path <PATH>`: repository path, default `.`.

### `list-drift`

Show recent health drift and graph deltas in a compact table.

```bash
morpharch list-drift --path .
```

**Flags**

- `-p, --path <PATH>`: repository path, default `.`.

---

## TUI Navigation Model

MorphArch uses one consistent interaction model across the TUI.

### Global

- `Tab` / `Shift+Tab`: move panel focus
- `1-4`: jump to Packages, Graph, Insights, or Timeline
- `q`: quit
- `?`: open help

### Selection and drill-down

- `j/k` or `Up/Down`: move selection in the active panel
- `Enter`: open selected cluster, member, or inspect target
- `Esc`: go back one semantic level

### Local views

- `h/l` or `[ ]`: switch local views or tabs

### Graph and timeline

- mouse wheel: zoom raw inspect graph
- drag graph background: pan in inspect mode
- `c`: reset graph viewport
- `r`: reheat raw graph layout
- `Left/Right`: move through timeline
- `Space` or `p`: play/pause timeline

### Filtering and panel visibility

- `/`: filter current sidebar or graph context
- `b`: toggle sidebar
- `i`: toggle detail panel
- `x`: toggle blast overlay when available

---

## Exit Codes

| Code | Meaning |
| --- | --- |
| `0` | Success |
| `1` | Runtime error |
| `2` | Internal failure / panic |

---

## Typical Workflows

### Explore a repo interactively

```bash
morpharch watch .
```

### Generate a point-in-time report

```bash
morpharch analyze HEAD --path .
```

### Review recent drift

```bash
morpharch list-drift --path .
```

### Limit history for a faster local session

```bash
morpharch watch . -n 150 -s 200
```
