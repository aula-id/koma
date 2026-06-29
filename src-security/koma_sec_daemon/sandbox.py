"""
Execution-hygiene layer for running untrusted subprocesses.

run() wraps subprocess.run() with:
  - Hard wall-clock timeout (subprocess.run timeout=)
  - Optional RLIMIT_AS + RLIMIT_CPU via preexec_fn (Unix only)
  - Optional bubblewrap (bwrap) network-isolation sandbox
  - Output cap at 400 000 chars with a truncation note
"""

import subprocess
import sys
from typing import Optional

_TRUNCATE_AT = 400_000
_TRUNCATE_NOTE = "\n…[truncated]"

# bwrap prefix for --unshare-net sandboxing (best-effort; bwrap must be installed)
_BWRAP_PREFIX = [
    "bwrap",
    "--unshare-net",
    "--die-with-parent",
    "--dev", "/dev",
    "--proc", "/proc",
    "--ro-bind", "/", "/",
]


def _make_preexec(mem_mb: Optional[int]):
    """Return a preexec_fn that sets RLIMIT_AS and RLIMIT_CPU (Unix only)."""
    try:
        import resource  # noqa: PLC0415
    except ImportError:
        # Non-Unix platform — no resource limits available
        return None

    def preexec():
        if mem_mb is not None:
            limit = mem_mb * 1024 * 1024
            resource.setrlimit(resource.RLIMIT_AS, (limit, limit))
        # 60 s CPU hard limit — prevents runaway compute
        resource.setrlimit(resource.RLIMIT_CPU, (60, 60))

    return preexec


def run(
    cmd: list[str],
    timeout: int,
    mem_mb: Optional[int] = None,
    bwrap: bool = False,
) -> str:
    """
    Run *cmd* as a subprocess.

    Parameters
    ----------
    cmd:     Command + arguments list.
    timeout: Wall-clock timeout in seconds. Returns "[timed out after Ns]" on expiry.
    mem_mb:  If set, cap virtual address space (RLIMIT_AS) to this many MiB.
             Also installs a 60-second RLIMIT_CPU. Unix only; silently skipped elsewhere.
    bwrap:   If True, wrap the command in bubblewrap with --unshare-net (best-effort).

    Returns
    -------
    Combined stdout+stderr as a string, capped at 400 000 characters.
    """
    if bwrap:
        cmd = _BWRAP_PREFIX + cmd

    preexec_fn = _make_preexec(mem_mb) if mem_mb is not None else None

    try:
        result = subprocess.run(
            cmd,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            timeout=timeout,
            preexec_fn=preexec_fn,
        )
        output = result.stdout or ""
    except subprocess.TimeoutExpired:
        return f"[timed out after {timeout}s]"
    except Exception as exc:
        return f"[error launching process: {exc}]"

    if len(output) > _TRUNCATE_AT:
        output = output[:_TRUNCATE_AT] + _TRUNCATE_NOTE

    return output
