"""
sec_z3 — Z3 constraint-solver script execution tool.

compute : long-cpu
risk    : False
domain  : crypto

sys is imported top-level (stdlib). sandbox is imported LAZILY inside the
handler so the registry loads cleanly even in environments where the daemon
package path is not yet wired up at import time.
"""

from __future__ import annotations

import sys

DESCRIPTOR = {
    "name": "sec_z3",
    "description": (
        "Execute an agent-written Python Z3 (z3-solver) script and return its stdout. "
        "The general constraint-solving escape hatch."
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "code": {
                "type": "string",
                "description": (
                    "A self-contained Python program that imports z3 and prints its result."
                ),
            },
        },
        "required": ["code"],
    },
    "risk": False,
    "compute": "long-cpu",
    "domain": "crypto",
}


def _handler(args: dict, sessions) -> str:
    # Lazy import — keeps registry loadable without the daemon package on sys.path
    from koma_sec_daemon import sandbox  # noqa: PLC0415

    code = args["code"]

    # Run the user-supplied Z3 script under the daemon's own interpreter.
    # z3-solver is a pip dependency of the daemon venv, so the script can
    # "import z3" freely. Memory capped at 2 GiB; timeout 120 s.
    return sandbox.run(
        [sys.executable, "-c", code],
        timeout=120,
        mem_mb=2048,
    )


# Attach handler to descriptor for registry use
DESCRIPTOR["handler"] = _handler
