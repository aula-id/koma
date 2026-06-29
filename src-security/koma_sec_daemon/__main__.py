"""
koma_sec_daemon — long-lived security daemon entrypoint.

Protocol (newline-delimited JSON, parent speaks first):
  Handshake (parent → daemon):  {"v": 1, "token": "<T>"}   (token optional)
  Handshake (daemon → parent):  {"ok": true, "tools": [...descriptors...]}
                            or  {"ok": false, "error": "bad token"}

  Call (parent → daemon):       {"id": N, "op": "call", "tool": T, "args": {...}}
  Result (daemon → parent):     {"id": N, "ok": true, "result": "<str>"}
                            or  {"id": N, "ok": false, "error": "<str>"}

  EOF from parent → daemon exits cleanly, closing all sessions.

stdout is the frame channel (binary wire); all library/debug output goes to
stderr (same pattern as scrapion_agent/__main__.py).
"""

import sys
import argparse
import io


def main() -> None:
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
    from koma_sec_daemon.registry import descriptors, call
    from koma_sec_daemon.sessions import SessionStore

    sessions = SessionStore()

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

    write_frame(real_stdout, {"ok": True, "tools": descriptors()})

    # --- MAIN LOOP ---
    # Processes one frame at a time (single-threaded serialization): concurrent
    # Rust-side calls all queue up here, so a slow tool (e.g. a blocking sec_http
    # network request) will delay every other pending call for its full duration.
    # A future concurrency upgrade would use asyncio tasks or a ThreadPoolExecutor
    # to run handlers in parallel while keeping the I/O loop single-threaded.
    try:
        while True:
            try:
                frame = read_frame(sys.stdin)
            except ValueError as exc:
                # Malformed JSON — log to stderr, write error without id, continue
                print(f"[koma_sec_daemon] malformed frame: {exc}", file=sys.stderr)
                write_frame(real_stdout, {"ok": False, "error": f"malformed frame: {exc}"})
                continue

            if frame is None:
                # EOF — parent closed the pipe; exit cleanly
                break

            frame_id = frame.get("id")
            op = frame.get("op")

            if op != "call":
                write_frame(real_stdout, {"id": frame_id, "ok": False, "error": "unknown op"})
                continue

            tool_name = frame.get("tool", "")
            tool_args = frame.get("args") or {}

            # Check for unknown tool BEFORE calling so a handler's internal
            # KeyError (e.g. missing required arg) is never mis-reported as
            # "unknown tool".  All handler exceptions fall through to the
            # generic except below and produce a meaningful error string.
            if tool_name not in registry.REGISTRY:
                write_frame(real_stdout, {
                    "id": frame_id,
                    "ok": False,
                    "error": f"unknown tool: {tool_name}",
                })
                continue

            try:
                result = call(tool_name, tool_args, sessions)
                write_frame(real_stdout, {"id": frame_id, "ok": True, "result": result})
            except Exception as exc:
                write_frame(real_stdout, {
                    "id": frame_id,
                    "ok": False,
                    "error": str(exc),
                })
    finally:
        sessions.close_all()


if __name__ == "__main__":
    main()
