"""
sec_sage — execute a SageMath snippet via the sage CLI.

compute : long-cpu
risk    : False
domain  : crypto

The sage binary is invoked LAZILY inside the handler (via sandbox.run) so the
registry loads cleanly even when SageMath is not installed.
"""

from __future__ import annotations

DESCRIPTOR = {
    "name": "sec_sage",
    "description": (
        "Execute an agent-written SageMath snippet via the sage CLI and return its output."
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "code": {
                "type": "string",
                "description": "Sage source code to execute.",
            },
        },
        "required": ["code"],
    },
    "risk": False,
    "compute": "long-cpu",
    "domain": "crypto",
}


def _handler(args: dict, sessions) -> str:
    # Lazy import — keeps registry loadable without koma_sec_daemon.sandbox issues
    from koma_sec_daemon import sandbox  # noqa: PLC0415

    code = args["code"]
    return sandbox.run(["sage", "-c", code], timeout=180, mem_mb=4096)


# Attach handler to descriptor for registry use
DESCRIPTOR["handler"] = _handler
