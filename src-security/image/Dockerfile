# syntax=docker/dockerfile:1
# koma-safety guest image (ghcr.io/aula-id/koma-safety).
#
# Build:
#   docker build --build-arg KOMA_COMPOSITION=minimal|standard|extended -t koma-safety:local .
#
# Tags: :version (standard), :version-minimal, :version-extended — not :web/:red/:full.

FROM python:3.11-slim-bookworm

ARG KOMA_COMPOSITION=standard

# Base always-on: transport + RE python deps used by smoke handlers.
RUN apt-get update && apt-get install -y --no-install-recommends \
        curl ca-certificates jq \
    && rm -rf /var/lib/apt/lists/*

RUN pip install --no-cache-dir \
        requests==2.32.3 \
        pwntools==4.13.1 \
        lief==0.16.6 \
        unicorn==2.1.1

# Composition packages (fail soft on missing package names across distro builds).
RUN set -eux; \
    apt-get update; \
    if [ "$KOMA_COMPOSITION" = "standard" ] || [ "$KOMA_COMPOSITION" = "extended" ]; then \
      apt-get install -y --no-install-recommends \
        nmap sqlmap file gdb binutils \
        || true; \
      apt-get install -y --no-install-recommends radare2 binwalk \
        || true; \
    fi; \
    if [ "$KOMA_COMPOSITION" = "extended" ]; then \
      apt-get install -y --no-install-recommends \
        hashcat john hydra yara libimage-exiftool-perl \
        || true; \
    fi; \
    rm -rf /var/lib/apt/lists/*

RUN useradd --create-home --uid 10001 koma && \
    mkdir -p /work /artifacts /opt/koma/bin && \
    chown koma:koma /work /artifacts /opt/koma/bin

COPY entrypoint.sh /usr/local/bin/entrypoint.sh
COPY koma_proxy.py /opt/koma/koma_proxy.py
COPY koma_sec_daemon /opt/koma/koma_sec_daemon
COPY catalog /opt/koma/catalog
ENV PYTHONPATH=/opt/koma \
    PATH="/opt/koma/bin:${PATH}" \
    KOMA_STATE=/tmp/koma-state \
    KOMA_SEC_COMPOSITION=${KOMA_COMPOSITION}
RUN chmod 0755 /usr/local/bin/entrypoint.sh

USER koma
WORKDIR /work

ENV HTTP_PROXY=http://proxy:3128 \
    HTTPS_PROXY=http://proxy:3128 \
    NO_PROXY= \
    PYTHONUNBUFFERED=1

ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
CMD ["sleep", "infinity"]
