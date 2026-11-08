"""Explicit file ingress/egress for the security guest.

v1 stages everything under the host artifacts bind (mounted at /artifacts in the
guest). No full-workspace mount. Paths may not escape via .. or absolute jumps
outside the allowlisted roots.
"""
from __future__ import annotations

import os
import shutil
from pathlib import Path

from koma_sec_daemon import runtime_ops

# Soft cap so a model cannot fill the disk in one copy.
_MAX_BYTES = 64 * 1024 * 1024


def _safe_rel(dest: str) -> str:
    raw = (dest or "").strip().lstrip("/")
    if not raw:
        raise ValueError("destination path is empty")
    parts = Path(raw).parts
    if any(p in ("", ".", "..") for p in parts) or ".." in raw:
        raise ValueError(f"refusing unsafe path: {dest!r}")
    # Only artifacts (and optional work under artifacts/work).
    if parts[0] not in ("artifacts", "work"):
        # Bare relative → under artifacts/
        return str(Path("artifacts") / raw)
    return str(Path(*parts))


def _host_artifacts() -> Path:
    b = runtime_ops.current()
    if b is None:
        raise RuntimeError("boundary is down — call sec_runtime up first")
    b.artifacts.mkdir(parents=True, exist_ok=True)
    return b.artifacts


def _resolve_guest_rel(rel: str) -> Path:
    """Map guest-relative path to host path under the artifacts bind."""
    rel = _safe_rel(rel)
    art = _host_artifacts()
    # Guest sees /artifacts → host art. Guest /work → host art/work.
    p = Path(rel)
    if p.parts[0] == "artifacts":
        host = art.joinpath(*p.parts[1:]) if len(p.parts) > 1 else art
    else:
        # work/...
        host = art.joinpath(*p.parts)
    host = host.resolve()
    try:
        host.relative_to(art.resolve())
    except ValueError as exc:
        raise ValueError("path escapes artifacts root") from exc
    return host


def _resolve_host_src(src: str, allow_roots: list[str] | None) -> Path:
    path = Path(os.path.expanduser(src)).resolve()
    if not path.is_file():
        raise FileNotFoundError(f"source is not a file: {src}")
    size = path.stat().st_size
    if size > _MAX_BYTES:
        raise ValueError(f"source exceeds {_MAX_BYTES} bytes ({size})")
    if allow_roots:
        roots = [Path(os.path.expanduser(r)).resolve() for r in allow_roots]
        if not any(path == r or r in path.parents for r in roots):
            raise PermissionError(f"source not under allowed roots: {src}")
    return path


def copy_in(src: str, dest: str, allow_roots: list[str] | None = None) -> dict:
    host_src = _resolve_host_src(src, allow_roots)
    host_dest = _resolve_guest_rel(dest)
    host_dest.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(host_src, host_dest, follow_symlinks=False)
    rel = _safe_rel(dest)
    if rel.startswith("work/") or rel == "work":
        guest_path = "/" + rel
    elif rel.startswith("artifacts/"):
        guest_path = "/" + rel
    else:
        guest_path = "/artifacts/" + rel
    return {
        "ok": True,
        "src": str(host_src),
        "dest": guest_path,
        "bytes": host_src.stat().st_size,
    }


def copy_out(src: str, dest: str) -> dict:
    """Copy from guest path (under /artifacts or /work) to a host destination file."""
    host_src = _resolve_guest_rel(src)
    if not host_src.is_file():
        raise FileNotFoundError(f"guest file not found: {src}")
    size = host_src.stat().st_size
    if size > _MAX_BYTES:
        raise ValueError(f"source exceeds {_MAX_BYTES} bytes ({size})")
    host_dest = Path(os.path.expanduser(dest)).resolve()
    host_dest.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(host_src, host_dest, follow_symlinks=False)
    return {"ok": True, "src": src, "dest": str(host_dest), "bytes": size}


def ls(path: str = "artifacts") -> dict:
    host = _resolve_guest_rel(path if path not in ("", "/") else "artifacts")
    if not host.exists():
        return {"path": path, "entries": []}
    if not host.is_dir():
        raise NotADirectoryError(path)
    entries = []
    for child in sorted(host.iterdir(), key=lambda p: p.name):
        try:
            st = child.stat(follow_symlinks=False)
            entries.append(
                {
                    "name": child.name,
                    "type": "dir" if child.is_dir() else "file",
                    "bytes": st.st_size if child.is_file() else None,
                }
            )
        except OSError:
            continue
    return {"path": path, "entries": entries}


def handle(op: str, args: dict | None = None) -> dict:
    args = args or {}
    op = (op or "").strip().lower()
    if op == "copy_in":
        src = args.get("src")
        dest = args.get("dest") or args.get("dst")
        if not isinstance(src, str) or not isinstance(dest, str):
            raise ValueError("copy_in requires string src and dest")
        roots = args.get("allow_roots")
        if roots is not None and not isinstance(roots, list):
            roots = None
        return copy_in(src, dest, allow_roots=roots)
    if op == "copy_out":
        src = args.get("src")
        dest = args.get("dest") or args.get("dst")
        if not isinstance(src, str) or not isinstance(dest, str):
            raise ValueError("copy_out requires string src and dest")
        return copy_out(src, dest)
    if op == "ls":
        path = args.get("path") or args.get("dest") or "artifacts"
        if not isinstance(path, str):
            raise ValueError("ls requires string path")
        return ls(path)
    raise ValueError(f"unknown fs op: {op!r}")
