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
for f in apt_packages_high.txt apt_packages_medium.txt detect_bins.txt; do
  if [[ -f "$SRC/install_recipes/$f" ]]; then
    cp "$SRC/install_recipes/$f" "$IMG/install_recipes/$f"
  else
    : > "$IMG/install_recipes/$f"
  fi
done

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
test -f "$IMG/install_recipes/detect_bins.txt"

echo "synced → $IMG"
