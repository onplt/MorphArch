#!/bin/sh
# MorphArch installer script
# Usage: curl -fsSL https://raw.githubusercontent.com/onplt/morpharch/main/install.sh | sh
#
# Environment variables:
#   MORPHARCH_VERSION   - specific version to install (default: latest)
#   MORPHARCH_INSTALL   - installation directory (default: $HOME/.morpharch/bin)

set -eu

REPO="onplt/morpharch"
BINARY="morpharch"

# ── Detect platform ──

detect_platform() {
    OS=$(uname -s)
    ARCH=$(uname -m)

    case "$OS" in
        Linux)  OS_NAME="linux" ;;
        Darwin) OS_NAME="macos" ;;
        *)
            echo "Error: Unsupported operating system: $OS" >&2
            echo "MorphArch supports Linux and macOS." >&2
            exit 1
            ;;
    esac

    case "$ARCH" in
        x86_64|amd64)  ARCH_NAME="x86_64" ;;
        aarch64|arm64) ARCH_NAME="aarch64" ;;
        *)
            echo "Error: Unsupported architecture: $ARCH" >&2
            echo "MorphArch supports x86_64 and aarch64/arm64." >&2
            exit 1
            ;;
    esac

    ASSET_NAME="${BINARY}-${OS_NAME}-${ARCH_NAME}.tar.gz"
}

# ── Determine version ──

get_version() {
    if [ -n "${MORPHARCH_VERSION:-}" ]; then
        VERSION="$MORPHARCH_VERSION"
        return
    fi

    echo "Fetching latest version..."
    if command -v curl >/dev/null 2>&1; then
        VERSION=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
            | grep '"tag_name"' | head -1 | sed 's/.*"v\(.*\)".*/\1/')
    elif command -v wget >/dev/null 2>&1; then
        VERSION=$(wget -qO- "https://api.github.com/repos/${REPO}/releases/latest" \
            | grep '"tag_name"' | head -1 | sed 's/.*"v\(.*\)".*/\1/')
    else
        echo "Error: curl or wget is required." >&2
        exit 1
    fi

    if [ -z "$VERSION" ]; then
        echo "Error: Could not determine latest version." >&2
        exit 1
    fi
}

# ── Download helper ──

download() {
    URL="$1"
    OUTPUT="$2"
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$URL" -o "$OUTPUT"
    elif command -v wget >/dev/null 2>&1; then
        wget -q "$URL" -O "$OUTPUT"
    fi
}

# ── Main ──

main() {
    detect_platform
    get_version

    INSTALL_DIR="${MORPHARCH_INSTALL:-$HOME/.morpharch/bin}"
    DOWNLOAD_URL="https://github.com/${REPO}/releases/download/v${VERSION}/${ASSET_NAME}"
    CHECKSUM_URL="https://github.com/${REPO}/releases/download/v${VERSION}/SHA256SUMS.txt"

    echo "Installing morpharch v${VERSION} (${OS_NAME}/${ARCH_NAME})..."
    echo "  From: ${DOWNLOAD_URL}"
    echo "  To:   ${INSTALL_DIR}/${BINARY}"

    # Create temp directory
    TMP_DIR=$(mktemp -d)
    trap 'rm -rf "$TMP_DIR"' EXIT

    # Download archive
    echo "Downloading..."
    download "$DOWNLOAD_URL" "${TMP_DIR}/${ASSET_NAME}"

    # Verify checksum if SHA256SUMS.txt is available
    if download "$CHECKSUM_URL" "${TMP_DIR}/SHA256SUMS.txt" 2>/dev/null; then
        echo "Verifying checksum..."
        EXPECTED=$(grep "$ASSET_NAME" "${TMP_DIR}/SHA256SUMS.txt" | awk '{print $1}')
        if [ -n "$EXPECTED" ]; then
            if command -v sha256sum >/dev/null 2>&1; then
                ACTUAL=$(sha256sum "${TMP_DIR}/${ASSET_NAME}" | awk '{print $1}')
            elif command -v shasum >/dev/null 2>&1; then
                ACTUAL=$(shasum -a 256 "${TMP_DIR}/${ASSET_NAME}" | awk '{print $1}')
            else
                echo "Warning: sha256sum/shasum not found, skipping verification." >&2
                ACTUAL="$EXPECTED"
            fi

            if [ "$ACTUAL" != "$EXPECTED" ]; then
                echo "Error: Checksum verification failed!" >&2
                echo "  Expected: $EXPECTED" >&2
                echo "  Actual:   $ACTUAL" >&2
                exit 1
            fi
            echo "Checksum verified."
        fi
    else
        echo "Warning: Could not download checksums, skipping verification." >&2
    fi

    # Extract
    echo "Extracting..."
    tar -xzf "${TMP_DIR}/${ASSET_NAME}" -C "${TMP_DIR}"

    # Install
    mkdir -p "$INSTALL_DIR"
    mv "${TMP_DIR}/${BINARY}" "${INSTALL_DIR}/${BINARY}"
    chmod +x "${INSTALL_DIR}/${BINARY}"

    echo ""
    echo "morpharch v${VERSION} installed successfully to ${INSTALL_DIR}/${BINARY}"

    # Check if install dir is in PATH
    case ":$PATH:" in
        *":${INSTALL_DIR}:"*)
            echo ""
            echo "Run 'morpharch --version' to verify."
            ;;
        *)
            echo ""
            echo "Add the following to your shell profile (~/.bashrc, ~/.zshrc, etc.):"
            echo ""
            echo "  export PATH=\"${INSTALL_DIR}:\$PATH\""
            echo ""
            echo "Then restart your shell or run:"
            echo ""
            echo "  export PATH=\"${INSTALL_DIR}:\$PATH\""
            ;;
    esac
}

main
