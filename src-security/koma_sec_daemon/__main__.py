"""
koma_sec_daemon — long-lived security daemon entrypoint.

Protocol (newline-delimited JSON, parent speaks first):
  Handshake (parent → daemon):  {"v": 2, "token": "<T>"}   (token optional)
  Handshake (daemon → parent):  {"ok": true, "v": 2, "tools": [...descriptors...]}
                            or  {"ok": false, "error": "bad token" | "<refusal>"}

  `v` is the protocol MAJOR. A mismatch is refused, not guessed — see
  `protocol.negotiate`. Bump MAJOR when an older peer would mis-read a frame.

  Call (parent → daemon):       {"id": N, "op": "call", "tool": T, "args": {...}}
  Result (daemon → parent):     {"id": N, "ok": true, "result": "<str>"}
                            or  {"id": N, "ok": false, "error": "<str>"}

  EOF from parent → daemon exits cleanly, closing all sessions.

stdout is the frame channel (binary wire); all library/debug output goes to
stderr (same pattern as scrapion_agent/__main__.py).
"""
from __future__ import annotations

import sys
import os
import json
import argparse
import io


def _augment_path() -> None:
    """Append standard bin dirs to PATH so detection + tool subprocess calls find
    binaries installed outside the daemon's inherited PATH (e.g. ~/.local/bin,
    Homebrew). Only existing, not-already-present dirs are appended; existing PATH
    order is preserved."""
    home = os.path.expanduser("~")
    if sys.platform == "darwin":
        candidates = [
            os.path.join(home, ".local", "bin"),
            "/opt/homebrew/bin",
            "/opt/homebrew/sbin",
            "/usr/local/bin",
            "/usr/local/sbin",
            "/usr/bin",
            "/bin",
        ]
    else:  # linux and other unix
        candidates = [
            os.path.join(home, ".local", "bin"),
            "/usr/local/bin",
            "/usr/local/sbin",
            "/usr/bin",
            "/bin",
            "/usr/sbin",
            "/sbin",
        ]
    path = os.environ.get("PATH", "")
    existing = path.split(os.pathsep) if path else []
    existing_set = set(existing)
    additions = [d for d in candidates if os.path.isdir(d) and d not in existing_set]
    if additions:
        os.environ["PATH"] = os.pathsep.join(existing + additions)


def main() -> None:
    _augment_path()

    parser = argparse.ArgumentParser(
        description="koma security daemon — speak newline-delimited JSON on stdin/stdout",
        prog="python -m koma_sec_daemon",
    )
    parser.add_argument(
        "--token",
        default=None,
        help="Handshake token minted by koma; if provided the daemon verifies it.",
    )
    args = parser.parse_args()
    expected_token: str | None = args.token

    # --- Capture real stdout BEFORE redirecting ---
    # We need a text-mode, line-buffered writer for frames.
    # sys.stdout may be in binary mode if spawned with os.pipe, so we
    # wrap sys.stdout.buffer when available.
    if hasattr(sys.stdout, "buffer"):
        _raw_out = sys.stdout.buffer
        real_stdout = io.TextIOWrapper(_raw_out, encoding="utf-8", line_buffering=True)
    else:
        # Already text mode (e.g. interactive / tests piping text)
        real_stdout = sys.stdout

    # Redirect print() / library output to stderr so the frame channel stays clean
    sys.stdout = sys.stderr

    from koma_sec_daemon.protocol import read_frame, write_frame
    from koma_sec_daemon import registry
    from koma_sec_daemon import verdict
    from koma_sec_daemon.registry import descriptors, refused_descriptors, call
    from koma_sec_daemon.sessions import SessionStore

    sessions = SessionStore()

    # Ensure the persisted binary dir (where Tier-2 downloads land) is on PATH so
    # sandbox.run() subprocesses can find tools installed via the `install` op.
    # Done before the handshake so the very first tool call already sees them.
    from koma_sec_daemon.install_manifest import bin_dir
    _bd = bin_dir()
    os.makedirs(_bd, exist_ok=True)
    _venv_bin = os.path.dirname(sys.executable)
    os.environ["PATH"] = _bd + os.pathsep + _venv_bin + os.pathsep + os.environ.get("PATH", "")

    # --- HANDSHAKE ---
    try:
        frame = read_frame(sys.stdin)
    except ValueError as exc:
        write_frame(real_stdout, {"ok": False, "error": f"bad handshake frame: {exc}"})
        sys.exit(1)

    if frame is None:
        # EOF before handshake — nothing to do
        sys.exit(0)

    if expected_token is not None:
        received = frame.get("token")
        if received != expected_token:
            write_frame(real_stdout, {"ok": False, "error": "bad token"})
            sys.exit(1)

    from koma_sec_daemon.protocol import PROTOCOL_VERSION, negotiate

    refusal = negotiate(frame)
    if refusal is not None:
        write_frame(real_stdout, {"ok": False, "error": refusal})
        sys.exit(1)

    write_frame(
        real_stdout,
        {
            "ok": True,
            "v": PROTOCOL_VERSION,
            # Refused descriptors travel with the reply so the parent can surface
            # the reason. Absent (not empty) is fine: no refusals happened.
            "tools": descriptors(),
            # Separate from `tools`: a refusal is a bug, an unavailability is an
            # environment gap. Both travel so the parent can say the difference.
            "refused": refused_descriptors(),
            "unavailable": registry.unavailable_tools(),
        },
    )

    # --- MAIN LOOP ---
    #
    # The reader stays single-threaded and dispatch happens on a small pool, so a
    # slow tool cannot delay an unrelated call. Two rules keep that safe:
    #
    # - Each op is routed to keep the things that genuinely must be serialised
    #   serialised. `install` gets the dedicated install lane: it rewrites the
    #   shared bin dir and would race a running call's PATH lookup.
    # - Cancelling a future alone does NOT stop a running handler. `cancel` kills
    #   the process group of that id's active subprocesses, which is what actually
    #   ends a stuck tool.
    #
    # `call`/`install` each get their own pool; `health`/`cancel` are cheap and
    # stay on the loop. Every handler gets an `emit` so it can stream progress
    # frames for a long run instead of going silent.
    from concurrent.futures import ThreadPoolExecutor

    from koma_sec_daemon import cancel as cancel_mod

    calls = ThreadPoolExecutor(max_workers=4, thread_name_prefix="sec-call")
    installs = ThreadPoolExecutor(max_workers=1, thread_name_prefix="sec-install")

    def emit(payload: dict) -> None:
        """Write a progress frame from any thread. Never raises."""
        try:
            write_frame(real_stdout, payload)
        except (ValueError, OSError):
            pass

    def run_call(frame_id, tool_name, tool_args) -> None:
        try:
            result = call(tool_name, tool_args, sessions, cancel_key=frame_id)
            emit({"id": frame_id, "ok": True, "result": verdict.as_typed(result)})
        except registry.Unavailable as exc:
            # A missing dependency is a typed outcome, not a crash: the model sees
            # what to install instead of the whole call failing opaquely.
            emit({
                "id": frame_id,
                "ok": True,
                "result": verdict.typed(verdict.UNAVAILABLE, str(exc)),
            })
        except Exception as exc:
            emit({"id": frame_id, "ok": False, "error": str(exc)})
        finally:
            cancel_mod.release(frame_id)

    def run_install(frame_id, key) -> None:
        try:
            from koma_sec_daemon import installer
            emit({"id": frame_id, "ok": True, "result": installer.install(key)})
        except Exception as exc:
            emit({"id": frame_id, "ok": False, "error": str(exc)})

    try:
        while True:
            try:
                frame = read_frame(sys.stdin)
            except ValueError as exc:
                print(f"[koma_sec_daemon] malformed frame: {exc}", file=sys.stderr)
                write_frame(real_stdout, {"ok": False, "error": f"malformed frame: {exc}"})
                continue

            if frame is None:
                break

            frame_id = frame.get("id")
            op = frame.get("op")

            if op == "call":
                tool_name = frame.get("tool", "")
                tool_args = frame.get("args") or {}
                if tool_name not in registry.REGISTRY:
                    write_frame(real_stdout, {
                        "id": frame_id,
                        "ok": False,
                        "error": f"unknown tool: {tool_name}",
                    })
                    continue
                calls.submit(run_call, frame_id, tool_name, tool_args)

            elif op == "cancel":
                # Kills this id's child work and answers immediately, so the
                # parent is not blocked behind the thing it is cancelling.
                killed = cancel_mod.cancel(frame.get("target"))
                write_frame(real_stdout, {"id": frame_id, "ok": True, "result": str(killed)})

            elif op == "health":
                from koma_sec_daemon import health
                write_frame(real_stdout, {
                    "id": frame_id,
                    "ok": True,
                    "result": json.dumps(health.probe()),
                })

            elif op == "install":
                installs.submit(run_install, frame_id, frame.get("key", ""))

            else:
                write_frame(real_stdout, {"id": frame_id, "ok": False, "error": "unknown op"})
    finally:
        calls.shutdown(wait=False)
        installs.shutdown(wait=False)
        cancel_mod.cancel_all()
        sessions.close_all()


if __name__ == "__main__":
    main()
