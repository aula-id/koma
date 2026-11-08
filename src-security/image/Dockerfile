# syntax=docker/dockerfile:1
# koma-safety guest image — CD build context: src-security/image
#
# Domain compositions (KOMA_COMPOSITION) install from install_recipes/buckets/${C}.*
# minimal is not skipped: it bakes the handler binaries no domain image contains.
# Base (KOMA_BASE_IMAGE): debian bookworm by default; kali for recon/identity (see compositions.json).
# Do NOT mix Kali packages into Debian (breaks libc/curl).
#
# Local:
#   bash scripts/build-koma-safety.sh minimal
#   bash scripts/build-koma-safety.sh web

ARG KOMA_BASE_IMAGE=python:3.11-slim-bookworm
FROM ${KOMA_BASE_IMAGE}

ARG KOMA_COMPOSITION=minimal
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
      curl ca-certificates jq git \
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

# Domain bake lists (synced by scripts/sync-security-image.sh)
COPY install_recipes/ /tmp/install_recipes/

# Composition packages from buckets/${KOMA_COMPOSITION}.{apt,go,cargo,npm,src}.txt
# Soft-fail per package/batch; never pull a second distro's libc.
RUN set -eux; \
    export DEBIAN_FRONTEND=noninteractive; \
    export PATH="/usr/local/go/bin:/root/go/bin:/root/.cargo/bin:${PATH}"; \
    C="${KOMA_COMPOSITION}"; \
    B="/tmp/install_recipes/buckets"; \
    REPORT=/tmp/install_report.txt; \
    : > "$REPORT"; \
    echo "base=${KOMA_BASE_IMAGE}" >> "$REPORT"; \
    echo "composition=${C}" >> "$REPORT"; \
    install_list() { \
      list_file="$1"; \
      tag="$2"; \
      [ -f "$list_file" ] || return 0; \
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
    file_nonempty() { \
      f="$1"; \
      [ -f "$f" ] && [ -s "$f" ] && grep -vE '^\s*(#|$)' "$f" | grep -q .; \
    }; \
    if [ "${C}" = "minimal" ]; then \
      echo "composition=minimal (handler bins, not bulk catalog)" | tee -a "$REPORT"; \
    fi; \
    apt-get update; \
      # --- apt ---
      if file_nonempty "$B/${C}.apt.txt"; then \
        install_list "$B/${C}.apt.txt" apt; \
      else \
        echo "no_apt_list ${C}" >> "$REPORT"; \
      fi; \
      # --- go ---
      if file_nonempty "$B/${C}.go.txt"; then \
        if ! command -v go >/dev/null 2>&1; then \
          apt-get install -y --no-install-recommends golang-go \
            || echo "miss golang-go" >> "$REPORT"; \
        fi; \
        if command -v go >/dev/null 2>&1; then \
          export GOPATH=/root/go GOBIN=/opt/koma/bin; \
          mkdir -p "$GOBIN"; \
          while IFS= read -r line || [ -n "$line" ]; do \
            line=$(printf '%s' "$line" | tr -d '\r' | sed 's/#.*//' | xargs || true); \
            [ -n "$line" ] || continue; \
            if go install "$line"; then \
              echo "ok_go $line" >> "$REPORT"; \
            else \
              echo "miss_go $line" >> "$REPORT"; \
            fi; \
          done < "$B/${C}.go.txt"; \
        fi; \
      fi; \
      # --- cargo ---
      if file_nonempty "$B/${C}.cargo.txt"; then \
        if ! command -v cargo >/dev/null 2>&1; then \
          apt-get install -y --no-install-recommends cargo rustc \
            || (curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
                  | sh -s -- -y --default-toolchain stable \
                && . "$HOME/.cargo/env") \
            || echo "miss cargo" >> "$REPORT"; \
        fi; \
        if command -v cargo >/dev/null 2>&1 || [ -x "$HOME/.cargo/bin/cargo" ]; then \
          export PATH="$HOME/.cargo/bin:${PATH}"; \
          while IFS= read -r line || [ -n "$line" ]; do \
            line=$(printf '%s' "$line" | tr -d '\r' | sed 's/#.*//' | xargs || true); \
            [ -n "$line" ] || continue; \
            if cargo install --locked "$line" 2>/dev/null \
               || cargo install "$line"; then \
              echo "ok_cargo $line" >> "$REPORT"; \
            else \
              echo "miss_cargo $line" >> "$REPORT"; \
            fi; \
          done < "$B/${C}.cargo.txt"; \
          # ship cargo bins into /opt/koma/bin when installed to cargo home
          if [ -d "$HOME/.cargo/bin" ]; then \
            cp -n "$HOME/.cargo/bin"/* /opt/koma/bin/ 2>/dev/null || true; \
          fi; \
        fi; \
      fi; \
      # --- npm ---
      if file_nonempty "$B/${C}.npm.txt"; then \
        if ! command -v npm >/dev/null 2>&1; then \
          apt-get install -y --no-install-recommends npm nodejs \
            || echo "miss npm" >> "$REPORT"; \
        fi; \
        if command -v npm >/dev/null 2>&1; then \
          while IFS= read -r line || [ -n "$line" ]; do \
            line=$(printf '%s' "$line" | tr -d '\r' | sed 's/#.*//' | xargs || true); \
            [ -n "$line" ] || continue; \
            if npm install -g "$line"; then \
              echo "ok_npm $line" >> "$REPORT"; \
            else \
              echo "miss_npm $line" >> "$REPORT"; \
            fi; \
          done < "$B/${C}.npm.txt"; \
        fi; \
      fi; \
      # --- from-source allowlist (soft: shallow clone + best-effort install) ---
      if file_nonempty "$B/${C}.src.txt"; then \
        mkdir -p /opt/koma/src-tools; \
        if ! command -v git >/dev/null 2>&1; then \
          apt-get install -y --no-install-recommends git || true; \
        fi; \
        while IFS= read -r line || [ -n "$line" ]; do \
          line=$(printf '%s' "$line" | tr -d '\r' | sed 's/#.*//' | xargs || true); \
          [ -n "$line" ] || continue; \
          name=$(basename "$line" .git); \
          dest="/opt/koma/src-tools/$name"; \
          if git clone --depth 1 "$line" "$dest" 2>/dev/null; then \
            echo "ok_src_clone $line" >> "$REPORT"; \
            if [ -f "$dest/setup.py" ] || [ -f "$dest/pyproject.toml" ]; then \
              /opt/koma/venv/bin/pip install --no-cache-dir "$dest" \
                && echo "ok_src_pip $name" >> "$REPORT" \
                || echo "soft_src_pip $name" >> "$REPORT"; \
            elif [ -f "$dest/Cargo.toml" ] && command -v cargo >/dev/null 2>&1; then \
              (cd "$dest" && cargo install --path . --root /opt/koma) \
                && echo "ok_src_cargo $name" >> "$REPORT" \
                || echo "soft_src_cargo $name" >> "$REPORT"; \
            else \
              echo "src_cloned_only $name" >> "$REPORT"; \
            fi; \
          else \
            echo "miss_src $line" >> "$REPORT"; \
          fi; \
        done < "$B/${C}.src.txt"; \
      fi; \
      # --- pip (handler libs; venv, not system python) --- \
      if file_nonempty "$B/${C}.pip.txt"; then \
        while IFS= read -r line || [ -n "$line" ]; do \
          line=$(printf '%s' "$line" | tr -d '\r' | sed 's/#.*//' | xargs || true); \
          [ -n "$line" ] || continue; \
          if /opt/koma/venv/bin/pip install --no-cache-dir "$line"; then \
            echo "ok_pip $line" >> "$REPORT"; \
          else \
            echo "miss_pip $line" >> "$REPORT"; \
          fi; \
        done < "$B/${C}.pip.txt"; \
      fi; \
      # --- gem --- \
      if file_nonempty "$B/${C}.gem.txt"; then \
        if ! command -v gem >/dev/null 2>&1; then \
          apt-get install -y --no-install-recommends ruby \
            || echo "miss ruby" >> "$REPORT"; \
        fi; \
        if command -v gem >/dev/null 2>&1; then \
          while IFS= read -r line || [ -n "$line" ]; do \
            line=$(printf '%s' "$line" | tr -d '\r' | sed 's/#.*//' | xargs || true); \
            [ -n "$line" ] || continue; \
            if gem install --no-document "$line"; then \
              echo "ok_gem $line" >> "$REPORT"; \
            else \
              echo "miss_gem $line" >> "$REPORT"; \
            fi; \
          done < "$B/${C}.gem.txt"; \
        fi; \
      fi; \
      # --- ZAP (cross-platform zip; zap.sh is what sec_zap's detect looks for) --- \
      if [ -f "$B/${C}.zap.url" ]; then \
        zap_url=$(grep -vE '^\s*(#|$)' "$B/${C}.zap.url" | head -1 || true); \
        if [ -n "$zap_url" ]; then \
          mkdir -p /opt/zaproxy; \
          if curl -fsSL -o /tmp/zap.zip "$zap_url" \
             && unzip -q /tmp/zap.zip -d /opt/zaproxy; then \
            zap_sh=$(find /opt/zaproxy -name zap.sh -type f | head -1 || true); \
            if [ -n "$zap_sh" ]; then \
              chmod 0755 "$zap_sh" || true; \
              ln -sf "$zap_sh" /opt/koma/bin/zap.sh; \
              echo "ok_zap $zap_url" >> "$REPORT"; \
            else \
              echo "miss_zap no zap.sh" >> "$REPORT"; \
            fi; \
          else \
            echo "miss_zap $zap_url" >> "$REPORT"; \
          fi; \
          rm -f /tmp/zap.zip; \
        fi; \
      fi; \
      # RsaCtfTool clones but does not install a PATH entry. \
      rsa=/opt/koma/src-tools/RsaCtfTool; \
      if [ -f "$rsa/RsaCtfTool.py" ]; then \
        if [ -f "$rsa/requirements.txt" ]; then \
          /opt/koma/venv/bin/pip install --no-cache-dir -r "$rsa/requirements.txt" \
            && echo "ok_rsa_req" >> "$REPORT" \
            || echo "soft_rsa_req" >> "$REPORT"; \
        fi; \
        printf '%s\n' '#!/bin/sh' "exec /opt/koma/venv/bin/python $rsa/RsaCtfTool.py \"\$@\"" \
          > /opt/koma/bin/RsaCtfTool; \
        chmod 0755 /opt/koma/bin/RsaCtfTool; \
        echo "ok_rsa_wrap" >> "$REPORT"; \
      fi; \
      rm -rf /var/lib/apt/lists/*; \
    cp "$REPORT" /tmp/install_report_final.txt

RUN set -eux; \
    if ! id -u koma >/dev/null 2>&1; then \
      useradd --create-home --uid 10001 koma || useradd --create-home koma; \
    fi; \
    mkdir -p /work /artifacts /opt/koma/bin /opt/koma/catalog /opt/koma/src-tools; \
    chown -R koma:koma /work /artifacts /opt/koma || true; \
    cp /tmp/install_report_final.txt /opt/koma/install_report.txt || true

COPY entrypoint.sh /usr/local/bin/entrypoint.sh
COPY koma_proxy.py /opt/koma/koma_proxy.py
COPY koma_sec_daemon /opt/koma/koma_sec_daemon
COPY catalog /opt/koma/catalog

# Composition-specific bin detect list (fallback: empty)
RUN set -eux; \
    C="${KOMA_COMPOSITION}"; \
    BINS_SRC="/tmp/install_recipes/buckets/${C}.bins.txt"; \
    if [ -f "$BINS_SRC" ]; then \
      cp "$BINS_SRC" /opt/koma/catalog/detect_bins.txt; \
    else \
      : > /opt/koma/catalog/detect_bins.txt; \
    fi

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
ir = Path("/opt/koma/install_report.txt")
if ir.is_file():
    text = ir.read_text(encoding="utf-8", errors="replace")
    for key in ("ok ", "ok_batch ", "miss ", "ok_go ", "miss_go ", "ok_cargo ", "ok_npm ", "ok_src_clone "):
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
