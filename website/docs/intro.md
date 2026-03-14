# Introduction

MorphArch is a terminal tool for inspecting dependency structure and
architectural drift in large repositories.

It scans Git history, extracts dependency edges from source code, computes
health metrics, and gives you a grouped view of the repository before you drop
into member-level detail.

## What it is for

Large repositories become hard to reason about in two ways at the same time:

- the dependency graph becomes too dense to review directly
- structural drift accumulates faster than teams notice it

MorphArch addresses both problems by combining:

- language-aware dependency extraction
- repository health scoring
- grouped navigation in the terminal
- history replay across commits

## The TUI model

MorphArch is not only a raw graph viewer. The interface is organized into
three levels:

### `Map`

Get a high-level view of the repository.

- cluster overview
- strongest links
- structure summary

### `Cluster details`

Review one subsystem.

- top members or dependencies
- incoming and outgoing link pressure
- selected member or dependency lens

### `Inspect`

Inspect one member or module.

- focused one-hop dependency lens
- centered viewport on the selected node
- raw graph available when you need graph-level debugging

This keeps the UI useful on large repositories without forcing the full
dependency graph on screen all the time.

## What MorphArch helps you review

- **Drift over time**: see when coupling, cycles, or boundary pressure get worse
- **Repository structure**: start from grouped clusters instead of a raw node graph
- **Hotspots and impact**: identify risky modules and inspect likely downstream effects
- **Project-specific architecture rules**: define boundaries, ignore rules, scan heuristics, clustering, and presentation in `morpharch.toml`

## Who tends to use it

- **Architects** use it to review boundaries and structural pressure.
- **Tech leads** use it to monitor health changes across commits and justify cleanup work.
- **Developers** use it to answer questions like who depends on a module and what it pulls in.

## What is configurable

MorphArch works with zero configuration, but you can override:

- ignore paths and presets
- scan heuristics such as package depth and external dependency visibility
- scoring weights and thresholds
- boundary rules and exemptions
- clustering strategy, families, rules, and constraints
- presentation aliases, kind mode, and color mode

See the [Configuration Guide](./guides/configuration) for the full reference.

## Next steps

1. Read the [Installation](./installation) guide.
2. Follow the [Quick Start](./quick-start).
3. Learn how the [pipeline works](./concepts/how-it-works).
