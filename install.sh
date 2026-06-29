#!/bin/sh
# koma installer — https://koma.run
# Usage:
#   curl -fsSL https://koma.run/install.sh | sh
#   curl -fsSL https://koma.run/install.sh | sh -s -- --with-research
#
# Environment overrides:
#   KOMA_RELEASE_BASE   override the base download URL
#   KOMA_INSTALL_DIR    override the install directory (default ~/.local/bin)

set -e

# Base download URL (GitHub latest release). Override with KOMA_RELEASE_BASE=...
KOMA_RELEASE_BASE="${KOMA_RELEASE_BASE:-https://github.com/aula-id/koma/releases/latest/download}"

INSTALL_DIR="${KOMA_INSTALL_DIR:-$HOME/.local/bin}"

WITH_RESEARCH=0
for arg in "$@"; do
    case "$arg" in
        --with-research) WITH_RESEARCH=1 ;;
    esac
done

# ---------------------------------------------------------------------------
# OS detection
# ---------------------------------------------------------------------------
_os=$(uname -s)
case "$_os" in
    Linux)  os="linux"  ;;
    Darwin) os="darwin" ;;
    *)
        echo "ERROR: unsupported operating system: $_os" >&2
        echo "koma currently supports Linux and macOS." >&2
        exit 1
        ;;
esac

# ---------------------------------------------------------------------------
# Architecture detection
# ---------------------------------------------------------------------------
_arch=$(uname -m)
case "$_arch" in
    x86_64|amd64)   arch="x86_64" ;;
    aarch64|arm64)  arch="arm64"  ;;
    *)
        echo "ERROR: unsupported architecture: $_arch" >&2
        echo "koma currently supports x86_64 and arm64." >&2
        exit 1
        ;;
esac

# ---------------------------------------------------------------------------
# Build asset URL
# ---------------------------------------------------------------------------
# Release artifacts (see .github/workflows/release.yml) are published per
# platform with these names; install them as "koma" at $INSTALL_DIR/koma:
#   linux  x86_64 -> koma-linux-x64
#   linux  arm64  -> koma-linux-arm64
#   darwin arm64  -> koma-darwin-arm64   (Apple Silicon)
#   darwin x86_64 -> koma-darwin-x64     (Intel Mac)
case "${os}/${arch}" in
    linux/x86_64)  asset="koma-linux-x64"    ;;
    linux/arm64)   asset="koma-linux-arm64"  ;;
    darwin/arm64)  asset="koma-darwin-arm64" ;;
    darwin/x86_64) asset="koma-darwin-x64"   ;;
    *)
        echo "ERROR: no prebuilt koma binary for ${os}/${arch}." >&2
        echo "Supported: linux x86_64, linux arm64, macOS arm64, macOS x86_64." >&2
        exit 1
        ;;
esac
url="${KOMA_RELEASE_BASE}/${asset}"

echo "koma installer — detected ${os}/${arch}"
echo "  url:      $url"
echo "  install:  $INSTALL_DIR/koma"
echo ""

# ---------------------------------------------------------------------------
# Temp file + cleanup trap
# ---------------------------------------------------------------------------
tmp=$(mktemp)
cleanup() {
    rm -f "$tmp"
}
trap cleanup EXIT

# ---------------------------------------------------------------------------
# Download
# ---------------------------------------------------------------------------
echo "Downloading koma..."
if command -v curl > /dev/null 2>&1; then
    curl -fsSL "$url" -o "$tmp" || {
        echo "ERROR: download failed from $url" >&2
        exit 1
    }
elif command -v wget > /dev/null 2>&1; then
    wget -qO "$tmp" "$url" || {
        echo "ERROR: download failed from $url" >&2
        exit 1
    }
else
    echo "ERROR: neither curl nor wget found; please install one and retry." >&2
    exit 1
fi

chmod +x "$tmp"

# ---------------------------------------------------------------------------
# Install — fall back to sudo if the directory is not user-writable
# ---------------------------------------------------------------------------
mkdir -p "$INSTALL_DIR" 2>/dev/null || true
if [ -w "$INSTALL_DIR" ]; then
    mv "$tmp" "$INSTALL_DIR/koma"
else
    if [ "$(id -u)" = "0" ]; then
        mv "$tmp" "$INSTALL_DIR/koma"
    else
        echo "  $INSTALL_DIR is not writable; using sudo for install step."
        sudo mv "$tmp" "$INSTALL_DIR/koma"
        sudo chmod +x "$INSTALL_DIR/koma"
    fi
fi

# ---------------------------------------------------------------------------
# macOS: clear the Gatekeeper quarantine flag on the downloaded binary so it
# runs without the "unidentified developer" prompt. Best-effort — ignore if
# xattr is unavailable or the attribute was never set.
# ---------------------------------------------------------------------------
if [ "$os" = "darwin" ] && command -v xattr > /dev/null 2>&1; then
    if [ -w "$INSTALL_DIR/koma" ]; then
        xattr -d com.apple.quarantine "$INSTALL_DIR/koma" 2>/dev/null || true
    else
        sudo xattr -d com.apple.quarantine "$INSTALL_DIR/koma" 2>/dev/null || true
    fi
fi

# ---------------------------------------------------------------------------
# Optional: provision Python research environment
# ---------------------------------------------------------------------------
if [ "$WITH_RESEARCH" = "1" ]; then
    echo ""
    echo "Provisioning full internet mode environment (downloads ~80MB Firefox)..."
    "$INSTALL_DIR/koma" --internet-fullmode-install
fi

# ---------------------------------------------------------------------------
# Success
# ---------------------------------------------------------------------------
echo ""
echo "koma installed to $INSTALL_DIR/koma"
echo ""
echo "  Run 'koma' to start."
echo "  Re-run this installer with --with-research (or run"
echo "  'koma --internet-fullmode-install') to enable full internet mode."
echo ""

# Warn if install dir may not be on PATH
case ":${PATH}:" in
    *":${INSTALL_DIR}:"*) ;;
    *)
        echo "  NOTE: $INSTALL_DIR does not appear to be in your PATH."
        echo "  Add the following to your shell profile and restart your terminal:"
        echo "    export PATH=\"$INSTALL_DIR:\$PATH\""
        echo ""
        ;;
esac

echo "  https://koma.run"
