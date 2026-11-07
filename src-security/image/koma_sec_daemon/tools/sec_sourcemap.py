"""
sec_sourcemap — JavaScript source-map recovery tool.

compute : instant-cpu
risk    : False
domain  : web-re

Parses the JSON of a .map file and recovers original source files from the
parallel "sources" / "sourcesContent" arrays.  Stdlib json only; no lazy
imports required.
"""

from __future__ import annotations

import json

DESCRIPTOR = {
    "name": "sec_sourcemap",
    "description": (
        "Parse a JavaScript source map and recover the original source files. "
        "Reads the 'sources' and 'sourcesContent' arrays from the .map JSON "
        "and returns either all recovered files (name + content) or a single "
        "entry selected by index."
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "sourcemap": {
                "type": "string",
                "description": "The full JSON content of a .map (source map) file.",
            },
            "index": {
                "type": "integer",
                "description": (
                    "If provided, return only the Nth recovered source "
                    "(0-based). Omit to list all sources."
                ),
            },
        },
        "required": ["sourcemap"],
    },
    "risk": False,
    "compute": "instant-cpu",
    "domain": "web-re",
}


def _handler(args: dict, sessions) -> str:
    raw = args.get("sourcemap", "")
    index = args.get("index", None)

    try:
        data = json.loads(raw)
    except Exception as exc:
        return f"error: failed to parse sourcemap JSON: {exc}"

    if not isinstance(data, dict):
        return "error: sourcemap JSON root must be an object"

    sources = data.get("sources")
    contents = data.get("sourcesContent")

    if not isinstance(sources, list):
        return "error: sourcemap missing 'sources' array"

    # sourcesContent may be absent or partially null-padded
    if not isinstance(contents, list):
        contents = []

    n = len(sources)

    if index is not None:
        # Return a single entry
        if index < 0 or index >= n:
            return f"error: index {index} out of range (sources has {n} entries)"
        name = sources[index]
        content = contents[index] if index < len(contents) else None
        if content is None:
            return f"[{index}] {name}\n(no sourcesContent for this entry)"
        return f"[{index}] {name}\n{content}"

    # Return full listing
    parts: list[str] = []
    for i, name in enumerate(sources):
        content = contents[i] if i < len(contents) else None
        if content is None:
            parts.append(f"[{i}] {name}\n(no sourcesContent)")
        else:
            parts.append(f"[{i}] {name}\n{content}")

    if not parts:
        return "sourcemap contains no sources"

    return "\n\n".join(parts)


# Attach handler to descriptor for registry use
DESCRIPTOR["handler"] = _handler
