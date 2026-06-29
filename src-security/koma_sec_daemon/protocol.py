"""
Frame codec for newline-delimited JSON over a binary/text stream.

Frames are single UTF-8 lines: one JSON object per line, terminated by '\n'.
Blank lines are silently ignored.
Malformed JSON raises ValueError — the caller must turn that into an error frame.
"""

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


def write_frame(stream, obj: dict) -> None:
    """
    Serialise *obj* as a single JSON line and write it to *stream*, flushing
    immediately so the parent process receives each frame as it is produced.
    """
    payload = json.dumps(obj, ensure_ascii=False) + "\n"
    stream.write(payload)
    stream.flush()
