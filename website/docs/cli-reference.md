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
- `-n, --max-commits <N>`: Limit the scan to the last N commits. Set to `0` for full history.

---

### `watch`
Perform a scan and launch the interactive, animated TUI.

```bash
morpharch watch [path] [flags]
```

**Flags:**
- `-n, --max-commits <N>`: Limit the history visible in the TUI timeline.
- `-s, --max-snapshots <N>`: Max number of data points in the timeline (default: `200`).

---

### `analyze`
Perform a deep architectural audit of the current HEAD or a specific commit.

```bash
morpharch analyze [commit] [flags]
```

**Flags:**
- `--json`: Output the report in machine-readable JSON format.
- `-p, --path <PATH>`: Path to the repo (default: `.`).

---

### `list-drift`
Display a historical trend of health scores in a table format.

```bash
morpharch list-drift
```

---

## 🛠 Automation & CI/CD

### Exit Codes
MorphArch follows standard Unix exit codes to indicate status:

| Code | Meaning |
|------|---------|
| `0`  | **Success**: Command completed successfully. |
| `1`  | **Runtime Error**: General failure (e.g., path not found, Git error). |
| `2`  | **Panic**: Internal program failure (please report as a bug). |

:::tip Pro Tip
In CI/CD, use `morpharch analyze --json | jq '.total'` to extract the health score and fail the build if it falls below your team's threshold.
:::

---

### JSON Output Schema (`analyze --json`)

When running with the `--json` flag, MorphArch returns a structured object. Use this for custom reporting or dashboards.

```json
{
  "commit": "abc1234...",
  "total": 92,
  "metrics": {
    "cycles": 1,
    "violations": 2,
    "density": 3.8
  },
  "cycles": [
    ["pkg-a", "pkg-b", "pkg-a"]
  ],
  "violations": [
    {
      "from": "shared-lib",
      "to": "web-app",
      "rule": "shared/ -> apps/"
    }
  ],
  "metadata": {
    "node_count": 12,
    "edge_count": 45,
    "timestamp": 1709812345
  }
}
```

#### Field Definitions:
- `total`: The final Health Score (0-100).
- `metrics.cycles`: Number of circular dependency groups detected.
- `metrics.violations`: Number of boundary rules broken.
- `cycles`: An array of arrays, where each inner array lists the packages in a circular loop.
- `violations`: A list of specific edges that broke a defined architectural rule.
