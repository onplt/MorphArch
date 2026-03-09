# Security & Privacy

MorphArch is designed with a **privacy-first, local-only** philosophy. We understand that your source code is your most valuable asset, and we treat it with the highest level of security.

## 100% Local Analysis

Unlike many "SaaS" architecture tools, MorphArch does **not** upload your source code, AST metadata, or Git history to any remote server.

- **Offline Execution**: MorphArch can run entirely in air-gapped environments without an internet connection.
- **No Telemetry**: We do not collect usage statistics, IP addresses, or project names.
- **In-Memory Parsing**: Abstract Syntax Trees (AST) are built in system RAM and discarded immediately after the dependency edges are extracted.

---

## Data Persistence

MorphArch stores its analysis results in a local SQLite database located at `~/.morpharch/morpharch.db`.

### What IS stored locally?
- **Git Metadata**: Commit hashes, timestamps, commit messages, author names, and author emails (used exclusively to render the TUI timeline).
- **Topology Data**: Metadata about detected packages (names and relative paths) and dependency edges (counts of import statements between packages).
- **Scores**: Architectural drift scores and sub-metrics.

### What is NEVER stored or transmitted?
- Actual source code content (ASTs are built in memory and dropped).
- Sensitive information like API keys, secrets, or environment variables.
- Code blocks or function implementations.

---

## Open Source Auditability

As an open-source project, our security claims are fully auditable. You can inspect our [source code on GitHub](https://github.com/onplt/morpharch) to verify exactly how your data is handled.

- **Git Engine**: We use the pure-Rust `gix` (gitoxide) library for secure, high-performance Git operations.
- **No Hidden Dependencies**: We carefully vet our crate dependencies to ensure a minimal and secure attack surface.

:::tip Enterprise Compliance
If your organization requires a formal security assessment or a signed statement of privacy, please [open a discussion on GitHub](https://github.com/onplt/morpharch/discussions).
:::
