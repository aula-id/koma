"""
sec_jsdeobf — JavaScript deobfuscation/unpacking tool via webcrack (npx).

compute : instant-cpu
risk    : False
domain  : web-re

webcrack is invoked through npx (no install step required) so no Python
third-party library is needed.  sandbox is imported LAZILY inside the handler
so the registry loads cleanly even when node/npx is absent.
"""

from __future__ import annotations

import os
import tempfile

DESCRIPTOR = {
    "name": "sec_jsdeobf",
    "description": (
        "Deobfuscate/unpack JavaScript using webcrack (via npx). "
        "Accepts obfuscated JS source and returns the cleaned output."
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "code": {
                "type": "string",
                "description": "The obfuscated JavaScript source to deobfuscate.",
            },
        },
        "required": ["code"],
    },
    "risk": False,
    "compute": "instant-cpu",
    "domain": "web-re",
}


def _handler(args: dict, sessions) -> str:
    # Lazy import — keeps registry loadable without sandbox side-effects at import time
    from koma_sec_daemon import sandbox  # noqa: PLC0415

    code = args["code"]

    jp = None
    try:
        # Write obfuscated code to a temp file that webcrack will read
        with tempfile.NamedTemporaryFile(suffix=".js", mode="w", delete=False, encoding="utf-8") as fh:
            jp = fh.name
            fh.write(code)

        output = sandbox.run(["npx", "--yes", "webcrack", jp], timeout=120)
    finally:
        if jp is not None:
            try:
                os.unlink(jp)
            except OSError:
                pass

    return output


# Attach handler to descriptor for registry use
DESCRIPTOR["handler"] = _handler
