"""What the active guest image can actually run.

The daemon runs on the host because it drives Docker, but tools run in the guest.
Readiness must therefore come from the guest image, never from the host's PATH:
a Mac without `nmap` says nothing about a `recon` image that ships it.

One `docker run --network none --pull never` per image asks the image which of
the catalog's requirements it has (a binary on PATH or an importable module).
The answer is cached in memory and, for digest-pinned refs, on disk, so a
daemon restart on the same image costs nothing.

Inside the guest itself (image build, `guest_run`) the local PATH is the guest,
so the probe reads it directly.
"""
from __future__ import annotations

import hashlib
import importlib.util
import json
import os
import shutil
import subprocess
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path

PROBE_TIMEOUT_S = 90.0
# A failed probe (Docker down, image missing) is retried no sooner than this.
RETRY_AFTER_S = 15.0
_GUEST_ROOT = Path("/opt/koma")

# Runs inside the image. Reads {"names": [...]} on stdin, prints the present ones.
_PROBE_SCRIPT = r"""
import importlib.util, json, shutil, sys
names = json.load(sys.stdin)["names"]
def has(n):
    if shutil.which(n):
        return True
    try:
        return importlib.util.find_spec(n) is not None
    except Exception:
        return False
print(json.dumps({"present": [n for n in names if has(n)]}))
"""


@dataclass
class Probe:
    """Result for one image. `present` is None when the probe failed."""

    image: str
    present: frozenset[str] | None = None
    error: str | None = None
    names: frozenset[str] = field(default_factory=frozenset)
    at: float = field(default_factory=time.monotonic)

    @property
    def ok(self) -> bool:
        return self.present is not None

    def has(self, name: str) -> bool:
        return self.present is not None and name in self.present


def in_guest() -> bool:
    """True when this process is the guest image (its PATH is the toolkit)."""
    if os.environ.get("KOMA_SEC_IN_GUEST", "").strip() == "1":
        return True
    try:
        return Path(__file__).resolve().parent.parent == _GUEST_ROOT
    except OSError:
        return False


def active_image() -> str:
    return os.environ.get("KOMA_SEC_IMAGE") or "ghcr.io/aula-id/koma-safety:latest"


def local_has(name: str) -> bool:
    """The requirement on THIS machine. Only for the guest itself or host-run tools."""
    if shutil.which(name):
        return True
    try:
        return importlib.util.find_spec(name) is not None
    except (ImportError, ValueError):
        return False


def _cache_dir() -> Path:
    base = os.environ.get("KOMA_SEC_PROBE_CACHE")
    if base:
        return Path(base)
    return Path(os.path.expanduser("~")) / ".koma" / "security" / "probe"


def _cache_path(image: str, names: frozenset[str]) -> Path | None:
    # Only a digest pins content; a floating tag can change under the same name.
    if "@sha256:" not in image:
        return None
    key = hashlib.sha256(
        (image + "\n" + "\n".join(sorted(names))).encode("utf-8")
    ).hexdigest()[:32]
    return _cache_dir() / f"{key}.json"


def _read_cache(path: Path | None) -> frozenset[str] | None:
    if path is None:
        return None
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
        present = data.get("present")
        if isinstance(present, list):
            return frozenset(str(x) for x in present)
    except (OSError, ValueError):
        pass
    return None


def _write_cache(path: Path | None, present: frozenset[str]) -> None:
    if path is None:
        return
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps({"present": sorted(present)}), encoding="utf-8")
    except OSError:
        pass


def _run_probe(image: str, names: frozenset[str]) -> Probe:
    if not shutil.which("docker"):
        return Probe(image, error="docker is not installed", names=names)
    argv = [
        "docker", "run", "--rm", "-i",
        "--network", "none",
        "--pull", "never",
        "--entrypoint", "python3",
        image, "-c", _PROBE_SCRIPT,
    ]
    try:
        res = subprocess.run(
            argv,
            input=json.dumps({"names": sorted(names)}),
            capture_output=True,
            text=True,
            timeout=PROBE_TIMEOUT_S,
        )
    except subprocess.TimeoutExpired:
        return Probe(image, error=f"guest probe timed out after {PROBE_TIMEOUT_S:.0f}s", names=names)
    except OSError as exc:
        return Probe(image, error=f"guest probe failed: {exc}", names=names)
    if res.returncode != 0:
        tail = (res.stderr or res.stdout or "").strip().splitlines()
        why = tail[-1] if tail else f"exit {res.returncode}"
        return Probe(image, error=f"guest probe failed: {why}", names=names)
    try:
        present = frozenset(json.loads(res.stdout.strip().splitlines()[-1])["present"])
    except (ValueError, KeyError, IndexError, TypeError) as exc:
        return Probe(image, error=f"guest probe returned bad output: {exc}", names=names)
    return Probe(image, present=present, names=names)


_LOCK = threading.Lock()
_DONE = threading.Condition(_LOCK)
_PROBES: dict[str, Probe] = {}
_RUNNING: set[str] = set()


def _probe_now(image: str, names: frozenset[str]) -> Probe:
    path = _cache_path(image, names)
    cached = _read_cache(path)
    if cached is not None:
        return Probe(image, present=cached, names=names)
    if in_guest():
        return Probe(image, present=frozenset(n for n in names if local_has(n)), names=names)
    probe = _run_probe(image, names)
    if probe.present is not None:
        _write_cache(path, probe.present)
    return probe


def _worker(image: str, names: frozenset[str]) -> None:
    try:
        probe = _probe_now(image, names)
    except Exception as exc:  # noqa: BLE001 - a probe bug must not kill the daemon
        probe = Probe(image, error=f"guest probe crashed: {exc}", names=names)
    with _DONE:
        _PROBES[image] = probe
        _RUNNING.discard(image)
        _DONE.notify_all()


def start(names: frozenset[str], image: str | None = None) -> None:
    """Begin probing in the background (idempotent per image)."""
    image = image or active_image()
    with _LOCK:
        known = _PROBES.get(image)
        if image in _RUNNING or (known is not None and known.ok and names <= known.names):
            return
        _RUNNING.add(image)
    threading.Thread(target=_worker, args=(image, names), name="sec-guest-probe", daemon=True).start()


def current(names: frozenset[str], wait: float = 0.0, image: str | None = None) -> Probe | None:
    """The probe for the active image. None while it is still running after `wait`.

    A failed probe is retried on the next call, so starting Docker or pulling the
    image fixes readiness without restarting the daemon.
    """
    image = image or active_image()
    with _LOCK:
        known = _PROBES.get(image)
    if known is not None and known.ok and names <= known.names:
        return known
    if known is not None and not known.ok:
        if time.monotonic() - known.at < RETRY_AFTER_S:
            return known
        with _LOCK:
            if image not in _RUNNING:
                _PROBES.pop(image, None)
    start(names, image)
    with _DONE:
        if wait > 0:
            _DONE.wait_for(lambda: image not in _RUNNING, timeout=wait)
        probe = _PROBES.get(image)
        if image in _RUNNING:
            return None
        return probe


def reset() -> None:
    """Forget every probe (tests, or after an image changes under a tag)."""
    with _LOCK:
        _PROBES.clear()
