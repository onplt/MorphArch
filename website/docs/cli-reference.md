# CLI Reference

MorphArch provides a powerful command-line interface designed for both interactive use and automated CI/CD pipelines.

## Global Options

The following flags apply to all subcommands:

| Flag | Long Flag | Description |
|------|-----------|-------------|
| `-v` | `--verbose` | Enable INFO level logging (hidden by default). |
| `-h` | `--help` | Show help information. |
| `-V` | `--version` | Show the version number. |

---

## Commands

### `scan`
Scan a Git repository to build the architectural dependency database.

```bash
morpharch scan [path] [flags]
```

**Arguments:**
- `path`: The local directory of the monorepo (default: `.`).

**Flags:**
- `-n, --max-commits <N>`: Limit the scan to the last N commits. Set to `0` for full history (default).

---

### `watch`
Perform a scan and launch the interactive, animated TUI.

```bash
morpharch watch [path] [flags]
```

**Arguments:**
- `path`: The local directory of the monorepo (default: `.`).

**Flags:**
- `-n, --max-commits <N>`: Limit the history visible in the TUI timeline. Set to `0` for unlimited (default).
- `-s, --max-snapshots <N>`: Max number of data points in the timeline (default: `200`).

---

### `analyze`
Perform a deep architectural audit of a specific commit.

```bash
morpharch analyze [commit] [flags]
```

**Arguments:**
- `commit`: Commit reference to analyze (e.g., `HEAD`, `main~5`, `abc1234`). Defaults to HEAD if omitted.

**Flags:**
- `-p, --path <PATH>`: Path to the Git repository (default: `.`).

The analyze command outputs a comprehensive report including:
- Health score breakdown (6-component debt analysis)
- Boundary violation details
- Circular dependency detection (SCCs)
- Blast radius analysis (articulation points, downstream impact)
- AI-driven improvement recommendations

---

### `list-graphs`
List recent dependency graph snapshots stored in the database.

```bash
morpharch list-graphs
```

Shows the last 10 graph snapshots with commit info.

---

### `list-drift`
Display a historical trend of health scores in a table format.

```bash
morpharch list-drift
```

Shows drift scores, node/edge counts, and delta changes compared to the previous commit for the last 20 commits.

---

## Automation & CI/CD

### Exit Codes
MorphArch follows standard Unix exit codes to indicate status:

| Code | Meaning |
|------|---------|
| `0`  | **Success**: Command completed successfully. |
| `1`  | **Runtime Error**: General failure (e.g., path not found, Git error). |
| `2`  | **Panic**: Internal program failure (please report as a bug). |
