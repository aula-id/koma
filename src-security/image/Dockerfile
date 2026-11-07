# syntax=docker/dockerfile:1
# koma-safety guest image — CD build context: src-security/image
#
# Registry: ghcr.io/aula-id/koma-safety
# Compositions (build-arg KOMA_COMPOSITION):
#   minimal  — daemon + proxy path + curl/jq (smoke / CI)
#   standard — default power-user CLIs (release :version + :latest)
#   extended — larger apt set (release :version-extended)
#
# Local:
#   ./scripts/sync-security-image.sh
#   docker build --build-arg KOMA_COMPOSITION=minimal -t koma-safety:local-minimal .
#
# CD (release.yml): multi-arch buildx, composition matrix, OCI labels.

FROM python:3.11-slim-bookworm

ARG KOMA_COMPOSITION=standard
ARG KOMA_VERSION=0.0.0

LABEL org.opencontainers.image.title="koma-safety" \
      org.opencontainers.image.description="Koma security guest runtime (isolated tools)" \
      org.opencontainers.image.source="https://github.com/aula-id/koma" \
      org.opencontainers.image.vendor="aula-id" \
      org.opencontainers.image.version="${KOMA_VERSION}" \
      com.koma.composition="${KOMA_COMPOSITION}"

# ---- base always-on (all compositions) ----
USER root
RUN apt-get update && apt-get install -y --no-install-recommends \
        curl ca-certificates jq \
    && rm -rf /var/lib/apt/lists/*

# Pinned wheels used by smoke RE/crypto handlers.
RUN pip install --no-cache-dir \
        requests==2.32.3 \
        pwntools==4.13.1 \
        lief==0.16.6 \
        unicorn==2.1.1

# ---- composition packages ----
# minimal: nothing extra (curl/jq already above).
# standard: fail HARD on core CLIs so CD cannot ship an empty "standard".
# extended: adds heavier packages; soft-fail only for arch-missing optional names.
RUN set -eux; \
    case "${KOMA_COMPOSITION}" in \
      minimal) \
        echo "composition=minimal (base only)" ;; \
      standard) \
        apt-get update; \
        apt-get install -y --no-install-recommends \
          nmap sqlmap file gdb binutils; \
        apt-get install -y --no-install-recommends radare2 \
          || apt-get install -y --no-install-recommends radare2-cutter \
          || true; \
        rm -rf /var/lib/apt/lists/* ;; \
      extended) \
        apt-get update; \
        apt-get install -y --no-install-recommends \
          nmap sqlmap file gdb binutils; \
        apt-get install -y --no-install-recommends radare2 \
          || true; \
        apt-get install -y --no-install-recommends \
          hashcat john hydra yara libimage-exiftool-perl binwalk \
          || true; \
        rm -rf /var/lib/apt/lists/* ;; \
      *) \
        echo "unknown KOMA_COMPOSITION=${KOMA_COMPOSITION}" >&2; exit 1 ;; \
    esac

RUN useradd --create-home --uid 10001 koma && \
    mkdir -p /work /artifacts /opt/koma/bin /opt/koma/catalog && \
    chown -R koma:koma /work /artifacts /opt/koma

# Runtime payload (synced from src-security via scripts/sync-security-image.sh before CD).
COPY entrypoint.sh /usr/local/bin/entrypoint.sh
COPY koma_proxy.py /opt/koma/koma_proxy.py
COPY koma_sec_daemon /opt/koma/koma_sec_daemon
COPY catalog /opt/koma/catalog

ENV PYTHONPATH=/opt/koma \
    PATH="/opt/koma/bin:${PATH}" \
    KOMA_STATE=/tmp/koma-state \
    KOMA_SEC_COMPOSITION=${KOMA_COMPOSITION} \
    KOMA_VERSION=${KOMA_VERSION} \
    TERM=xterm

RUN chmod 0755 /usr/local/bin/entrypoint.sh \
    && python3 -c "import koma_sec_daemon, koma_sec_daemon.catalog; print('daemon_ok', koma_sec_daemon.catalog.handle('summary'))"

USER koma
WORKDIR /work

# Proxy env is how clients LEARN the route; the boundary is --internal (no gateway).
ENV HTTP_PROXY=http://proxy:3128 \
    HTTPS_PROXY=http://proxy:3128 \
    http_proxy=http://proxy:3128 \
    https_proxy=http://proxy:3128 \
    NO_PROXY= \
    no_proxy= \
    PYTHONUNBUFFERED=1

ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
CMD ["sleep", "infinity"]
