"""
koma_sec_daemon — empty process entrypoint.

Protocol (newline-delimited JSON, parent speaks first):
  Handshake (parent → daemon):  {"v": 1, "token": "<T>"}
  Handshake (daemon → parent):  {"ok": true, "tools": []}
                            or  {"ok": false, "error": "bad token"}

  Call / health / install are answered so the parent never hangs, but this
  process advertises and runs no tools.

  EOF from parent → daemon exits.
"""
from __future__ import annotations

import argparse
import io
import sys

from koma_sec_daemon.protocol import read_frame, write_frame


def main() -> None:
    parser = argparse.ArgumentParser(
        description="koma security daemon (empty) — newline-delimited JSON on stdin/stdout",
        prog="python -m koma_sec_daemon",
    )
    parser.add_argument(
        "--token",
        default=None,
        help="Handshake token minted by koma; if provided the daemon verifies it.",
    )
    args = parser.parse_args()
    expected_token: str | None = args.token

    if hasattr(sys.stdout, "buffer"):
        real_stdout = io.TextIOWrapper(
            sys.stdout.buffer, encoding="utf-8", line_buffering=True
        )
    else:
        real_stdout = sys.stdout
    # Keep the frame channel clean. Library output goes to stderr.
    sys.stdout = sys.stderr

    try:
        frame = read_frame(sys.stdin)
    except ValueError as exc:
        write_frame(real_stdout, {"ok": False, "error": f"bad handshake frame: {exc}"})
        sys.exit(1)

    if frame is None:
        sys.exit(0)

    if expected_token is not None and frame.get("token") != expected_token:
        write_frame(real_stdout, {"ok": False, "error": "bad token"})
        sys.exit(1)

    write_frame(real_stdout, {"ok": True, "tools": []})

    while True:
        try:
            frame = read_frame(sys.stdin)
        except ValueError as exc:
            write_frame(real_stdout, {"ok": False, "error": f"malformed frame: {exc}"})
            continue

        if frame is None:
            break

        frame_id = frame.get("id")
        op = frame.get("op")

        if op == "health":
            # Rust parses `result` as a JSON array of install-health entries.
            write_frame(real_stdout, {"id": frame_id, "ok": True, "result": "[]"})
        elif op == "install":
            write_frame(
                real_stdout,
                {"id": frame_id, "ok": True, "result": "no dependencies"},
            )
        elif op == "call":
            tool = frame.get("tool") or ""
            write_frame(
                real_stdout,
                {
                    "id": frame_id,
                    "ok": False,
                    "error": f"unknown tool: {tool}",
                },
            )
        else:
            write_frame(
                real_stdout,
                {"id": frame_id, "ok": False, "error": "unknown op"},
            )


if __name__ == "__main__":
    main()
