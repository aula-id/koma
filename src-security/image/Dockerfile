# syntax=docker/dockerfile:1
# koma-safety guest image — CD build context: src-security/image
#
# Compositions (KOMA_COMPOSITION) + base (KOMA_BASE_IMAGE):
#   minimal  — python:3.11-slim-bookworm (no Kali; CI/smoke)
#   standard — kalilinux/kali-rolling (native Kali apt — do NOT mix into Debian)
#   extended — same Kali base + medium apt list
#
# Mixing Kali packages into Debian bookworm breaks libc/curl (see failed hybrid bake).
# Kali docs: never add kali.sources to a non-Kali OS.
#
# Local:
#   bash scripts/build-koma-safety.sh minimal
#   bash scripts/build-koma-safety.sh standard

ARG KOMA_BASE_IMAGE=python:3.11-slim-bookworm
FROM ${KOMA_BASE_IMAGE}

ARG KOMA_COMPOSITION=standard
ARG KOMA_VERSION=0.0.0
ARG KOMA_BASE_IMAGE

LABEL org.opencontainers.image.title="koma-safety" \
      org.opencontainers.image.description="Koma security guest runtime (isolated tools)" \
      org.opencontainers.image.source="https://github.com/aula-id/koma" \
      org.opencontainers.image.vendor="aula-id" \
      org.opencontainers.image.version="${KOMA_VERSION}" \
      com.koma.composition="${KOMA_COMPOSITION}" \
      com.koma.base="${KOMA_BASE_IMAGE}"

USER root

# Base OS tools + Python deps in a venv (Kali is PEP 668 / externally managed).
RUN set -eux; \
    export DEBIAN_FRONTEND=noninteractive; \
    apt-get update; \
    apt-get install -y --no-install-recommends \
      curl ca-certificates jq \
      python3 python3-pip python3-venv python3-dev \
      gcc build-essential cmake pkg-config; \
    mkdir -p /opt/koma/bin; \
    python3 -m venv /opt/koma/venv; \
    /opt/koma/venv/bin/pip install --no-cache-dir -U pip wheel; \
    /opt/koma/venv/bin/pip install --no-cache-dir \
      'requests>=2.32,<3' \
      'pwntools>=4.13,<5' \
      'lief>=0.16'; \
    # unicorn may lack a manylinux wheel on some arches — soft-fail
    /opt/koma/venv/bin/pip install --no-cache-dir 'unicorn>=2.1' \
      || echo "warn: unicorn pip install skipped"; \
    ln -sf /opt/koma/venv/bin/python /opt/koma/bin/python3; \
    ln -sf /opt/koma/venv/bin/python /usr/local/bin/python3-koma || true; \
    rm -rf /var/lib/apt/lists/*

COPY install_recipes/apt_packages_high.txt /tmp/apt_packages_high.txt
COPY install_recipes/apt_packages_medium.txt /tmp/apt_packages_medium.txt
COPY install_recipes/detect_bins.txt /tmp/detect_bins.txt

# Composition packages — all from the *native* base repos only (Debian or Kali).
# Soft-fail per package/batch; never pull a second distro's libc.
RUN set -eux; \
    export DEBIAN_FRONTEND=noninteractive; \
    REPORT=/tmp/install_report.txt; \
    : > "$REPORT"; \
    echo "base=${KOMA_BASE_IMAGE}" >> "$REPORT"; \
    echo "composition=${KOMA_COMPOSITION}" >> "$REPORT"; \
    install_list() { \
      list_file="$1"; \
      tag="$2"; \
      [ -s "$list_file" ] || return 0; \
      pkgs=$(grep -vE '^\s*(#|$)' "$list_file" | tr -d '\r' | tr '\n' ' '); \
      [ -n "$pkgs" ] || return 0; \
      set -- $pkgs; \
      while [ "$#" -gt 0 ]; do \
        batch=""; n=0; \
        while [ "$#" -gt 0 ] && [ "$n" -lt 15 ]; do \
          batch="$batch $1"; shift; n=$((n+1)); \
        done; \
        if apt-get install -y --no-install-recommends $batch; then \
          echo "ok_batch $tag$batch" >> "$REPORT"; \
        else \
          echo "soft_batch $tag$batch" >> "$REPORT"; \
          apt-get -y -f install >/dev/null 2>&1 || true; \
          for p in $batch; do \
            if apt-get install -y --no-install-recommends "$p"; then \
              echo "ok $p" >> "$REPORT"; \
            else \
              echo "miss $p" >> "$REPORT"; \
              apt-get -y -f install >/dev/null 2>&1 || true; \
            fi; \
          done; \
        fi; \
      done; \
    }; \
    case "${KOMA_COMPOSITION}" in \
      minimal) \
        echo "composition=minimal (no bulk tools)" | tee -a "$REPORT" ;; \
      standard|extended) \
        apt-get update; \
        apt-get install -y --no-install-recommends \
          nmap sqlmap file gdb binutils \
          || true; \
        apt-get install -y --no-install-recommends radare2 \
          || apt-get install -y --no-install-recommends radare2-cutter \
          || true; \
        echo "core_ok" >> "$REPORT"; \
        install_list /tmp/apt_packages_high.txt high; \
        if [ "${KOMA_COMPOSITION}" = "extended" ]; then \
          install_list /tmp/apt_packages_medium.txt medium; \
        fi; \
        rm -rf /var/lib/apt/lists/* ;; \
      *) \
        echo "unknown KOMA_COMPOSITION=${KOMA_COMPOSITION}" >&2; exit 1 ;; \
    esac; \
    cp "$REPORT" /tmp/install_report_final.txt

RUN set -eux; \
    if ! id -u koma >/dev/null 2>&1; then \
      useradd --create-home --uid 10001 koma || useradd --create-home koma; \
    fi; \
    mkdir -p /work /artifacts /opt/koma/bin /opt/koma/catalog; \
    chown -R koma:koma /work /artifacts /opt/koma || true; \
    cp /tmp/install_report_final.txt /opt/koma/install_report.txt || true

COPY entrypoint.sh /usr/local/bin/entrypoint.sh
COPY koma_proxy.py /opt/koma/koma_proxy.py
COPY koma_sec_daemon /opt/koma/koma_sec_daemon
COPY catalog /opt/koma/catalog
COPY install_recipes/detect_bins.txt /opt/koma/catalog/detect_bins.txt

ENV PYTHONPATH=/opt/koma \
    PATH="/opt/koma/bin:/opt/koma/venv/bin:${PATH}" \
    VIRTUAL_ENV=/opt/koma/venv \
    KOMA_STATE=/tmp/koma-state \
    KOMA_SEC_COMPOSITION=${KOMA_COMPOSITION} \
    KOMA_VERSION=${KOMA_VERSION} \
    TERM=xterm

RUN chmod 0755 /usr/local/bin/entrypoint.sh \
    && python3 -c "import koma_sec_daemon, koma_sec_daemon.catalog; print('daemon_ok', koma_sec_daemon.catalog.handle('summary'))" \
    && python3 - <<'PY'
import json, shutil, os
from pathlib import Path
bins = []
p = Path("/opt/koma/catalog/detect_bins.txt")
if p.is_file():
    bins = [ln.strip() for ln in p.read_text(encoding="utf-8").splitlines() if ln.strip() and not ln.strip().startswith("#")]
present, missing = [], []
for b in bins:
    if shutil.which(b):
        present.append(b)
    else:
        missing.append(b)
report = {
    "composition": os.environ.get("KOMA_SEC_COMPOSITION", ""),
    "bins_total": len(bins),
    "present_count": len(present),
    "missing_count": len(missing),
    "present": present[:200],
    "missing": missing[:200],
    "present_truncated": len(present) > 200,
    "missing_truncated": len(missing) > 200,
}
Path("/opt/koma/catalog/detect_report.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
print("detect", report["present_count"], "/", report["bins_total"])
# install report counters
ir = Path("/opt/koma/install_report.txt")
if ir.is_file():
    text = ir.read_text(encoding="utf-8", errors="replace")
    for key in ("ok ", "ok_batch ", "miss ", "ok_kali "):
        print("report", key.strip(), text.count("\n" + key) + (1 if text.startswith(key) else 0))
PY

USER koma
WORKDIR /work

ENV HTTP_PROXY=http://proxy:3128 \
    HTTPS_PROXY=http://proxy:3128 \
    http_proxy=http://proxy:3128 \
    https_proxy=http://proxy:3128 \
    NO_PROXY= \
    no_proxy= \
    PYTHONUNBUFFERED=1

ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
CMD ["sleep", "infinity"]
