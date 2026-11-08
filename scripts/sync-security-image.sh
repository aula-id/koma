#!/usr/bin/env bash
# Sync host security sources into the Docker build context (src-security/image).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/src-security"
IMG="$SRC/image"

rsync -a --delete \
  --exclude '__pycache__' \
  --exclude '*.py[cod]' \
  --exclude '.DS_Store' \
  "$SRC/koma_sec_daemon/" "$IMG/koma_sec_daemon/"

rsync -a --delete \
  --exclude '.DS_Store' \
  "$SRC/catalog/" "$IMG/catalog/"

mkdir -p "$IMG/install_recipes"

# Legacy flat lists (kept for pack tooling / backward compat)
for f in apt_packages_high.txt apt_packages_medium.txt detect_bins.txt; do
  if [[ -f "$SRC/install_recipes/$f" ]]; then
    cp "$SRC/install_recipes/$f" "$IMG/install_recipes/$f"
  else
    : > "$IMG/install_recipes/$f"
  fi
done

# Domain composition buckets + compositions.json (Phase 1 bake)
if [[ -d "$SRC/install_recipes/buckets" ]]; then
  mkdir -p "$IMG/install_recipes/buckets"
  rsync -a --delete \
    --exclude '.DS_Store' \
    "$SRC/install_recipes/buckets/" "$IMG/install_recipes/buckets/"
else
  mkdir -p "$IMG/install_recipes/buckets"
fi

# Ensure minimal empty bucket files exist for daemon-only bake
for ext in apt go npm cargo src bins; do
  f="$IMG/install_recipes/buckets/minimal.${ext}.txt"
  if [[ ! -f "$f" ]]; then
    : > "$f"
  fi
done

if [[ -f "$SRC/install_recipes/buckets/compositions.json" ]]; then
  cp "$SRC/install_recipes/buckets/compositions.json" \
    "$IMG/install_recipes/compositions.json"
  cp "$SRC/install_recipes/buckets/compositions.json" \
    "$IMG/install_recipes/buckets/compositions.json"
elif [[ -f "$SRC/install_recipes/compositions.json" ]]; then
  cp "$SRC/install_recipes/compositions.json" \
    "$IMG/install_recipes/compositions.json"
fi

if [[ -f "$SRC/proxy/koma_proxy.py" ]]; then
  cp "$SRC/proxy/koma_proxy.py" "$IMG/koma_proxy.py"
elif [[ -f "$SRC/koma_proxy.py" ]]; then
  cp "$SRC/koma_proxy.py" "$IMG/koma_proxy.py"
fi

test -f "$IMG/catalog/packs/smoke.json"
test -f "$IMG/catalog/schema.json"
test -f "$IMG/koma_sec_daemon/__main__.py"
test -f "$IMG/koma_proxy.py"
test -f "$IMG/entrypoint.sh"
test -f "$IMG/Dockerfile"
test -d "$IMG/install_recipes/buckets"
test -f "$IMG/install_recipes/buckets/minimal.bins.txt"

echo "synced → $IMG"
