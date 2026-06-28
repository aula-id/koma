#!/bin/sh
# koma native build — https://koma.run
#
# Builds the koma binary FROM SOURCE for the HOST machine (native compile, not
# cross, not docker). Run it from the repo root.
#
# Usage:
#   ./build.sh              build target/release/koma for this host
#   ./build.sh --install    build, then copy the binary to $INSTALL_DIR
#
# Environment overrides:
#   KOMA_INSTALL=1          same as --install
#   KOMA_INSTALL_DIR        install destination (default /usr/local/bin)

set -e

INSTALL_DIR="${KOMA_INSTALL_DIR:-/usr/local/bin}"

DO_INSTALL=0
if [ "${KOMA_INSTALL:-0}" = "1" ]; then
    DO_INSTALL=1
fi
for arg in "$@"; do
    case "$arg" in
        --install) DO_INSTALL=1 ;;
        -h|--help)
            echo "Usage: ./build.sh [--install]"
            echo "  --install   copy the built binary to \$INSTALL_DIR (default $INSTALL_DIR)"
            exit 0
            ;;
        *)
            echo "ERROR: unknown argument: $arg" >&2
            echo "Usage: ./build.sh [--install]" >&2
            exit 1
            ;;
    esac
done

# ---------------------------------------------------------------------------
# Host detection: OS (uname -s) + arch (uname -m) -> rust target triple.
# ---------------------------------------------------------------------------
_os=$(uname -s)
_arch=$(uname -m)

target=""
case "$_os" in
    Linux)
        case "$_arch" in
            x86_64|amd64)   target="x86_64-unknown-linux-gnu"  ;;
            aarch64|arm64)  target="aarch64-unknown-linux-gnu" ;;
        esac
        ;;
    Darwin)
        case "$_arch" in
            # Apple Silicon (M-series) only — Intel Macs are not a supported host.
            arm64|aarch64)  target="aarch64-apple-darwin" ;;
        esac
        ;;
esac

if [ -z "$target" ]; then
    echo "ERROR: unsupported host: ${_os}/${_arch}" >&2
    echo "" >&2
    echo "build.sh supports a native build on:" >&2
    echo "  - Linux x86_64 (amd64)" >&2
    echo "  - Linux aarch64 (arm64)" >&2
    echo "  - macOS arm64 (Apple M-series)" >&2
    echo "" >&2
    echo "Detected ${_os}/${_arch}, which is not one of these." >&2
    exit 1
fi

echo "koma build — host ${_os}/${_arch} (target ${target})"

# ---------------------------------------------------------------------------
# Toolchain check: cargo must be on PATH.
# ---------------------------------------------------------------------------
if ! command -v cargo > /dev/null 2>&1; then
    echo "ERROR: cargo (Rust) was not found on your PATH." >&2
    echo "" >&2
    echo "Install Rust via rustup, then re-run ./build.sh :" >&2
    echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh" >&2
    echo "" >&2
    echo "After install, restart your shell (or 'source \$HOME/.cargo/env')." >&2
    echo "More: https://rustup.rs" >&2
    exit 1
fi

# ---------------------------------------------------------------------------
# Build (native, release).
# ---------------------------------------------------------------------------
echo "Building koma (cargo build --release -p agent)..."
echo "  this is a from-source native build; first run can take a few minutes."
cargo build --release -p agent

bin="target/release/koma"
if [ ! -f "$bin" ]; then
    echo "ERROR: build reported success but $bin is missing." >&2
    echo "Are you running ./build.sh from the repo root?" >&2
    exit 1
fi

# ---------------------------------------------------------------------------
# macOS: strip the Gatekeeper quarantine flag so the binary runs unprompted.
# Best-effort — ignore if xattr is missing or the attribute was never set.
# ---------------------------------------------------------------------------
if [ "$_os" = "Darwin" ] && command -v xattr > /dev/null 2>&1; then
    xattr -d com.apple.quarantine "$bin" 2>/dev/null || true
fi

echo ""
echo "Build complete: $bin"

# ---------------------------------------------------------------------------
# Optional install to $INSTALL_DIR.
# When not requested, just print the command so nothing is forced.
# ---------------------------------------------------------------------------
if [ "$DO_INSTALL" = "1" ]; then
    echo ""
    echo "Installing to $INSTALL_DIR/koma ..."
    if [ -w "$INSTALL_DIR" ]; then
        cp "$bin" "$INSTALL_DIR/koma"
        chmod +x "$INSTALL_DIR/koma"
    else
        echo "  $INSTALL_DIR is not writable; using sudo for the copy step."
        sudo cp "$bin" "$INSTALL_DIR/koma"
        sudo chmod +x "$INSTALL_DIR/koma"
    fi
    # Re-strip quarantine on the installed copy (macOS).
    if [ "$_os" = "Darwin" ] && command -v xattr > /dev/null 2>&1; then
        if [ -w "$INSTALL_DIR/koma" ]; then
            xattr -d com.apple.quarantine "$INSTALL_DIR/koma" 2>/dev/null || true
        else
            sudo xattr -d com.apple.quarantine "$INSTALL_DIR/koma" 2>/dev/null || true
        fi
    fi
    echo "Installed: $INSTALL_DIR/koma"
    echo "Run 'koma' to start."
else
    echo ""
    echo "  Run it:        ./$bin"
    echo "  Install it:    ./build.sh --install      (copies to $INSTALL_DIR)"
    echo "  Or by hand:    sudo cp $bin $INSTALL_DIR/koma"
fi

echo ""
echo "  https://koma.run"
