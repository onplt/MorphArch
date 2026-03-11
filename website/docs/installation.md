# Installation

MorphArch is distributed as a high-performance Rust binary. You can install it using several methods.

## Prerequisites

- **Zero External Dependencies**: Because MorphArch is powered by the `gitoxide` (`gix`) pure-Rust Git engine, you do **not** even need the `git` CLI installed on your system to run it!
- **Rust Toolchain** (only for `cargo install` or building from source): You need the Rust compiler (v1.88+) and Cargo installed. If you don't have it, install it from [rustup.rs](https://rustup.rs/).

---

## Quick Install (Recommended)

The fastest way to install MorphArch — no Rust toolchain required.

**Linux / macOS:**

```bash
curl -fsSL https://raw.githubusercontent.com/onplt/morpharch/main/install.sh | sh
```

**Windows (PowerShell):**

```powershell
irm https://raw.githubusercontent.com/onplt/morpharch/main/install.ps1 | iex
```

These scripts automatically detect your platform, download the correct pre-built binary from GitHub Releases, and place it in your PATH.

---

## Installation Methods

### Homebrew (macOS/Linux)

```bash
brew install onplt/morpharch
```

### npm

```bash
npm install -g morpharch
```

### From crates.io

```bash
cargo install morpharch
```

### cargo-binstall

If you have [cargo-binstall](https://github.com/cargo-bins/cargo-binstall), you can skip the compilation step:

```bash
cargo binstall morpharch
```

### Scoop (Windows)

```powershell
scoop bucket add morpharch https://github.com/onplt/scoop-morpharch
scoop install morpharch
```

### AUR (Arch Linux)

```bash
yay -S morpharch-bin
```

### DEB / RPM

Download `.deb` or `.rpm` packages directly from the [GitHub Releases](https://github.com/onplt/morpharch/releases) page.

### Docker

```bash
docker run --rm -v .:/repo ghcr.io/onplt/morpharch scan .
```

### From Source

If you want the latest features from the `main` branch:

```bash
git clone https://github.com/onplt/morpharch.git
cd morpharch
cargo build --release
```

The binary will be available at `./target/release/morpharch`. You can move it to your `/usr/local/bin` or equivalent.

---

## Verifying Installation

Run the following command to verify that MorphArch is installed correctly:

```bash
morpharch --version
```

If you see the version number, you're ready to visualize your architecture!
