#!/usr/bin/env bash
# Build + smoke-test koma-safety compositions (local Colima/Docker or CI).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
IMG="$ROOT/src-security/image"
COMP="${1:-minimal}"
TAG="${2:-koma-safety:local-${COMP}}"
if [[ -z "${KOMA_VERSION:-}" && -f "$ROOT/version.json" ]]; then
  KOMA_VERSION="$(python3 -c "import json; print(json.load(open('$ROOT/version.json'))['version'])" 2>/dev/null || true)"
fi
VERSION="${KOMA_VERSION:-0.0.0-dev}"

bash "$ROOT/scripts/sync-security-image.sh"

docker build \
  --build-arg "KOMA_COMPOSITION=${COMP}" \
  --build-arg "KOMA_VERSION=${VERSION}" \
  -t "$TAG" \
  "$IMG"

echo "== image inspect labels =="
docker image inspect "$TAG" --format '{{json .Config.Labels}}' | head -c 500; echo

echo "== smoke: import + catalog summary =="
docker run --rm --entrypoint python3 "$TAG" -c \
  'from koma_sec_daemon import catalog; import json; print(json.dumps(catalog.handle("summary"), indent=2)[:800])'

echo "== smoke: composition env =="
docker run --rm --entrypoint sh "$TAG" -c 'echo composition=$KOMA_SEC_COMPOSITION version=$KOMA_VERSION; which curl jq; id'

echo "== smoke: detect common bins (best-effort) =="
docker run --rm --entrypoint sh "$TAG" -c \
  'for b in curl jq python3 nmap sqlmap gdb file; do
     if command -v "$b" >/dev/null 2>&1; then echo "ok $b"; else echo "missing $b"; fi
   done'

echo "OK $TAG"
