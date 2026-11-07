"""
sec_recording — save and load a recording of requests, for replay and for tests.

compute : instant-cpu
risk    : False
domain  : web

A recording is the fixed baseline everything else compares against: `sec_replay`
re-sends it, and a blue detector regression runs the same recording against a patched
target. So the format is deliberately boring JSON, and the file is plain on disk —
a recording that can only be read by this tool is not evidence.

Local only. Paths go through the daemon's own traversal guard so a recording
cannot land outside the artifacts dir.

Two ops, so one tool rather than two:
  - `save`  from a list of raw HTTP requests (usually produced by a proxy export)
  - `load`  returns the recording, parsed and summarised
"""

from __future__ import annotations

import json
import os
import re
from pathlib import Path

MAX_ENTRIES = 500
MAX_BODY = 256 * 1024
NAME_RE = re.compile(r"^[A-Za-z0-9._-]{1,80}$")


def _spawn():
    from koma_sec_daemon import registry

    return registry.spawn_for_thread()


DESCRIPTOR = {
    "name": "sec_recording",
    "description": (
        "Save or load a recording of HTTP requests. The recording is the fixed "
        "baseline for replay and for regression: save once, replay many times."
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "op": {
                "type": "string",
                "enum": ["save", "load"],
                "description": "Save a new recording or load an existing one.",
            },
            "name": {
                "type": "string",
                "description": "Recording name (letters, digits, dot, dash, underscore).",
            },
            "requests": {
                "type": "array",
                "description": (
                    "save only: entries as raw request text, or as "
                    "{method, url, headers{}, body} objects."
                ),
                "items": {},
            },
        },
        "required": ["op", "name"],
    },
    "risk": False,
    "compute": "instant-cpu",
    "domain": "web",
}


def _dir(sessions) -> Path:
    """Recordings dir, created. Sibling of evidence, not inside it."""
    root = Path(os.environ.get("KOMA_BOARD") or "/tmp/koma-board")
    d = root / "recording"
    d.mkdir(parents=True, exist_ok=True)
    return d


def _path(sessions, name: str) -> Path:
    if not NAME_RE.match(name):
        raise ValueError(f"bad recording name {name!r} (letters, digits, . _ - only)")
    return _dir(sessions) / f"{name}.json"


def _entry(raw) -> dict:
    """Normalise one entry to `{method, url, headers, body}`."""
    if isinstance(raw, str):
        head, _, body = raw.partition("\n\n")
        lines = head.splitlines()
        if not lines:
            raise ValueError("empty request")
        parts = lines[0].split()
        if len(parts) < 2:
            raise ValueError(f"malformed request line {lines[0]!r}")
        headers = {}
        for line in lines[1:]:
            k, _, v = line.partition(":")
            if k.strip():
                headers[k.strip()] = v.strip()
        return {"method": parts[0], "url": parts[1], "headers": headers, "body": body}
    if isinstance(raw, dict):
        if "url" not in raw:
            raise ValueError("object entry has no url")
        return {
            "method": str(raw.get("method") or "GET"),
            "url": str(raw["url"]),
            "headers": dict(raw.get("headers") or {}),
            "body": str(raw.get("body") or ""),
        }
    raise ValueError(f"unsupported entry type {type(raw).__name__}")


def _handler(args: dict, sessions) -> str:
    op = args["op"]
    name = args["name"]
    # A traversal attempt is the caller's input, not a crash.
    try:
        path = _path(sessions, name)
    except ValueError as exc:
        return json.dumps({"verdict": "error", "excerpt": str(exc)})

    if op == "load":
        if not path.exists():
            return json.dumps(
                {"verdict": "not_found", "excerpt": f"no recording named {name!r}"}
            )
        try:
            doc = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, ValueError) as exc:
            return json.dumps({"verdict": "error", "excerpt": f"unreadable: {exc}"})
        entries = doc.get("entries") or []
        summary = "\n".join(
            f"{i}: {e.get('method')} {e.get('url')}" for i, e in enumerate(entries[:50])
        )
        return json.dumps(
            {
                "verdict": "confirmed" if entries else "unproven",
                "excerpt": f"{len(entries)} entries in {path}\n\n{summary}",
            }
        )

    raw_entries = args.get("requests") or []
    if not raw_entries:
        return json.dumps({"verdict": "error", "excerpt": "save needs at least one request"})
    if len(raw_entries) > MAX_ENTRIES:
        return json.dumps(
            {"verdict": "error", "excerpt": f"{len(raw_entries)} entries; cap is {MAX_ENTRIES}"}
        )

    entries, rejected = [], []
    for i, raw in enumerate(raw_entries):
        try:
            e = _entry(raw)
        except ValueError as exc:
            rejected.append(f"{i}: {exc}")
            continue
        if len(e["body"]) > MAX_BODY:
            e["body"] = e["body"][:MAX_BODY]
            e["truncated"] = True
        entries.append(e)

    if not entries:
        return json.dumps(
            {"verdict": "error", "excerpt": "no valid entries:\n" + "\n".join(rejected)}
        )

    # A recording carries the ORDER it was captured in; replay depends on it, so a
    # dict is not enough to preserve it and the list is kept as a list.
    path.write_text(
        json.dumps({"name": name, "entries": entries}, indent=1), encoding="utf-8"
    )
    verdict = "confirmed" if not rejected else "unproven"
    excerpt = f"saved {len(entries)} entries to {path}"
    if rejected:
        excerpt += f"\n{len(rejected)} rejected:\n" + "\n".join(rejected[:10])
    return json.dumps({"verdict": verdict, "excerpt": excerpt})


def handler(args: dict, sessions) -> str:
    return _handler(args, sessions)
