# Configuration Guide

MorphArch is "zero-config" by default, but truly shines when you customize it to match your team's architectural standards.

## The `morpharch.toml` File

Place this file in your repository root. MorphArch will automatically detect and apply these settings during the `scan` and `watch` commands.

```toml
# --- Scoring Settings ---
[scoring]
# The point at which coupling is considered "too dense".
# Default: 3.5
density_threshold = 4.0

# --- Layer Boundaries ---
[boundaries]
# Format: ["Source Prefix", "Illegal Destination Prefix"]
# MorphArch will penalize dependencies that match these pairs.
rules = [
    ["packages/core", "apps/"],    # Core should not know about Apps
    ["packages/shared", "packages/features"], # Shared libs should be pure
    ["libs/", "cmd/"]              # Libraries should not depend on CLI entrypoints
]

# --- Scanner Exclusions ---
[scan]
# Patterns to skip during AST parsing.
# MorphArch respects .gitignore, but you can add specific paths here.
ignore = [
    "**/tests/**",
    "**/benchmarks/**",
    "vendor/"
]
```

## Environment Variables

For CI/CD environments, you can override settings using environment variables:

- `MORPHARCH_CONFIG`: Path to a custom config file.
- `MORPHARCH_DB_PATH`: Path to the SQLite database (default: `~/.morpharch/morpharch.db`).

---

## Workspace Autodetection

MorphArch is built for monorepos. It natively understands:

### Nx & Turborepo
It parses `project.json` and `turbo.json` to identify package boundaries automatically.

### Cargo Workspaces
It reads the `[workspace]` section of your root `Cargo.toml`.

### pnpm & Lerna
It follows `pnpm-workspace.yaml` and `lerna.json` structures.

:::info
If your workspace is not detected, MorphArch will fall back to **Directory-level analysis**, treating each top-level folder as a package.
:::
