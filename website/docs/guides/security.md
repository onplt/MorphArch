# Security & Privacy

MorphArch is designed with a **privacy-first, local-only** philosophy. Your
source code stays on your machine.

## 100% Local Analysis

MorphArch does **not** upload your source code, Git history, or dependency
metadata to a remote service.

- **Offline execution**: works in air-gapped environments
- **No telemetry**: no usage analytics, IP collection, or project-name tracking
- **In-memory parsing**: source is parsed in memory and reduced to dependency
  metadata only

---

## Data Persistence

MorphArch stores its analysis results in:

- a local SQLite database at `~/.morpharch/morpharch.db`
- a local subtree cache under `~/.morpharch/subtree-cache/`

### What is stored locally?

- **Git metadata**: commit hashes, timestamps, messages, author names, and
  author emails
- **Topology data**: module labels, dependency edges, and weights
- **Scores**: health, drift, and related sub-metrics
- **Scan cache**: repo-scoped history frames, checkpoints, and subtree cache
  entries used for replay and incremental updates

### What is never stored or transmitted?

- raw source code
- API keys, secrets, or environment variables
- full AST payloads

---

## Open Source Auditability

As an open-source project, these claims are auditable. You can inspect the
[source code on GitHub](https://github.com/onplt/morpharch) to verify exactly
how your data is handled.

- **Git engine**: MorphArch uses the pure-Rust `gix` library for Git operations.
- **No hidden services**: MorphArch is a local CLI and TUI, not a hosted SaaS.

:::tip Enterprise compliance
If your organization requires a formal security assessment or a signed privacy
statement, please [open a discussion on GitHub](https://github.com/onplt/morpharch/discussions).
:::
