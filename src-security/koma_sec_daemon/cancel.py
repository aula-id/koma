"""
Cancellation that actually stops work.

`future.cancel()` only stops work that has not started; a handler already running
keeps running. So cancellation here is really about the child processes a handler
spawned: `cancel(id)` kills that id's process group.

Process groups, not pids: a handler that shells out produces a tree, and killing
the leader alone leaves grandchildren orphaned and holding the port.
"""

from __future__ import annotations

import os
import signal
import subprocess
import threading

_LOCK = threading.Lock()
# frame id -> live children
_LIVE: dict[object, set[subprocess.Popen]] = {}
# Keys promoted to "everything", for a shutdown kill.
_ALL: set[object] = set()


def register(key: object, proc: subprocess.Popen) -> None:
    """Track a child against the frame that started it."""
    with _LOCK:
        _LIVE.setdefault(key, set()).add(proc)


def release(key: object) -> None:
    """Forget a frame. Called when its handler returns, however it returned."""
    with _LOCK:
        _LIVE.pop(key, None)


def _kill(proc: subprocess.Popen) -> bool:
    """Kill one child's whole group. False when it was already gone."""
    if proc.poll() is not None:
        return False
    try:
        # start_new_session puts the child in its own group, so the negative pid
        # addresses the tree rather than just this process.
        os.killpg(os.getpgid(proc.pid), signal.SIGTERM)
    except OSError:
        try:
            proc.kill()
        except OSError:
            return False
    return True


def cancel(key: object) -> int:
    """Kill every child registered for `key`. Returns how many were alive.

    Terminates rather than hard-killing, so a tool can flush what it has. The
    caller's own timeout is what escalates if that does not take.
    """
    with _LOCK:
        procs = list(_LIVE.pop(key, ()))
    killed = sum(1 for p in procs if _kill(p))
    if killed:
        print(f"[koma_sec_daemon] cancelled {killed} child(ren) for {key}", flush=True)
    return killed


def cancel_all() -> int:
    """Kill every tracked child, ignoring keys. Used on shutdown."""
    with _LOCK:
        procs = [p for procs in _LIVE.values() for p in procs]
        _LIVE.clear()
    return sum(1 for p in procs if _kill(p))


def wrap(key: object) -> callable:
    """Popen factory that auto-registers. Pass as `spawn=` to a handler.

    Existing handlers call `sandbox.run`, which accepts this seam, so nothing has
    to be rewritten per tool.
    """

    def spawn(argv, **kwargs):
        proc = subprocess.Popen(argv, **kwargs)  # noqa: S603 - argv is a list
        register(key, proc)
        return proc

    return spawn
