"""Run a pack-only catalog binary without a dedicated Python handler.

Model calls sec_run with name=<catalog id> and args:
  { "argv": ["-h"] } or { "raw": "-h --json" } optional timeout seconds.
"""
from __future__ import annotations

import shlex
import shutil
import subprocess
from typing import Any


def _bin_from_row(row: dict | None, fallback: str) -> str:
    if row:
        req = row.get("requires") or []
        if req and isinstance(req[0], str) and req[0].strip():
            return req[0].strip()
        how = str(row.get("how_to") or "")
        if "detect=" in how:
            return how.split("detect=", 1)[1].split(";")[0].strip() or fallback
    return fallback


def resolve_bin(tool_name: str) -> tuple[str | None, str | None]:
    """Return (bin_path_or_name, error). error set when missing."""
    from koma_sec_daemon import catalog

    row = catalog.get_tool(tool_name, lane=None)
    bin_name = _bin_from_row(row, tool_name)
    path = shutil.which(bin_name)
    if not path:
        return None, f"{tool_name}: binary not on PATH ({bin_name})"
    return path, None


def run(tool_name: str, args: dict[str, Any] | None = None) -> str:
    args = args or {}
    path, err = resolve_bin(tool_name)
    if err or not path:
        try:
            from koma_sec_daemon import verdict

            return verdict.typed(verdict.UNAVAILABLE, err or "missing binary")
        except Exception:
            return err or "missing binary"

    timeout = args.get("timeout", 120)
    try:
        timeout_f = float(timeout)
    except (TypeError, ValueError):
        timeout_f = 120.0
    timeout_f = max(1.0, min(timeout_f, 600.0))

    argv: list[str] = [path]
    if isinstance(args.get("argv"), list):
        argv.extend(str(x) for x in args["argv"])
    elif isinstance(args.get("raw"), str) and args["raw"].strip():
        argv.extend(shlex.split(args["raw"]))
    elif isinstance(args.get("args"), str) and args["args"].strip():
        argv.extend(shlex.split(args["args"]))

    try:
        proc = subprocess.run(
            argv,
            capture_output=True,
            text=True,
            timeout=timeout_f,
            check=False,
        )
    except subprocess.TimeoutExpired:
        msg = f"{tool_name}: timed out after {timeout_f}s"
        try:
            from koma_sec_daemon import verdict

            return verdict.typed(verdict.ERROR, msg)
        except Exception:
            return msg
    except OSError as exc:
        msg = f"{tool_name}: {exc}"
        try:
            from koma_sec_daemon import verdict

            return verdict.typed(verdict.ERROR, msg)
        except Exception:
            return msg

    out = (proc.stdout or "") + (("\n" + proc.stderr) if proc.stderr else "")
    out = out.strip() or f"(exit {proc.returncode}, empty output)"
    if len(out) > 100_000:
        out = out[:100_000] + "\n…[truncated]"
    try:
        from koma_sec_daemon import verdict

        if proc.returncode != 0:
            return verdict.typed(
                verdict.ERROR,
                f"exit {proc.returncode}\n{out}",
            )
        # Success path: untyped body is fine; parent wraps.
        return verdict.typed(verdict.UNTYPED, out)
    except Exception:
        return out