#!/usr/bin/env bash
# Refresh (or check) the Nix fixed-output hash for src-webgui's npm deps.
#
# Why this exists: flake.nix uses pkgs.fetchNpmDeps so the GUI build can
# `npm install` offline in the sandbox. That hash is NOT app version — it
# fingerprints the lockfile-resolved tarball cache. Every package-lock.json
# change needs a new hash or CI's `nix flake check` dies with a mismatch.
#
# Usage:
#   ./scripts/update-webgui-npm-deps.sh          # write src-webgui/npm-deps-hash
#   ./scripts/update-webgui-npm-deps.sh --check  # exit 1 if hash is stale
#
# Requires: nix with flakes (nix-command). Prefers the flake's nixpkgs pin so
# the hash matches CI's fetchNpmDeps implementation.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
LOCK="$ROOT/src-webgui/package-lock.json"
HASH_FILE="$ROOT/src-webgui/npm-deps-hash"
MODE="write"

for arg in "$@"; do
  case "$arg" in
    --check) MODE="check" ;;
    -h|--help)
      sed -n '2,16p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "update-webgui-npm-deps.sh: unknown arg: $arg" >&2
      exit 2
      ;;
  esac
done

if [[ ! -f "$LOCK" ]]; then
  echo "update-webgui-npm-deps.sh: missing $LOCK" >&2
  exit 1
fi

if ! command -v nix >/dev/null 2>&1; then
  echo "update-webgui-npm-deps.sh: nix not found on PATH" >&2
  exit 1
fi

# Use the flake input's nixpkgs so prefetch-npm-deps matches fetchNpmDeps in
# flake.nix (same tool revision → same hash). Fall back to nixpkgs-unstable
# only when the flake lock is unusable (e.g. first-time bootstrap).
prefetch() {
  if nix eval --raw .#packages.x86_64-linux.default.name >/dev/null 2>&1 \
    || nix flake metadata >/dev/null 2>&1; then
    # Resolve prefetch-npm-deps from the same nixpkgs the flake pins.
    nix shell --inputs-from "$ROOT" nixpkgs#prefetch-npm-deps \
      -c prefetch-npm-deps "$LOCK"
  else
    nix shell nixpkgs#prefetch-npm-deps -c prefetch-npm-deps "$LOCK"
  fi
}

echo "update-webgui-npm-deps.sh: prefetching from package-lock.json …" >&2
NEW_HASH="$(prefetch | tail -n1 | tr -d '[:space:]')"

if [[ ! "$NEW_HASH" =~ ^sha256- ]]; then
  echo "update-webgui-npm-deps.sh: unexpected prefetch output: $NEW_HASH" >&2
  exit 1
fi

OLD_HASH=""
if [[ -f "$HASH_FILE" ]]; then
  OLD_HASH="$(tr -d '[:space:]' <"$HASH_FILE")"
fi

if [[ "$MODE" == "check" ]]; then
  if [[ "$OLD_HASH" == "$NEW_HASH" ]]; then
    echo "update-webgui-npm-deps.sh: ok ($NEW_HASH)"
    exit 0
  fi
  echo "update-webgui-npm-deps.sh: STALE npm-deps hash" >&2
  echo "  file:     $HASH_FILE" >&2
  echo "  expected: $NEW_HASH  (from package-lock.json)" >&2
  echo "  actual:   ${OLD_HASH:-<missing>}" >&2
  echo >&2
  echo "Fix:  ./scripts/update-webgui-npm-deps.sh && git add src-webgui/npm-deps-hash" >&2
  exit 1
fi

printf '%s\n' "$NEW_HASH" >"$HASH_FILE"
if [[ "$OLD_HASH" == "$NEW_HASH" ]]; then
  echo "update-webgui-npm-deps.sh: unchanged ($NEW_HASH)"
else
  echo "update-webgui-npm-deps.sh: wrote $HASH_FILE"
  echo "  ${OLD_HASH:-<missing>} -> $NEW_HASH"
fi
