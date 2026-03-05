# MorphArch Examples

Practical usage examples for MorphArch.

## Quick Start

```bash
# Install
cargo install morpharch

# Scan a monorepo (all commits)
morpharch scan /path/to/monorepo

# Scan with a commit limit
morpharch scan /path/to/monorepo -n 100

# Launch the animated TUI
morpharch watch /path/to/monorepo

# Analyze architecture for the current HEAD
morpharch analyze

# View health trend
morpharch list-drift
```

## Scanning a Monorepo

```bash
# Full scan of an Nx workspace
morpharch scan ~/projects/my-nx-monorepo

# Scan only the last 50 commits for a quick overview
morpharch scan ~/projects/my-nx-monorepo -n 50

# Scan a Cargo workspace
morpharch scan ~/projects/my-rust-workspace
```

## Interactive TUI

```bash
# Launch TUI with default settings (200 timeline snapshots)
morpharch watch .

# Launch with more timeline snapshots for detailed history
morpharch watch . -s 500

# Limit scan depth for faster startup
morpharch watch . -n 100 -s 50
```

### TUI Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `j` / `Down` | Navigate to next (older) commit |
| `k` / `Up` | Navigate to previous (newer) commit |
| `p` / `Space` | Play / pause auto-play |
| `r` | Reheat graph physics |
| `/` | Enter search mode |
| `Esc` | Exit search / quit |
| `q` | Quit |

## Architecture Analysis

```bash
# Analyze the current HEAD commit
morpharch analyze

# Analyze a specific commit
morpharch analyze abc1234

# Analyze a relative reference
morpharch analyze main~10

# View the health trend over the last 20 commits
morpharch list-drift

# List stored graph snapshots
morpharch list-graphs
```

## CI Integration

Add MorphArch to your CI pipeline to track health over time:

```yaml
# .github/workflows/health-check.yml
name: Architecture Health Check
on: [push]
jobs:
  health:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0  # Full history needed
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo install morpharch
      - run: morpharch scan . -n 50
      - run: morpharch analyze
```
