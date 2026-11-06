"""
Frame codec for newline-delimited JSON over a binary/text stream.

Frames are single UTF-8 lines: one JSON object per line, terminated by '\n'.
Blank lines are silently ignored.
Malformed JSON raises ValueError — the caller must turn that into an error frame.
"""
from __future__ import annotations

import json


def read_frame(stream) -> dict | None:
    """
    Read one JSON frame from *stream* (stdin or any line-readable object).

    Returns the decoded dict, or None on EOF.
    Blank lines are skipped.
    Raises ValueError on malformed JSON.
    """
    while True:
        line = stream.readline()
        # EOF: readline returns "" (text mode) or b"" (binary mode)
        if line == "" or line == b"":
            return None
        # Normalise bytes to str
        if isinstance(line, (bytes, bytearray)):
            line = line.decode("utf-8", errors="replace")
        line = line.strip()
        if not line:
            continue  # skip blank lines
        return json.loads(line)  # raises ValueError on bad JSON


# Protocol revision. Bump MINOR for additive fields a v1 peer ignores, MAJOR for a
# change an older peer must refuse rather than mis-read. The parent sends its own
# revision in the handshake frame; `negotiate` refuses rather than guessing.
PROTOCOL_VERSION = 2


def negotiate(hello: dict) -> str | None:
    """
    Validate the parent's handshake revision. Return a refusal string, or None when
    the peer is compatible.

    A MAJOR mismatch is refused outright. A MINOR skew is accepted in EITHER
    direction: the parent tolerates a daemon that lacks newer fields, and this
    daemon tolerates a parent that predates them. Only a missing revision is
    refused, so a half-migrated pair cannot silently run with unmigrated
    capability metadata.
    """
    raw = hello.get("v")
    if raw is None:
        return "handshake carried no protocol revision"
    try:
        peer = int(raw)
    except (TypeError, ValueError):
        return f"handshake revision {raw!r} is not an integer"
    mine_major = PROTOCOL_VERSION
    if peer != mine_major:
        return f"protocol major mismatch: parent v{peer}, daemon v{mine_major}"
    return None


def write_frame(stream, obj: dict) -> None:
    """
    Serialise *obj* as a single JSON line and write it to *stream*, flushing
    immediately so the parent process receives each frame as it is produced.
    """
    payload = json.dumps(obj, ensure_ascii=False) + "\n"
    stream.write(payload)
    stream.flush()
