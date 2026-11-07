"""
health — install-health probe for every external dependency.

probe() walks the MANIFEST (in order) and reports whether each dep is present
on this host. Pure stdlib (shutil, importlib.util) — no heavy/third-party
imports, so it is safe to call on a daemon with ZERO toolkit installed.

The returned dicts deliberately expose ONLY wire-safe fields (key, name, tier,
present, method, tools, hint) — never the install internals (repo, asset_re,
pip pkgs, …). The Rust /security cockpit consumes this list.
"""
from __future__ import annotations

import importlib.util
import shutil

from koma_sec_daemon.install_manifest import MANIFEST

# Fields safe to send over the wire (install internals are intentionally omitted).
_WIRE_KEYS = ("key", "name", "tier", "method", "tools", "hint")


def _is_present(detect: str, detect_kind: str) -> bool:
    """
    True if the dep identified by *detect* is installed.

    detect_kind == "which"  -> shutil.which(detect) resolves a binary on PATH.
    detect_kind == "import" -> importlib.util.find_spec(detect) finds the module.

    Any probe error (e.g. a broken/partial install raising on find_spec) is
    swallowed and reported as absent, never as an exception.
    """
    try:
        if detect_kind == "which":
            return shutil.which(detect) is not None
        if detect_kind == "import":
            return importlib.util.find_spec(detect) is not None
    except Exception:
        return False
    return False


def probe() -> list[dict]:
    """
    Return per-dependency health in MANIFEST order.

    Each entry: {key, name, tier, present, method, tools, hint}.
    Deterministic and side-effect free.
    """
    out: list[dict] = []
    for dep in MANIFEST:
        entry = {k: dep[k] for k in _WIRE_KEYS}
        entry["present"] = _is_present(dep["detect"], dep["detect_kind"])
        out.append(entry)
    return out
