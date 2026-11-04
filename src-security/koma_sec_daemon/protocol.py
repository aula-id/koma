"""Frame codec: one JSON object per line, terminated by '\\n'."""
from __future__ import annotations

import json


def read_frame(stream) -> dict | None:
    """Return the next JSON object, or None on EOF. Raises ValueError on bad JSON."""
    while True:
        line = stream.readline()
        if line == "" or line == b"":
            return None
        if isinstance(line, (bytes, bytearray)):
            line = line.decode("utf-8", errors="replace")
        line = line.strip()
        if not line:
            continue
        parsed = json.loads(line)
        if not isinstance(parsed, dict):
            raise ValueError("frame was not a JSON object")
        return parsed


def write_frame(stream, obj: dict) -> None:
    """Write one JSON line and flush so the parent sees it immediately."""
    stream.write(json.dumps(obj, ensure_ascii=False) + "\n")
    stream.flush()
