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

echo "== image size =="
docker images --format 'table {{.Repository}}:{{.Tag}}\t{{.Size}}\t{{.ID}}' "$TAG"

echo "== labels =="
docker image inspect "$TAG" --format '{{json .Config.Labels}}' | head -c 600; echo

echo "== catalog summary =="
docker run --rm --entrypoint python3 "$TAG" -c \
  'from koma_sec_daemon import catalog; import json; print(json.dumps(catalog.handle("summary"), indent=2)[:1200])'

echo "== detect_report =="
docker run --rm --entrypoint sh "$TAG" -c \
  'if [ -f /opt/koma/catalog/detect_report.json ]; then head -c 2500 /opt/koma/catalog/detect_report.json; echo; else echo missing_detect_report; fi'

echo "== composition env =="
docker run --rm --entrypoint sh "$TAG" -c \
  'echo composition=$KOMA_SEC_COMPOSITION version=$KOMA_VERSION; id; which curl jq nmap 2>/dev/null || true'

echo "OK $TAG"
