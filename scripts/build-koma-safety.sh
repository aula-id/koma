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

# Base image policy:
#   minimal  → Debian slim (fast CI)
#   standard/extended → native Kali (package names match; no Debian+Kali mix)
case "${COMP}" in
  minimal)
    BASE_IMAGE="${KOMA_BASE_IMAGE:-python:3.11-slim-bookworm}"
    ;;
  standard|extended)
    BASE_IMAGE="${KOMA_BASE_IMAGE:-kalilinux/kali-rolling}"
    ;;
  *)
    echo "unknown composition: ${COMP}" >&2
    exit 1
    ;;
esac

# allow override: standard/extended may use kali-last-release for stability
#   KOMA_BASE_IMAGE=kalilinux/kali-last-release bash scripts/build-koma-safety.sh standard

bash "$ROOT/scripts/sync-security-image.sh"

echo "building composition=${COMP} base=${BASE_IMAGE} tag=${TAG}"
docker build \
  --build-arg "KOMA_COMPOSITION=${COMP}" \
  --build-arg "KOMA_VERSION=${VERSION}" \
  --build-arg "KOMA_BASE_IMAGE=${BASE_IMAGE}" \
  -t "$TAG" \
  "$IMG"

echo "== image size =="
docker images --format 'table {{.Repository}}:{{.Tag}}\t{{.Size}}\t{{.ID}}' "$TAG"

echo "== labels =="
docker image inspect "$TAG" --format '{{json .Config.Labels}}' | head -c 800; echo

echo "== catalog summary =="
docker run --rm --entrypoint python3 "$TAG" -c \
  'from koma_sec_daemon import catalog; import json; print(json.dumps(catalog.handle("summary"), indent=2)[:1200])'

echo "== detect_report =="
docker run --rm --entrypoint sh "$TAG" -c \
  'if [ -f /opt/koma/catalog/detect_report.json ]; then python3 -c "import json; r=json.load(open(\"/opt/koma/catalog/detect_report.json\")); print(\"present\", r[\"present_count\"], \"/\", r[\"bins_total\"]); print(\"sample\", r[\"present\"][:25])"; else echo missing_detect_report; fi'

echo "== install_report counts =="
docker run --rm --entrypoint sh "$TAG" -c \
  'if [ -f /opt/koma/install_report.txt ]; then
     echo ok:$(grep -c "^ok " /opt/koma/install_report.txt || true)
     echo ok_batch:$(grep -c "^ok_batch " /opt/koma/install_report.txt || true)
     echo miss:$(grep -c "^miss " /opt/koma/install_report.txt || true)
     head -5 /opt/koma/install_report.txt
   else echo missing_install_report; fi'

echo "== composition env =="
docker run --rm --entrypoint sh "$TAG" -c \
  'echo composition=$KOMA_SEC_COMPOSITION version=$KOMA_VERSION; id; command -v curl jq nmap python3 2>/dev/null || true'

echo "OK $TAG"
