"""
sec_rsa — RsaCtfTool wrapper for RSA key/cipher attacks.

compute : long-cpu
risk    : False
domain  : crypto

RsaCtfTool is invoked as a subprocess via sandbox.run so the registry
loads cleanly even when the binary is absent (import-time safety).
"""

from __future__ import annotations

import shlex

DESCRIPTOR = {
    "name": "sec_rsa",
    "description": (
        "Run RsaCtfTool with the given CLI arguments (key files must live "
        "in the daemon working dir)."
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "args": {
                "type": "string",
                "description": (
                    "RsaCtfTool argument string, e.g. "
                    "'--publickey key.pem --uncipher 12345 --attack all'."
                ),
            },
        },
        "required": ["args"],
    },
    "risk": False,
    "compute": "long-cpu",
    "domain": "crypto",
}


def _handler(args: dict, sessions) -> str:
    # Lazy import — keeps registry loadable without sandbox at import time
    from koma_sec_daemon import sandbox  # noqa: PLC0415

    cli_args = args["args"]
    try:
        parts = shlex.split(cli_args)
    except ValueError as exc:
        return f"error: failed to parse args string: {exc}"

    cmd = ["RsaCtfTool"] + parts
    return sandbox.run(cmd, timeout=180, mem_mb=2048)


# Attach handler to descriptor for registry use
DESCRIPTOR["handler"] = _handler
