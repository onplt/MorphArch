# Configuration Guide

MorphArch is designed to be **"zero-config"** out of the box. It relies on topological analysis rather than strict string-matching to evaluate your architecture, meaning it works automatically for almost any monorepo without tedious setup.

## How it works without a config file

### 1. Monorepo Detection
MorphArch is built for monorepos. It natively understands and parses dependencies by treating top-level folders or standard workspace structures as discrete packages.
- It automatically processes code in Rust, TypeScript, JavaScript, Python, and Go.

### 2. Topological Layering (Boundary Rules)
Instead of forcing you to write regex rules in a `morpharch.toml` file to declare "App cannot depend on Lib", MorphArch uses **Topological Sorting**.
- It analyzes the natural flow of your dependencies.
- It detects **Back-edges** (when a low-level module unexpectedly imports a high-level module that depends on it).
- This means boundary violations are detected algorithmically without manual configuration!

### 3. Entry Point Detection
MorphArch automatically forgives natural entry points from being flagged as "God Modules" or "Fragile".
- Any file or module named `main`, `index`, `app`, `lib`, or `mod` is recognized as a composition root.

## Environment Variables

For CI/CD environments, you can override system-level settings using environment variables:

- `MORPHARCH_DB_PATH`: Path to the SQLite database (default: `~/.morpharch/morpharch.db`).

---

## Future Roadmap

While MorphArch is currently zero-config to maximize ease of use, future versions may introduce a `morpharch.toml` file to allow teams to define custom architectural exceptions, strict boundary regexes, and specific density thresholds tailored to their unique domain logic.
