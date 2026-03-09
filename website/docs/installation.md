# Installation

MorphArch is distributed as a high-performance Rust binary. You can install it using several methods.

## Prerequisites

- **Rust Toolchain**: You need the Rust compiler (v1.85+) and Cargo installed. If you don't have it, install it from [rustup.rs](https://rustup.rs/).
- **Zero External Dependencies**: Because MorphArch is powered by the `gitoxide` (`gix`) pure-Rust Git engine, you do **not** even need the `git` CLI installed on your system to run it!

---

## Installation Methods

### From crates.io (Recommended)

The easiest way to install MorphArch is via Cargo:

```bash
cargo install morpharch
```

### From Source

If you want the latest features from the `main` branch:

```bash
git clone https://github.com/onplt/morpharch.git
cd morpharch
cargo build --release
```

The binary will be available at `./target/release/morpharch`. You can move it to your `/usr/local/bin` or equivalent.

### Homebrew (macOS/Linux)

:::tip Coming Soon
We are working on a Homebrew formula. For now, please use the `cargo install` method.
:::

---

## Verifying Installation

Run the following command to verify that MorphArch is installed correctly:

```bash
morpharch --version
```

If you see the version number, you're ready to visualize your architecture!
