"""
sec_pwntmpl — generate a pwntools exploit skeleton via "pwn template".

compute : instant-cpu
risk    : False
domain  : pwn

All work is done by the pwntools CLI binary (pwn template), invoked via
sandbox.run().  No third-party Python imports needed at the top level.
"""

from __future__ import annotations

import shlex

DESCRIPTOR = {
    "name": "sec_pwntmpl",
    "description": (
        "Generate a pwntools exploit skeleton via 'pwn template'. "
        "Optionally point it at a local binary or a remote host/port."
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "binary": {
                "type": "string",
                "description": "Path to a local binary to target (optional).",
            },
            "host": {
                "type": "string",
                "description": "Remote hostname or IP address (optional).",
            },
            "port": {
                "type": "integer",
                "description": "Remote TCP port (optional, used with host).",
            },
        },
        "required": [],
    },
    "risk": False,
    "compute": "instant-cpu",
    "domain": "pwn",
}


def _handler(args: dict, sessions) -> str:
    from koma_sec_daemon import sandbox  # noqa: PLC0415

    binary = args.get("binary")
    host = args.get("host")
    port = args.get("port")

    cmd = ["pwn", "template"]
    if binary:
        cmd.append(binary)
    if host:
        cmd += ["--host", host]
    if port is not None:
        cmd += ["--port", str(port)]

    return sandbox.run(cmd, timeout=30)


# Attach handler to descriptor for registry use
DESCRIPTOR["handler"] = _handler
