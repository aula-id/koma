"""
installer — tiered, best-effort dependency installer.

install(key) dispatches on the manifest descriptor's "method":

  manual  -> never touches the system; returns the manual hint.
  pip     -> [sys.executable, -m, pip, install, <pkgs>] then any "post" argv
             run with sys.executable. Because sys.executable is the daemon's
             venv python (~/.koma/security/venv), this installs into that venv.
  gem     -> [gem, install, --user-install, <gem>] (requires ruby/gem on PATH).
  binary  -> resolve the latest GitHub release, download the asset matching
             asset_re, extract the wanted members into ~/.koma/security/bin and
             chmod 0o755 each.

Every path returns a STRING summary (success or "error: …"). Nothing raises
across the IPC boundary — the daemon hands this result straight back in the
{"id","ok","result"} envelope. requests is imported LAZILY (binary path only)
so this module is importable with ZERO toolkit installed.

Stdlib only at module level.
"""

import os
import re
import subprocess
import sys
import tarfile
import tempfile
import zipfile

from koma_sec_daemon.install_manifest import KEY_INDEX, bin_dir

# Generous wall-clock budget — pip building wheels / playwright fetching
# chromium can be slow on a cold cache.
_PIP_TIMEOUT = 600
_GEM_TIMEOUT = 600
_NET_TIMEOUT = 30

# How much subprocess output to echo back (tail, the useful part).
_TAIL_CHARS = 2000


def _tail(text: str, n: int = _TAIL_CHARS) -> str:
    """Return the last *n* chars of *text*, prefixed with an elision marker."""
    text = text or ""
    if len(text) <= n:
        return text
    return "…" + text[-n:]


def _run(argv: list, timeout: int) -> tuple:
    """
    Run *argv*, capturing combined stdout+stderr as text.

    Returns (rc, output). rc is None on timeout / launch failure, with a
    human-readable note in output. Never raises.
    """
    try:
        proc = subprocess.run(
            argv,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            timeout=timeout,
        )
        return proc.returncode, (proc.stdout or "")
    except subprocess.TimeoutExpired:
        return None, f"[timed out after {timeout}s]"
    except Exception as exc:  # FileNotFoundError, etc.
        return None, f"[failed to launch: {exc}]"


def _install_pip(dep: dict) -> str:
    """pip-install the package(s), then run any post commands. Returns a summary."""
    pkgs = dep.get("pip") or []
    if not pkgs:
        return f"error: install {dep['key']} failed: no pip packages declared"

    rc, out = _run([sys.executable, "-m", "pip", "install", *pkgs], _PIP_TIMEOUT)
    if rc != 0:
        return (
            f"error: pip install failed for {dep['key']} "
            f"(rc={rc}); {dep['hint']}\n{_tail(out)}"
        )

    # Optional post-install steps (e.g. `playwright install chromium`), each run
    # with the venv python: [sys.executable, *post_argv].
    for post in dep.get("post") or []:
        prc, pout = _run([sys.executable, *post], _PIP_TIMEOUT)
        if prc != 0:
            return (
                f"error: post-install step {post} failed for {dep['key']} "
                f"(rc={prc}); {dep['hint']}\n{_tail(pout)}"
            )

    return f"installed {dep['key']} ({', '.join(pkgs)}) via pip\n{_tail(out)}"


def _install_gem(dep: dict) -> str:
    """gem-install (user-install) the gem. Returns a summary."""
    import shutil  # stdlib; local import keeps the module head tidy

    if shutil.which("gem") is None:
        return f"error: ruby/gem not found; {dep['hint']}"

    gem = dep["gem"]
    rc, out = _run(["gem", "install", "--user-install", gem], _GEM_TIMEOUT)
    if rc != 0:
        return (
            f"error: gem install failed for {dep['key']} "
            f"(rc={rc}); {dep['hint']}\n{_tail(out)}"
        )
    return f"installed {dep['key']} ({gem}) via gem --user-install\n{_tail(out)}"


def _extract_member(archive_path: str, member_name: str, dest_path: str) -> bool:
    """
    Find *member_name* (by basename, anywhere in the archive) inside the
    zip/tar at *archive_path* and write it to *dest_path*, chmod 0o755.

    Returns True on success, False if the member was not found. Archive type is
    chosen by extension. Raises only on genuinely corrupt archives (the caller
    wraps the whole binary install in a try/except).
    """
    lower = archive_path.lower()

    if lower.endswith(".zip"):
        with zipfile.ZipFile(archive_path) as zf:
            for info in zf.infolist():
                if info.is_dir():
                    continue
                if os.path.basename(info.filename) == member_name:
                    with zf.open(info) as src, open(dest_path, "wb") as dst:
                        dst.write(src.read())
                    os.chmod(dest_path, 0o755)
                    return True
        return False

    # Treat everything else as a tarball (.tar.gz / .tgz / .tar.xz / .tar.bz2).
    with tarfile.open(archive_path) as tf:
        for info in tf.getmembers():
            if not info.isfile():
                continue
            if os.path.basename(info.name) == member_name:
                src = tf.extractfile(info)
                if src is None:
                    return False
                with src, open(dest_path, "wb") as dst:
                    dst.write(src.read())
                os.chmod(dest_path, 0o755)
                return True
    return False


def _install_binary(dep: dict) -> str:
    """
    Download the latest GitHub release asset matching dep["asset_re"] and extract
    dep["members"] into ~/.koma/security/bin. Returns a summary string.

    Linux-only for v1. requests is imported lazily here.
    """
    key = dep["key"]
    hint = dep["hint"]

    if not sys.platform.startswith("linux"):
        return f"error: install {key} unsupported on this platform ({sys.platform}); {hint}"

    try:
        import requests  # noqa: PLC0415 — lazy, optional dependency

        api = f"https://api.github.com/repos/{dep['repo']}/releases/latest"
        resp = requests.get(
            api,
            headers={"Accept": "application/vnd.github+json"},
            timeout=_NET_TIMEOUT,
        )
        if resp.status_code != 200:
            return (
                f"error: install failed for {key}: GitHub API returned "
                f"{resp.status_code}; {hint}"
            )

        assets = resp.json().get("assets") or []
        pattern = re.compile(dep["asset_re"], re.IGNORECASE)
        chosen = None
        for asset in assets:
            name = asset.get("name", "")
            if pattern.search(name):
                chosen = asset
                break

        if chosen is None:
            return f"error: no matching release asset for {key} on this platform; {hint}"

        url = chosen.get("browser_download_url")
        if not url:
            return f"error: install failed for {key}: asset has no download URL; {hint}"

        # Stream the asset to a temp file (suffix preserved so extraction can
        # pick zip vs tar by extension).
        suffix = "." + chosen["name"].split(".", 1)[1] if "." in chosen["name"] else ""
        fd, tmp_path = tempfile.mkstemp(suffix=suffix)
        try:
            with os.fdopen(fd, "wb") as fh:
                dl = requests.get(url, timeout=_NET_TIMEOUT, stream=True)
                if dl.status_code != 200:
                    return (
                        f"error: install failed for {key}: download returned "
                        f"{dl.status_code}; {hint}"
                    )
                for chunk in dl.iter_content(chunk_size=65536):
                    if chunk:
                        fh.write(chunk)

            dest_dir = bin_dir()
            os.makedirs(dest_dir, exist_ok=True)

            written = []
            missing = []
            for member in dep["members"]:
                dest_path = os.path.join(dest_dir, member)
                if _extract_member(tmp_path, member, dest_path):
                    written.append(dest_path)
                else:
                    missing.append(member)

            if missing:
                return (
                    f"error: install failed for {key}: members not found in archive "
                    f"({', '.join(missing)}); {hint}"
                )

            return f"installed {key} -> {', '.join(written)}"
        finally:
            try:
                os.remove(tmp_path)
            except OSError:
                pass

    except Exception as exc:
        # Network errors, rate-limit JSON parse failures, corrupt archives, etc.
        return f"error: install failed for {key}: {exc}; {hint}"


def install(key: str) -> str:
    """
    Install the dependency identified by *key*. Returns a status string.

    Tier-3 ("manual") deps are never auto-installed — their manual hint is
    returned. Unknown keys and any unexpected error are reported as strings so
    nothing ever raises across the IPC boundary.
    """
    try:
        dep = KEY_INDEX[key]
    except KeyError:
        return f"error: unknown dependency: {key}"

    method = dep.get("method")
    try:
        if method == "manual":
            return "manual install required: " + dep["hint"]
        if method == "pip":
            return _install_pip(dep)
        if method == "gem":
            return _install_gem(dep)
        if method == "binary":
            return _install_binary(dep)
        return f"error: install {key} failed: unknown method {method!r}"
    except Exception as exc:
        return f"error: install {key} failed: {exc}"
