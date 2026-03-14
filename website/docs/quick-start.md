# Quick Start

Get from zero to a usable repository scan in a few commands.

## 1. Install MorphArch

Choose the install method that fits your environment:

```bash
cargo install morpharch
```

Linux or macOS:

```bash
curl -fsSL https://raw.githubusercontent.com/onplt/morpharch/main/install.sh | sh
```

Windows PowerShell:

```powershell
irm https://raw.githubusercontent.com/onplt/morpharch/main/install.ps1 | iex
```

## 2. Run your first scan

From the repository root:

```bash
morpharch watch .
```

This does two things:

1. scans Git history and stores repo-scoped scan data locally
2. opens the TUI on the current snapshot

If you want a smaller first pass on a large repository, start with a commit
limit:

```bash
morpharch watch . -n 150 -s 200
```

If you want the full available history, use `-n 0`.

## 3. Start with the map

MorphArch opens on the `Map` view by default.

Use it to answer questions like:

- What are the main subsystems in this repository?
- Which clusters are tightly coupled?
- Which areas look isolated, overloaded, or unusually connected?

### Core navigation

- `Tab` / `Shift+Tab`: move panel focus
- `j/k` or arrow keys: move selection
- `Enter`: drill in
- `Esc`: drill out
- `h/l` or `[ ]`: switch local views

## 4. Open cluster details

From the map:

- select a cluster in the sidebar, or
- click a cluster directly in the map

Inside cluster details, you will see:

- cluster summary
- top members or dependencies
- incoming and outgoing link pressure
- a focused member or dependency lens

## 5. Inspect one member

Select a member and press `Enter`.

This opens `Inspect`, where MorphArch:

- centers the selected node
- shows a focused one-hop subgraph
- keeps the raw graph available for debugging instead of using it as the default view

This is the right place to answer:

- Who depends on this module?
- What does this module depend on?
- Is it acting like a bridge, sink, or hub?

## 6. Move through history

Focus the timeline and scrub commits with:

- `Left` / `Right`
- `j/k`
- mouse drag on the timeline
- `Space` to auto-play

Use this to watch architecture drift, coupling changes, and cluster evolution
across commits.

## 7. Generate a static report

If you want a non-interactive report for the current commit:

```bash
morpharch analyze --path .
```

For recent drift:

```bash
morpharch list-drift --path .
```

## 8. Add project-specific config when needed

Create a `morpharch.toml` if you want to customize:

- ignore presets and repo-specific ignore bundles
- scan heuristics such as package depth and external visibility
- scoring weights and thresholds
- boundary rules
- semantic families and clustering constraints
- presentation aliases, kinds, and color mode

See the [Configuration Guide](./guides/configuration) for the full reference.
