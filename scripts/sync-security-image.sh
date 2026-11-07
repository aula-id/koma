#!/usr/bin/env bash
# Sync host security sources into the Docker build context (src-security/image).
# CD and local builds must run this (or rely on a fresh tree) before docker build.
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

# Proxy lives at image root for COPY path stability.
if [[ -f "$SRC/proxy/koma_proxy.py" ]]; then
  cp "$SRC/proxy/koma_proxy.py" "$IMG/koma_proxy.py"
elif [[ -f "$SRC/koma_proxy.py" ]]; then
  cp "$SRC/koma_proxy.py" "$IMG/koma_proxy.py"
fi

# Smoke: catalog packs present
test -f "$IMG/catalog/packs/smoke.json"
test -f "$IMG/catalog/schema.json"
test -f "$IMG/koma_sec_daemon/__main__.py"
test -f "$IMG/koma_proxy.py"
test -f "$IMG/entrypoint.sh"
test -f "$IMG/Dockerfile"

echo "synced → $IMG"
