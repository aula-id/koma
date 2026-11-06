# syntax=docker/dockerfile:1
# Koma cyber guest. Everything a security run gets, and nothing it does not.
#
# The boundary is the reason this image exists. It is not a convenience wrapper:
# the guest has an isolated network with a single route out (the proxy), a
# read-only root, a non-root user, and no Docker socket. Losing any of those
# makes a call "run on the host", which is what the gate checks for.
#
# Build-time pinning: the tag carries the content digest at publish time (see
# `image-digest.txt` produced by the release step), and the runtime records the
# resolved digest on every run so a drifted image is visible rather than silent.

FROM python:3.11-slim-bookworm

# Pinned: `lief`/`unicorn` are load-bearing for the RE tools and their wheels are
# version-sensitive, so they are not floated.
# curl is the transport every HTTP tool drives; the image is useless without it.
RUN apt-get update && apt-get install -y --no-install-recommends \
        curl ca-certificates jq \
    && rm -rf /var/lib/apt/lists/*

RUN pip install --no-cache-dir \
        requests==2.32.3 \
        pwntools==4.13.1 \
        lief==0.16.6 \
        unicorn==2.1.1

# Unprivileged at runtime, fixed uid so a mounted artifact dir has one owner.
RUN useradd --create-home --uid 10001 koma && \
    mkdir -p /work /artifacts && \
    chown koma:koma /work /artifacts

COPY entrypoint.sh /usr/local/bin/entrypoint.sh
COPY koma_proxy.py /opt/koma/koma_proxy.py
RUN chmod 0755 /usr/local/bin/entrypoint.sh

USER koma
WORKDIR /work

# The guest reaches nothing directly: the isolated network has no gateway. The
# proxy is reachable as a peer, and every tool is pointed at it explicitly —
# proxy env vars alone are not the boundary, they are only how the client learns
# where to go.
ENV HTTP_PROXY=http://proxy:3128 \
    HTTPS_PROXY=http://proxy:3128 \
    NO_PROXY= \
    PYTHONUNBUFFERED=1

ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
CMD ["sleep", "infinity"]
