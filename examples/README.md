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

# Launch the TUI
morpharch watch /path/to/monorepo

# Analyze architecture for the current HEAD
morpharch analyze --path /path/to/monorepo

# View health trend and cached graph frames
morpharch list-drift --path /path/to/monorepo
morpharch list-graphs --path /path/to/monorepo
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
| `j` / `k` | Move selection inside the active panel |
| `Tab` | Switch panel focus |
| `h` / `l` | Switch local views or insight tabs |
| `Enter` | Open a cluster, item, or inspect target |
| `Left` / `Right` | Move through timeline |
| `p` / `Space` | Play / pause auto-play |
| `r` | Reheat graph physics |
| `/` | Enter search mode |
| `Esc` | Exit search / drill out |
| `q` | Quit |

## Architecture Analysis

```bash
# Analyze the current HEAD commit
morpharch analyze --path .

# Analyze a specific commit
morpharch analyze abc1234 --path .

# Analyze a relative reference
morpharch analyze main~10 --path .

# View the health trend over the last 20 commits
morpharch list-drift --path .

# List stored graph snapshots
morpharch list-graphs --path .
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
          fetch-depth: 0
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo install morpharch
      - run: morpharch scan . -n 50
      - run: morpharch analyze --path .
```
