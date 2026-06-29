"""
sec_rop — ROP gadget search tool using ROPgadget.

compute : instant-cpu
risk    : False
domain  : pwn

ROPgadget is invoked via sandbox.run() so the registry loads cleanly even
when the binary is not present (import-time safety for the smoke test).
"""

from __future__ import annotations

DESCRIPTOR = {
    "name": "sec_rop",
    "description": (
        "Search ROP gadgets in a binary with ROPgadget (optionally filtered by regex)."
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "binary": {
                "type": "string",
                "description": "Path to the target binary to scan for ROP gadgets (required).",
            },
            "pattern": {
                "type": "string",
                "description": "Optional regex pattern passed to --re to filter gadgets.",
            },
        },
        "required": ["binary"],
    },
    "risk": False,
    "compute": "instant-cpu",
    "domain": "pwn",
}


def _handler(args: dict, sessions) -> str:
    # Lazy import — keeps registry loadable without sandbox available at import time
    from koma_sec_daemon import sandbox  # noqa: PLC0415

    binary = args["binary"]
    pattern = args.get("pattern")

    cmd = ["ROPgadget", "--binary", binary]
    if pattern:
        cmd += ["--re", pattern]

    return sandbox.run(cmd, timeout=120)


# Attach handler to descriptor for registry use
DESCRIPTOR["handler"] = _handler
