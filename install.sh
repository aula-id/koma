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
    MINGW*|MSYS*|CYGWIN*) os="windows" ;;
    *)
        echo "ERROR: unsupported operating system: $_os" >&2
        echo "koma currently supports Linux, macOS, and Windows (via Git Bash)." >&2
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
# platform with these names; install them as "koma" (or "koma.exe" on
# Windows) at $INSTALL_DIR:
#   linux   x86_64 -> koma-linux-x64
#   linux   arm64  -> koma-linux-arm64
#   darwin  arm64  -> koma-darwin-arm64   (Apple Silicon)
#   darwin  x86_64 -> koma-darwin-x64     (Intel Mac)
#   windows x86_64 -> koma-windows-x64.exe
case "${os}/${arch}" in
    linux/x86_64)   asset="koma-linux-x64"       ;;
    linux/arm64)    asset="koma-linux-arm64"     ;;
    darwin/arm64)   asset="koma-darwin-arm64"    ;;
    darwin/x86_64)  asset="koma-darwin-x64"      ;;
    windows/x86_64) asset="koma-windows-x64.exe" ;;
    windows/arm64)
        echo "ERROR: no prebuilt koma binary for windows/arm64." >&2
        echo "koma on Windows currently supports x86_64 only." >&2
        exit 1
        ;;
    *)
        echo "ERROR: no prebuilt koma binary for ${os}/${arch}." >&2
        echo "Supported: linux x86_64, linux arm64, macOS arm64, macOS x86_64, windows x86_64." >&2
        exit 1
        ;;
esac
url="${KOMA_RELEASE_BASE}/${asset}"

# Install filename: Windows needs the .exe extension for the shell/PATHEXT
# lookup to resolve it; every other platform installs as extensionless "koma".
bin_name="koma"
if [ "$os" = "windows" ]; then
    bin_name="koma.exe"
fi

echo "koma installer — detected ${os}/${arch}"
echo "  url:      $url"
echo "  install:  $INSTALL_DIR/$bin_name"
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
#
# On Linux: install the real ELF as koma.bin and a shell launcher as koma.
# The launcher preflights shared-library availability (missing webkit/gtk,
# too-old glibc) before the dynamic linker crashes.  macOS / Windows:
# single binary named koma / koma.exe.
# ---------------------------------------------------------------------------
mkdir -p "$INSTALL_DIR" 2>/dev/null || true

if [ "$os" = "linux" ]; then
    # Linux: two-file layout — koma (launcher) + koma.bin (real ELF).
    _install_linux_binary() {
        mv "$tmp" "$INSTALL_DIR/koma.bin"
        # --- embedded launcher — keep in sync with packaging/linux/koma-launcher.sh ---
        cat > "$INSTALL_DIR/koma" << 'KOMA_LAUNCHER'
#!/bin/sh
# koma launcher — preflight shared libs before the dynamic linker crashes.
# Keep in sync with packaging/linux/koma-launcher.sh
set -e
resolve_dir() {
    _src="$0"
    while [ -L "$_src" ]; do
        _dir="$(cd -P "$(dirname -- "$_src")" && pwd)"
        _src="$(readlink -- "$_src")"
        case "$_src" in
            /*) ;;
            *)  _src="$_dir/$_src" ;;
        esac
    done
    cd -P "$(dirname -- "$_src")" && pwd
}
DIR="$(resolve_dir)"
BIN="$DIR/koma.bin"
if [ ! -f "$BIN" ]; then
    echo "koma: $BIN not found." >&2
    echo "The koma launcher expects the real binary alongside itself as koma.bin." >&2
    exit 1
fi
if [ ! -x "$BIN" ]; then
    echo "koma: $BIN is not executable." >&2
    echo "Run: chmod +x $BIN" >&2
    exit 1
fi
ldd_out=""
ldd_ok=0
if command -v ldd >/dev/null 2>&1; then
    ldd_out="$(ldd "$BIN" 2>&1)" && ldd_ok=1 || ldd_ok=0
fi
if [ "$ldd_ok" = "1" ] && echo "$ldd_out" | grep -q "not a dynamic executable"; then
    exec "$BIN" "$@"
fi
if [ "$ldd_ok" = "1" ] && ! echo "$ldd_out" | grep -q "not found"; then
    exec "$BIN" "$@"
fi
glibc_needed=""
missing_libs=""
if [ "$ldd_out" != "" ]; then
    glibc_needed=$(echo "$ldd_out" | grep -oE 'GLIBC_[0-9]+\.[0-9]+' | sort -uV | tr '\n' ' ')
    missing_libs=$(echo "$ldd_out" | grep "not found" | grep -v 'GLIBC_' | awk '{print $1}' | sort -u)
fi
if [ -n "$glibc_needed" ]; then
    echo "" >&2
    echo "koma: prebuilt binary requires a newer glibc than this system provides." >&2
    echo "" >&2
    if ldd --version >/dev/null 2>&1; then
        _glibcv="$(ldd --version 2>&1 | head -1)"
        echo "  Host:    $_glibcv" >&2
    fi
    echo "  Need:    glibc $glibc_needed" >&2
    echo "" >&2
    echo "You have two options:" >&2
    echo "" >&2
    echo "  1. Build from source on this machine (links against your local glibc):" >&2
    echo "     git clone https://github.com/aula-id/koma.git && cd koma" >&2
    echo "     ./build.sh" >&2
    echo "" >&2
    echo "  2. Use Ubuntu 22.04 LTS or newer (ships glibc 2.35+)." >&2
    echo "" >&2
    if [ -n "$missing_libs" ]; then
        echo "Additional missing libraries:" >&2
        echo "$missing_libs" | sed 's/^/  /' >&2
        echo "" >&2
    fi
    exit 127
fi
if [ -n "$missing_libs" ]; then
    has_webkit=0
    has_gtk=0
    echo "$missing_libs" | grep -q 'libwebkit2gtk' && has_webkit=1
    echo "$missing_libs" | grep -q 'libgtk-3' && has_gtk=1
    if [ "$has_webkit" = "1" ] || [ "$has_gtk" = "1" ]; then
        echo "" >&2
        echo "koma: missing system libraries required for the GUI." >&2
        echo "" >&2
        echo "Install them with your package manager:" >&2
        echo "" >&2
        if command -v apt-get >/dev/null 2>&1; then
            echo "  sudo apt-get install -y libwebkit2gtk-4.1-0 libgtk-3-0" >&2
        elif command -v dnf >/dev/null 2>&1; then
            echo "  sudo dnf install webkit2gtk4.1 gtk3" >&2
        elif command -v pacman >/dev/null 2>&1; then
            echo "  sudo pacman -S webkit2gtk-4.1 gtk3" >&2
        elif command -v zypper >/dev/null 2>&1; then
            echo "  sudo zypper install libwebkit2gtk-4_1-0 libgtk-3-0" >&2
        else
            echo "  # Debian/Ubuntu:" >&2
            echo "  sudo apt-get install -y libwebkit2gtk-4.1-0 libgtk-3-0" >&2
            echo "  # Fedora:" >&2
            echo "  sudo dnf install webkit2gtk4.1 gtk3" >&2
            echo "  # Arch:" >&2
            echo "  sudo pacman -S webkit2gtk-4.1 gtk3" >&2
        fi
        echo "" >&2
        echo "(Package names may vary by distro/version.)" >&2
        echo "" >&2
    else
        echo "" >&2
        echo "koma: missing shared libraries:" >&2
        echo "$ldd_out" | grep "not found" | sed 's/^/  /' >&2
        echo "" >&2
        echo "Install the missing libraries via your package manager." >&2
        echo "" >&2
    fi
    exit 127
fi
exec "$BIN" "$@"
KOMA_LAUNCHER
        chmod +x "$INSTALL_DIR/koma"
    }

    if [ -w "$INSTALL_DIR" ]; then
        _install_linux_binary
    else
        if [ "$(id -u)" = "0" ]; then
            _install_linux_binary
        else
            echo "  $INSTALL_DIR is not writable; using sudo for install step."
            _tmp_launcher=$(mktemp)
            cat > "$_tmp_launcher" << 'KOMA_LAUNCHER'
#!/bin/sh
# koma launcher — preflight shared libs before the dynamic linker crashes.
# Keep in sync with packaging/linux/koma-launcher.sh
set -e
resolve_dir() {
    _src="$0"
    while [ -L "$_src" ]; do
        _dir="$(cd -P "$(dirname -- "$_src")" && pwd)"
        _src="$(readlink -- "$_src")"
        case "$_src" in
            /*) ;;
            *)  _src="$_dir/$_src" ;;
        esac
    done
    cd -P "$(dirname -- "$_src")" && pwd
}
DIR="$(resolve_dir)"
BIN="$DIR/koma.bin"
if [ ! -f "$BIN" ]; then
    echo "koma: $BIN not found." >&2
    exit 1
fi
if [ ! -x "$BIN" ]; then
    echo "koma: $BIN is not executable." >&2
    echo "Run: chmod +x $BIN" >&2
    exit 1
fi
ldd_out=""
ldd_ok=0
if command -v ldd >/dev/null 2>&1; then
    ldd_out="$(ldd "$BIN" 2>&1)" && ldd_ok=1 || ldd_ok=0
fi
if [ "$ldd_ok" = "1" ] && echo "$ldd_out" | grep -q "not a dynamic executable"; then
    exec "$BIN" "$@"
fi
if [ "$ldd_ok" = "1" ] && ! echo "$ldd_out" | grep -q "not found"; then
    exec "$BIN" "$@"
fi
glibc_needed=""
missing_libs=""
if [ "$ldd_out" != "" ]; then
    glibc_needed=$(echo "$ldd_out" | grep -oE 'GLIBC_[0-9]+\.[0-9]+' | sort -uV | tr '\n' ' ')
    missing_libs=$(echo "$ldd_out" | grep "not found" | grep -v 'GLIBC_' | awk '{print $1}' | sort -u)
fi
if [ -n "$glibc_needed" ]; then
    echo "" >&2
    echo "koma: prebuilt binary requires a newer glibc than this system provides." >&2
    echo "" >&2
    if ldd --version >/dev/null 2>&1; then
        _glibcv="$(ldd --version 2>&1 | head -1)"
        echo "  Host:    $_glibcv" >&2
    fi
    echo "  Need:    glibc $glibc_needed" >&2
    echo "" >&2
    echo "You have two options:" >&2
    echo "" >&2
    echo "  1. Build from source on this machine (links against your local glibc):" >&2
    echo "     git clone https://github.com/aula-id/koma.git && cd koma" >&2
    echo "     ./build.sh" >&2
    echo "" >&2
    echo "  2. Use Ubuntu 22.04 LTS or newer (ships glibc 2.35+)." >&2
    echo "" >&2
    if [ -n "$missing_libs" ]; then
        echo "Additional missing libraries:" >&2
        echo "$missing_libs" | sed 's/^/  /' >&2
        echo "" >&2
    fi
    exit 127
fi
if [ -n "$missing_libs" ]; then
    has_webkit=0
    has_gtk=0
    echo "$missing_libs" | grep -q 'libwebkit2gtk' && has_webkit=1
    echo "$missing_libs" | grep -q 'libgtk-3' && has_gtk=1
    if [ "$has_webkit" = "1" ] || [ "$has_gtk" = "1" ]; then
        echo "" >&2
        echo "koma: missing system libraries required for the GUI." >&2
        echo "" >&2
        echo "Install them with your package manager:" >&2
        echo "" >&2
        if command -v apt-get >/dev/null 2>&1; then
            echo "  sudo apt-get install -y libwebkit2gtk-4.1-0 libgtk-3-0" >&2
        elif command -v dnf >/dev/null 2>&1; then
            echo "  sudo dnf install webkit2gtk4.1 gtk3" >&2
        elif command -v pacman >/dev/null 2>&1; then
            echo "  sudo pacman -S webkit2gtk-4.1 gtk3" >&2
        elif command -v zypper >/dev/null 2>&1; then
            echo "  sudo zypper install libwebkit2gtk-4_1-0 libgtk-3-0" >&2
        else
            echo "  # Debian/Ubuntu:" >&2
            echo "  sudo apt-get install -y libwebkit2gtk-4.1-0 libgtk-3-0" >&2
            echo "  # Fedora:" >&2
            echo "  sudo dnf install webkit2gtk4.1 gtk3" >&2
            echo "  # Arch:" >&2
            echo "  sudo pacman -S webkit2gtk-4.1 gtk3" >&2
        fi
        echo "" >&2
        echo "(Package names may vary by distro/version.)" >&2
        echo "" >&2
    else
        echo "" >&2
        echo "koma: missing shared libraries:" >&2
        echo "$ldd_out" | grep "not found" | sed 's/^/  /' >&2
        echo "" >&2
        echo "Install the missing libraries via your package manager." >&2
        echo "" >&2
    fi
    exit 127
fi
exec "$BIN" "$@"
KOMA_LAUNCHER
            sudo mv "$_tmp_launcher" "$INSTALL_DIR/koma"
            sudo chmod +x "$INSTALL_DIR/koma"
        fi
    fi
else
    # macOS / Windows: single binary, no launcher needed.
    if [ -w "$INSTALL_DIR" ]; then
        mv "$tmp" "$INSTALL_DIR/$bin_name"
    else
        if [ "$(id -u)" = "0" ]; then
            mv "$tmp" "$INSTALL_DIR/$bin_name"
        else
            echo "  $INSTALL_DIR is not writable; using sudo for install step."
            sudo mv "$tmp" "$INSTALL_DIR/$bin_name"
            sudo chmod +x "$INSTALL_DIR/$bin_name"
        fi
    fi
fi

# ---------------------------------------------------------------------------
# macOS: clear the Gatekeeper quarantine flag on the downloaded binary so it
# runs without the "unidentified developer" prompt. Best-effort — ignore if
# xattr is unavailable or the attribute was never set.
# ---------------------------------------------------------------------------
if [ "$os" = "darwin" ] && command -v xattr > /dev/null 2>&1; then
    if [ -w "$INSTALL_DIR/$bin_name" ]; then
        xattr -d com.apple.quarantine "$INSTALL_DIR/$bin_name" 2>/dev/null || true
    else
        sudo xattr -d com.apple.quarantine "$INSTALL_DIR/$bin_name" 2>/dev/null || true
    fi
fi

# ---------------------------------------------------------------------------
# Linux: non-fatal preflight warning (after install, before success banner).
# Tells the user right away if their system is missing webkit/gtk, instead of
# waiting for them to run `koma` and hit the error.
# ---------------------------------------------------------------------------
if [ "$os" = "linux" ] && command -v ldd >/dev/null 2>&1; then
    _ldd_check="$(ldd "$INSTALL_DIR/koma.bin" 2>&1)" || true
    if echo "$_ldd_check" | grep -qE 'GLIBC_[0-9]'; then
        _glibc_needed="$(echo "$_ldd_check" | grep -oE 'GLIBC_[0-9]+\.[0-9]+' | sort -uV | tr '\n' ' ')"
        echo ""
        echo "WARNING: this binary requires glibc $_glibc_needed" >&2
        echo "  If koma fails to start, build from source: ./build.sh" >&2
    elif echo "$_ldd_check" | grep -q "not found"; then
        echo ""
        echo "WARNING: some shared libraries are missing — koma may not start." >&2
        echo "  Debian/Ubuntu: sudo apt-get install -y libwebkit2gtk-4.1-0 libgtk-3-0" >&2
        echo "  Fedora:        sudo dnf install webkit2gtk4.1 gtk3" >&2
        echo "  Arch:          sudo pacman -S webkit2gtk-4.1 gtk3" >&2
    fi
fi

# ---------------------------------------------------------------------------
# Optional: provision Python research environment
# ---------------------------------------------------------------------------
if [ "$WITH_RESEARCH" = "1" ]; then
    if [ "$os" = "windows" ]; then
        echo ""
        echo "WARNING: research/full internet mode is not supported on Windows yet — installing base koma only." >&2
    else
        echo ""
        echo "Provisioning full internet mode environment (downloads ~80MB Firefox)..."
        "$INSTALL_DIR/$bin_name" --internet-fullmode-install
    fi
fi

# ---------------------------------------------------------------------------
# Success
# ---------------------------------------------------------------------------
echo ""
echo "koma installed to $INSTALL_DIR/$bin_name"
echo ""
echo "  Run 'koma' to start."
if [ "$os" != "windows" ]; then
    echo "  Re-run this installer with --with-research (or run"
    echo "  'koma --internet-fullmode-install') to enable full internet mode."
fi
echo ""

# Ensure INSTALL_DIR is on PATH. If missing, append the export to the user's
# shell rc file and tell them how to refresh THIS shell. (An installer runs in a
# subshell and cannot reload the parent interactive shell itself, so the final
# `source` is a step the user runs.)
case ":${PATH}:" in
    *":${INSTALL_DIR}:"*) ;;
    *)
        # Pick the rc file for the user's login shell. Git Bash on Windows
        # always uses ~/.bashrc regardless of $SHELL quirks.
        rc=""
        if [ "$os" = "windows" ]; then
            rc="$HOME/.bashrc"
        else
            case "$(basename "${SHELL:-}")" in
                zsh)  rc="$HOME/.zshrc"  ;;
                bash) rc="$HOME/.bashrc" ;;
                *)
                    if [ -f "$HOME/.zshrc" ]; then rc="$HOME/.zshrc"; else rc="$HOME/.bashrc"; fi
                    ;;
            esac
        fi
        export_line="export PATH=\"$INSTALL_DIR:\$PATH\""
        if [ -n "$rc" ] && ! grep -qsF "$INSTALL_DIR" "$rc" 2>/dev/null; then
            printf '\n# Added by koma installer\n%s\n' "$export_line" >> "$rc"
            echo "  Added $INSTALL_DIR to your PATH in $rc"
        fi
        echo ""
        echo "  To use 'koma' in your current shell right now, run:"
        echo "    source $rc"
        echo "  (new terminals will pick it up automatically)"
        echo ""
        ;;
esac

echo "  https://koma.run"
