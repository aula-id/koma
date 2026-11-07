"""Boundary lifecycle for the security guest (koma-safety).

Host daemon owns the control plane: up/down/status/health. Tool bodies should run
inside the guest once the boundary is up; missing boundary never falls back silently
for runtime ops themselves.
"""
from __future__ import annotations

import os
import secrets
import threading
from pathlib import Path

from koma_sec_daemon import boundary as boundary_mod

# One boundary per daemon process (one parent session). Protected for the call pool.
_LOCK = threading.Lock()
_BOUNDARY: boundary_mod.Boundary | None = None
_SCOPE_KEY: str = ""
_RUN_DIR: Path | None = None


def _default_image() -> str:
    return os.environ.get("KOMA_SEC_IMAGE") or "ghcr.io/aula-id/koma-safety:latest"


def _run_dir() -> Path:
    global _RUN_DIR
    if _RUN_DIR is not None:
        return _RUN_DIR
    base = os.environ.get("KOMA_SEC_RUN_DIR")
    if base:
        _RUN_DIR = Path(base)
    else:
        home = Path(os.path.expanduser("~")) / ".koma" / "security" / "runs"
        _RUN_DIR = home / (_SCOPE_KEY or "default")
    _RUN_DIR.mkdir(parents=True, exist_ok=True)
    return _RUN_DIR


def _guest_running(b: boundary_mod.Boundary) -> bool:
    if not b.guest:
        return False
    res = boundary_mod._run(  # noqa: SLF001 - shared docker helper
        ["docker", "inspect", "-f", "{{.State.Running}}", b.guest]
    )
    return res.returncode == 0 and res.stdout.strip().lower() == "true"


def status() -> dict:
    with _LOCK:
        b = _BOUNDARY
        if b is None:
            problem = boundary_mod.preflight()
            return {
                "up": False,
                "image": _default_image(),
                "scope_key": _SCOPE_KEY or None,
                "preflight": problem,
                "guest": None,
                "proxy": None,
                "network": None,
            }
        up = _guest_running(b)
        out = {
            "up": up,
            "image": b.image,
            "scope_key": b.scope_key,
            "guest": b.guest or None,
            "proxy": b.proxy or None,
            "network": b.network or None,
            "artifacts": str(b.artifacts),
            "preflight": None,
        }
        try:
            rpath = b.rules_path
            rlines = [
                ln.strip()
                for ln in rpath.read_text(encoding="utf-8").splitlines()
                if ln.strip() and not ln.strip().startswith("#")
            ]
            out["net"] = "off" if not rlines else "scope"
            out["rules"] = rlines
            out["rules_count"] = len(rlines)
        except OSError:
            out["net"] = "unknown"
            out["rules"] = []
            out["rules_count"] = 0
        if up:
            try:
                out["record"] = b.record()
            except Exception as exc:  # noqa: BLE001
                out["record_error"] = str(exc)
        return out


def up(targets: list[str] | None = None, exclusions: list[str] | None = None) -> dict:
    """Bring the boundary up. Fail-closed — never runs tools on the host as a substitute."""
    global _BOUNDARY, _SCOPE_KEY
    with _LOCK:
        problem = boundary_mod.preflight()
        if problem:
            raise boundary_mod.BoundaryError(problem)
        if _BOUNDARY is not None and _guest_running(_BOUNDARY):
            return {"up": True, "already": True, "record": _BOUNDARY.record()}
        if not _SCOPE_KEY:
            _SCOPE_KEY = secrets.token_hex(6)
        run_dir = _run_dir()
        image = _default_image()
        b = boundary_mod.new_boundary(_SCOPE_KEY, image, run_dir)
        # Default empty rules = deny-all egress until scope writes rules. Callers may
        # pass targets for an explicit allow list.
        boundary_mod.write_rules(
            b.rules_path,
            boundary_mod.rules_for(targets or [], exclusions or []),
        )
        try:
            b.up()
        except Exception:
            b.down()
            raise
        _BOUNDARY = b
        return {"up": True, "already": False, "record": b.record(), "artifacts": str(b.artifacts)}


def down() -> dict:
    global _BOUNDARY
    with _LOCK:
        if _BOUNDARY is None:
            return {"up": False, "cleared": False}
        try:
            _BOUNDARY.down()
        finally:
            _BOUNDARY = None
        return {"up": False, "cleared": True}


def health() -> dict:
    st = status()
    st["docker_ok"] = boundary_mod.preflight() is None
    st["healthy"] = bool(st.get("up")) and st["docker_ok"]
    return st


def ensure_up() -> boundary_mod.Boundary:
    """Lazy bring-up for sec_run. Raises BoundaryError on failure."""
    with _LOCK:
        if _BOUNDARY is not None and _guest_running(_BOUNDARY):
            return _BOUNDARY
    up()
    with _LOCK:
        if _BOUNDARY is None or not _guest_running(_BOUNDARY):
            raise boundary_mod.BoundaryError("boundary failed to start")
        return _BOUNDARY




def set_net(mode: str, targets: list[str] | None = None, exclusions: list[str] | None = None) -> dict:
    """Rewrite egress rules while the boundary is up (or prepare rules path).

    mode=off  → empty rules (deny all)
    mode=scope → rules from targets/exclusions
    """
    mode = (mode or "").strip().lower()
    with _LOCK:
        b = _BOUNDARY
        if b is None:
            raise boundary_mod.BoundaryError("boundary is down — sec_runtime up first")
        if mode in ("off", "none", "deny"):
            lines: list[str] = []
            mode_out = "off"
        elif mode in ("scope", "on", "allow"):
            lines = boundary_mod.rules_for(targets or [], exclusions or [])
            mode_out = "scope"
        else:
            raise ValueError(f"unknown net mode: {mode!r} (use off|scope)")
        boundary_mod.write_rules(b.rules_path, lines)
        return {
            "up": _guest_running(b),
            "net": mode_out,
            "rules": len(lines),
            "rules_path": str(b.rules_path),
        }


def rules_snapshot() -> dict:
    with _LOCK:
        b = _BOUNDARY
        if b is None:
            return {"up": False, "rules": [], "net": "down"}
        path = b.rules_path
        lines = []
        try:
            text = path.read_text(encoding="utf-8")
            lines = [
                ln.strip()
                for ln in text.splitlines()
                if ln.strip() and not ln.strip().startswith("#")
            ]
        except OSError:
            pass
        return {
            "up": _guest_running(b),
            "rules": lines,
            "net": "off" if not lines else "scope",
            "rules_path": str(path),
        }

def current() -> boundary_mod.Boundary | None:
    with _LOCK:
        return _BOUNDARY


def handle(op: str, args: dict | None = None) -> dict:
    args = args or {}
    op = (op or "").strip().lower()
    if op == "status":
        return status()
    if op == "up":
        targets = args.get("targets") if isinstance(args.get("targets"), list) else None
        exclusions = args.get("exclusions") if isinstance(args.get("exclusions"), list) else None
        return up(targets=targets, exclusions=exclusions)
    if op == "down":
        return down()
    if op == "health":
        return health()
    if op in ("net", "net_set"):
        mode = args.get("mode") if isinstance(args.get("mode"), str) else "off"
        targets = args.get("targets") if isinstance(args.get("targets"), list) else None
        exclusions = args.get("exclusions") if isinstance(args.get("exclusions"), list) else None
        return set_net(mode, targets=targets, exclusions=exclusions)
    if op in ("rules", "net_status"):
        return rules_snapshot()
    raise ValueError(f"unknown runtime op: {op!r}")
