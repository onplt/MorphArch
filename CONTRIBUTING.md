# Contributing to MorphArch

Thank you for your interest in contributing to MorphArch. This guide covers
everything you need to get started.

## Prerequisites

- **Rust 1.88 or later** (the project's MSRV)
- **A C compiler** (required by `rusqlite` bundled SQLite and tree-sitter grammars)
  - Linux: `gcc` or `clang` (install via your package manager)
  - macOS: Xcode Command Line Tools (`xcode-select --install`)
  - Windows: MSVC Build Tools (install via Visual Studio Installer)
- **Git**

## Getting Started

1. Fork the repository on GitHub.

2. Clone your fork:

   ```sh
   git clone https://github.com/<your-username>/morpharch.git
   cd morpharch
   ```

3. Build the project:

   ```sh
   cargo build
   ```

4. Run the test suite:

   ```sh
   cargo test
   ```

5. Create a branch for your work:

   ```sh
   git checkout -b your-branch-name
   ```

## Development Workflow

### Building

```sh
cargo build          # Debug build
cargo build --release # Optimized build
```

### Running

```sh
cargo run -- <arguments>
```

### Testing

```sh
cargo test           # Run all tests
cargo test <name>    # Run a specific test
```

### Formatting

The project uses `rustfmt` with the configuration in `.rustfmt.toml`. Format
your code before committing:

```sh
cargo fmt
```

CI will reject code that does not pass `cargo fmt --check`.

### Linting

```sh
cargo clippy --all-targets
```

CI treats all Clippy warnings as errors.

### Documentation

```sh
cargo doc --no-deps --open
```

## Pull Request Process

1. Ensure your changes compile on the MSRV (Rust 1.88).
2. Run `cargo fmt`, `cargo clippy`, and `cargo test` locally.
3. Write clear, descriptive commit messages.
4. Open a pull request against the `main` branch.
5. Fill out the pull request template.
6. Wait for CI to pass and a maintainer to review.

### Commit Messages

Use concise, imperative-mood commit messages:

- `feat: add Python import resolver`
- `fix: handle cyclic dependencies in graph walk`
- `refactor: extract scoring logic into separate module`
- `docs: update CONTRIBUTING with MSRV note`
- `test: add integration test for incremental scan`

## Code Style

- Maximum line width is 100 characters (enforced by rustfmt).
- Use `anyhow` for error propagation in application code.
- Prefer returning `Result` over panicking.
- Add doc comments (`///`) to all public items.
- Keep modules focused: one responsibility per module.

## Architecture Notes

MorphArch is a binary crate structured around these core areas:

- **Git scanning** -- repository traversal via `gix`
- **Parsing** -- language-aware import extraction with safe fast paths and AST fallback
- **Graph** -- `petgraph`-based dependency graph with drift scoring
- **Storage** -- repo-scoped SQLite persistence via frames, checkpoints, and scan state
- **TUI** -- `ratatui` + `crossterm` animated graph renderer

When adding a new language parser, follow the existing pattern:

1. add language detection
2. implement a safe fast path when possible
3. add AST fallback support when needed
4. cover false-positive cases with tests

## Reporting Issues

- Use the bug report template for bugs.
- Use the feature request template for new ideas.
- Check existing issues before opening a new one.

## License

By contributing, you agree that your contributions will be licensed under the
same terms as the project: MIT OR Apache-2.0.
