"""One-shot tool invocation intended for `docker exec` inside the guest image.

Usage:
  python -m koma_sec_daemon.guest_run <tool> '<json-args>'
Prints the result string on stdout; non-zero exit on failure.
"""
from __future__ import annotations

import json
import sys

from koma_sec_daemon import registry
from koma_sec_daemon.sessions import SessionStore


def main(argv: list[str] | None = None) -> int:
    argv = list(sys.argv[1:] if argv is None else argv)
    if len(argv) < 1:
        print("usage: guest_run <tool> [json-args]", file=sys.stderr)
        return 2
    tool = argv[0]
    raw_args = argv[1] if len(argv) > 1 else "{}"
    try:
        args = json.loads(raw_args) if raw_args else {}
    except json.JSONDecodeError as exc:
        print(f"bad args json: {exc}", file=sys.stderr)
        return 2
    if not isinstance(args, dict):
        print("args must be a JSON object", file=sys.stderr)
        return 2
    sessions = SessionStore()
    try:
        result = registry.call(tool, args, sessions)
        sys.stdout.write(result if isinstance(result, str) else str(result))
        return 0
    except registry.Unavailable as exc:
        # Mirror daemon typed unavailable as plain text; parent wraps verdict.
        sys.stdout.write(str(exc))
        return 0
    except KeyError:
        # Pack-only / generic CLI path (registry.call already tries packs; this
        # is a last-chance direct resolve when call raised before the fallback).
        try:
            from koma_sec_daemon import generic_cli
            from koma_sec_daemon.registry import _generic_from_pack

            generic = _generic_from_pack(tool, args)
            if generic is not None:
                sys.stdout.write(generic)
                return 0
        except Exception as exc:  # noqa: BLE001
            print(str(exc), file=sys.stderr)
            return 1
        print(f"unknown tool: {tool}", file=sys.stderr)
        return 1
    except Exception as exc:  # noqa: BLE001
        print(str(exc), file=sys.stderr)
        return 1
    finally:
        sessions.close_all()


if __name__ == "__main__":
    raise SystemExit(main())
