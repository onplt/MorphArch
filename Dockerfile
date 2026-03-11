# MorphArch Docker image
# Build: docker build -t morpharch .
# Usage: docker run --rm -v /path/to/repo:/repo morpharch scan .
#        docker run --rm -it -v /path/to/repo:/repo morpharch watch .
#
# Note: The 'watch' command (TUI) requires -it flags for interactive terminal.

# ── Stage 1: Build ──
FROM rust:1.88-bookworm AS builder

WORKDIR /build

# Copy manifests first for dependency caching
COPY Cargo.toml Cargo.lock ./

# Create dummy src to cache dependency compilation
RUN mkdir src && \
    echo "fn main() {}" > src/main.rs && \
    cargo build --release 2>/dev/null || true && \
    rm -rf src

# Copy actual source and build
COPY src/ src/
RUN touch src/main.rs && \
    cargo build --release --locked && \
    strip target/release/morpharch

# ── Stage 2: Runtime ──
FROM debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends \
      ca-certificates \
      git && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/morpharch /usr/local/bin/morpharch

WORKDIR /repo

ENTRYPOINT ["morpharch"]
CMD ["--help"]
