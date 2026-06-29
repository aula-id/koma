"""
sec_dalfox — dalfox XSS scanner tool.

compute : network
risk    : True
domain  : web

dalfox is invoked via sandbox.run() so the registry loads cleanly even when
the binary is absent (import-time safety for the smoke test).
"""

from __future__ import annotations

import shlex

DESCRIPTOR = {
    "name": "sec_dalfox",
    "description": "Run dalfox XSS scanner against a URL.",
    "parameters": {
        "type": "object",
        "properties": {
            "url": {
                "type": "string",
                "description": "Target URL to scan for XSS (required).",
            },
            "data": {
                "type": "string",
                "description": "Optional POST data body passed to dalfox via --data.",
            },
            "extra_args": {
                "type": "string",
                "description": "Optional extra dalfox arguments as a single string (shell-split).",
            },
        },
        "required": ["url"],
    },
    "risk": True,
    "compute": "network",
    "domain": "web",
}


def _handler(args: dict, sessions) -> str:
    # Lazy import — keeps registry loadable when koma_sec_daemon.sandbox is absent
    from koma_sec_daemon import sandbox  # noqa: PLC0415

    url = args["url"]
    data = args.get("data")
    extra_args = args.get("extra_args")

    cmd = ["dalfox", "url", url, "--no-color", "--silence"]

    if data:
        cmd += ["--data", data]

    if extra_args:
        try:
            cmd += shlex.split(extra_args)
        except ValueError as exc:
            return f"error: failed to parse extra_args: {exc}"

    return sandbox.run(cmd, timeout=300)


# Attach handler to descriptor for registry use
DESCRIPTOR["handler"] = _handler
