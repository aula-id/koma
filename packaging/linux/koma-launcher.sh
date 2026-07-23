#!/bin/sh
# koma launcher — Linux safety-net wrapper for prebuilt binaries.
#
# Resolves the real ELF (koma.bin) next to this script, preflights shared-
# library availability via ldd, and either execs the binary or prints a
# friendly, actionable error message.
#
# CONSTRAINT: We never run sudo, apt, dnf, pacman, or any package manager.
# We only PRINT commands for the user to run themselves.
#
# This is the canonical copy. install.sh embeds a duplicate via here-doc;
# keep them in sync when editing.

set -e

# ---------------------------------------------------------------------------
# Resolve directory of this script (handles symlinks best-effort).
# ---------------------------------------------------------------------------
resolve_dir() {
    # $0 may be a relative or absolute path; strip trailing slash.
    _src="$0"
    while [ -L "$_src" ]; do
        _dir="$(cd -P "$(dirname -- "$_src")" && pwd)"
        _src="$(readlink -- "$_src")"
        # If readlink gave a relative path, resolve against _dir.
        case "$_src" in
            /*) ;;
            *)  _src="$_dir/$_src" ;;
        esac
    done
    cd -P "$(dirname -- "$_src")" && pwd
}

DIR="$(resolve_dir)"
BIN="$DIR/koma.bin"

# ---------------------------------------------------------------------------
# Sanity: real binary must exist and be executable.
# ---------------------------------------------------------------------------
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

# ---------------------------------------------------------------------------
# Preflight: ldd to check shared libraries before the dynamic linker does.
# Prefer ldd over ldconfig -p because ldd catches BOTH missing .so files
# AND symbol version mismatches (e.g. GLIBC_2.38 not found).
# ---------------------------------------------------------------------------
ldd_out=""
ldd_ok=0
if command -v ldd >/dev/null 2>&1; then
    ldd_out="$(ldd "$BIN" 2>&1)" && ldd_ok=1 || ldd_ok=0
fi

if [ "$ldd_ok" = "1" ] && echo "$ldd_out" | grep -q "not a dynamic executable"; then
    # ldd says it's not an ELF — unusual, but let the exec try anyway.
    exec "$BIN" "$@"
fi

if [ "$ldd_ok" = "1" ] && ! echo "$ldd_out" | grep -q "not found"; then
    # Clean ldd — all libs resolved. Also check for GLIBC version strings
    # in the output (ldd doesn't print "not found" for version mismatches
    # on all systems; some print a warning but still show the lib).
    # Double-check: if ldd exited 0 and no "not found", we're good.
    exec "$BIN" "$@"
fi

# ---------------------------------------------------------------------------
# ldd found issues. Classify and print actionable message.
# ---------------------------------------------------------------------------
glibc_needed=""
missing_libs=""

if [ "$ldd_out" != "" ]; then
    # Extract GLIBC version requirements from "GLIBC_X.Y not found" lines.
    glibc_needed=$(echo "$ldd_out" | grep -oE 'GLIBC_[0-9]+\.[0-9]+' | sort -uV | tr '\n' ' ')

    # Extract missing .so names (not glibc — those are handled separately).
    missing_libs=$(echo "$ldd_out" | grep "not found" | grep -v 'GLIBC_' | awk '{print $1}' | sort -u)
fi

# --- Case 1: GLIBC too old ---
if [ -n "$glibc_needed" ]; then
    echo "" >&2
    echo "koma: prebuilt binary requires a newer glibc than this system provides." >&2
    echo "" >&2

    # Show host glibc version if possible.
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

    # Also mention any missing GUI libs.
    if [ -n "$missing_libs" ]; then
        echo "Additional missing libraries:" >&2
        echo "$missing_libs" | sed 's/^/  /' >&2
        echo "" >&2
    fi

    exit 127
fi

# --- Case 2: Missing GUI / other shared libraries ---
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
        # Other missing libs — just show what ldd found.
        echo "" >&2
        echo "koma: missing shared libraries:" >&2
        echo "$ldd_out" | grep "not found" | sed 's/^/  /' >&2
        echo "" >&2
        echo "Install the missing libraries via your package manager." >&2
        echo "" >&2
    fi

    exit 127
fi

# --- Fallback: ldd unavailable or unclassifiable ---
# If we got here without a clear classification, try to exec anyway.
# This handles edge cases where ldd is missing or behaved unexpectedly.
exec "$BIN" "$@"
