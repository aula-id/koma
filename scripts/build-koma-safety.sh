#!/usr/bin/env bash
# Build + smoke-test koma-safety domain compositions (local Colima/Docker or CI).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
IMG="$ROOT/src-security/image"
COMP="${1:-minimal}"
TAG="${2:-koma-safety:local-${COMP}}"
MAX_BYTES=$((9 * 1000 * 1000 * 1000)) # 9GB hard ceiling (decimal GB)

if [[ -z "${KOMA_VERSION:-}" && -f "$ROOT/version.json" ]]; then
  KOMA_VERSION="$(python3 -c "import json; print(json.load(open('$ROOT/version.json'))['version'])" 2>/dev/null || true)"
fi
VERSION="${KOMA_VERSION:-0.0.0-dev}"

bash "$ROOT/scripts/sync-security-image.sh"

COMP_JSON="$ROOT/src-security/install_recipes/buckets/compositions.json"
if [[ ! -f "$COMP_JSON" ]]; then
  COMP_JSON="$IMG/install_recipes/compositions.json"
fi

resolve_base_image() {
  local comp="$1"
  if [[ -n "${KOMA_BASE_IMAGE:-}" ]]; then
    printf '%s\n' "$KOMA_BASE_IMAGE"
    return 0
  fi
  if [[ -f "$COMP_JSON" ]]; then
    local hint
    hint="$(python3 - "$COMP_JSON" "$comp" <<'PY'
import json, sys
path, cid = sys.argv[1], sys.argv[2]
try:
    data = json.load(open(path))
except Exception:
    print("")
    raise SystemExit(0)
rows = data if isinstance(data, list) else data.get("compositions") or data.get("items") or []
for row in rows:
    if isinstance(row, dict) and row.get("id") == cid:
        print(str(row.get("base_hint") or "debian").strip().lower())
        break
else:
    print("")
PY
)"
    case "$hint" in
      kali)
        printf '%s\n' "kalilinux/kali-rolling"
        return 0
        ;;
      debian|bookworm|"")
        printf '%s\n' "python:3.11-slim-bookworm"
        return 0
        ;;
      *)
        # unknown hint → debian default
        printf '%s\n' "python:3.11-slim-bookworm"
        return 0
        ;;
    esac
  fi
  # Fallback without compositions.json: kali only for recon/identity family
  case "$comp" in
    recon|identity)
      printf '%s\n' "kalilinux/kali-rolling"
      ;;
    *)
      printf '%s\n' "python:3.11-slim-bookworm"
      ;;
  esac
}

BASE_IMAGE="$(resolve_base_image "$COMP")"

# Validate composition id when catalog is present
if [[ -f "$COMP_JSON" ]]; then
  if ! python3 - "$COMP_JSON" "$COMP" <<'PY'
import json, sys
path, cid = sys.argv[1], sys.argv[2]
data = json.load(open(path))
rows = data if isinstance(data, list) else data.get("compositions") or data.get("items") or []
ids = {r.get("id") for r in rows if isinstance(r, dict)}
sys.exit(0 if cid in ids else 1)
PY
  then
    echo "unknown composition: ${COMP} (not in compositions.json)" >&2
    exit 1
  fi
fi

# Bucket files should exist for known compositions (minimal may be empty)
BUCKET_DIR="$ROOT/src-security/install_recipes/buckets"
if [[ -d "$BUCKET_DIR" && ! -f "$BUCKET_DIR/${COMP}.bins.txt" && "$COMP" != "minimal" ]]; then
  echo "warn: missing bucket files for composition=${COMP} under $BUCKET_DIR" >&2
fi

echo "building composition=${COMP} base=${BASE_IMAGE} tag=${TAG}"
docker build \
  --build-arg "KOMA_COMPOSITION=${COMP}" \
  --build-arg "KOMA_VERSION=${VERSION}" \
  --build-arg "KOMA_BASE_IMAGE=${BASE_IMAGE}" \
  -t "$TAG" \
  "$IMG"

echo "== image size =="
docker images --format 'table {{.Repository}}:{{.Tag}}\t{{.Size}}\t{{.ID}}' "$TAG"
SIZE_BYTES="$(docker image inspect "$TAG" --format '{{.Size}}')"
echo "size_bytes=${SIZE_BYTES} max_bytes=${MAX_BYTES}"
if [[ -n "${SIZE_BYTES}" ]] && [[ "${SIZE_BYTES}" =~ ^[0-9]+$ ]]; then
  if (( SIZE_BYTES > MAX_BYTES )); then
    echo "FAIL: image ${TAG} is ${SIZE_BYTES} bytes (> 9GB ceiling)" >&2
    exit 1
  fi
  # human-ish
  python3 -c "b=int('${SIZE_BYTES}'); print(f'size_human={b/1e9:.2f} GB')" 2>/dev/null || true
else
  echo "warn: could not parse image size from docker inspect" >&2
fi

echo "== labels =="
docker image inspect "$TAG" --format '{{json .Config.Labels}}' | head -c 800; echo

echo "== catalog summary =="
docker run --rm --entrypoint python3 "$TAG" -c \
  'from koma_sec_daemon import catalog; import json; print(json.dumps(catalog.handle("summary"), indent=2)[:1200])'

echo "== detect_report =="
docker run --rm --entrypoint sh "$TAG" -c \
  'if [ -f /opt/koma/catalog/detect_report.json ]; then python3 -c "import json; r=json.load(open(\"/opt/koma/catalog/detect_report.json\")); print(\"present\", r[\"present_count\"], \"/\", r[\"bins_total\"]); print(\"sample\", r[\"present\"][:25]); print(\"missing_sample\", r.get(\"missing\", [])[:15])"; else echo missing_detect_report; fi'

echo "== install_report counts =="
docker run --rm --entrypoint sh "$TAG" -c \
  'if [ -f /opt/koma/install_report.txt ]; then
     echo ok:$(grep -c "^ok " /opt/koma/install_report.txt || true)
     echo ok_batch:$(grep -c "^ok_batch " /opt/koma/install_report.txt || true)
     echo miss:$(grep -c "^miss " /opt/koma/install_report.txt || true)
     echo ok_go:$(grep -c "^ok_go " /opt/koma/install_report.txt || true)
     echo miss_go:$(grep -c "^miss_go " /opt/koma/install_report.txt || true)
     head -8 /opt/koma/install_report.txt
   else echo missing_install_report; fi'

echo "== composition env =="
docker run --rm --entrypoint sh "$TAG" -c \
  'echo composition=$KOMA_SEC_COMPOSITION version=$KOMA_VERSION; id; command -v curl jq python3 2>/dev/null || true'

echo "OK $TAG size_bytes=${SIZE_BYTES:-unknown}"
