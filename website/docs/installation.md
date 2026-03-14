# Installation

MorphArch is distributed as a local CLI binary for architecture analysis.

## Prerequisites

- No external Git CLI is required for normal use
- Rust is only required if you install from source or with Cargo

---

## Quick Install

### Cargo

```bash
cargo install morpharch
```

### Linux / macOS

```bash
curl -fsSL https://raw.githubusercontent.com/onplt/morpharch/main/install.sh | sh
```

### Windows PowerShell

```powershell
irm https://raw.githubusercontent.com/onplt/morpharch/main/install.ps1 | iex
```

---

## Other Install Methods

### Homebrew

```bash
brew install onplt/morpharch
```

### npm

```bash
npm install -g morpharch
```

### cargo-binstall

```bash
cargo binstall morpharch
```

### Scoop

```powershell
scoop bucket add morpharch https://github.com/onplt/scoop-morpharch
scoop install morpharch
```

### Docker

```bash
docker run --rm -v .:/repo ghcr.io/onplt/morpharch scan . -n 1
```

---

## Build From Source

```bash
git clone https://github.com/onplt/morpharch.git
cd morpharch
cargo build --release
```

The binary will be available at:

```text
target/release/morpharch
```

---

## Verify the Install

```bash
morpharch --version
```

Then try:

```bash
morpharch scan . -n 1
morpharch watch . -n 50
```

You should see a short scan complete successfully, then land on the map view
and be able to open clusters and inspect individual members.
