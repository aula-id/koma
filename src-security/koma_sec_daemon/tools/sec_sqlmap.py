"""
sec_sqlmap — run sqlmap against a URL to detect/exploit SQL injection (batch mode).

compute : executes-target
risk    : True
domain  : web

sqlmap is invoked via sandbox.run() so the registry loads cleanly even when
the binary is absent (import-time safety for the smoke test).
"""

from __future__ import annotations

import shlex

DESCRIPTOR = {
    "name": "sec_sqlmap",
    "description": (
        "Run sqlmap against a URL to detect/exploit SQL injection (batch mode). "
        "Returns combined stdout+stderr from the sqlmap process."
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "url": {
                "type": "string",
                "description": "Target URL to test for SQL injection (required).",
            },
            "data": {
                "type": "string",
                "description": "POST body data string (passed as --data).",
            },
            "level": {
                "type": "integer",
                "description": "Test level 1-5 (passed as --level).",
                "minimum": 1,
                "maximum": 5,
            },
            "risk_level": {
                "type": "integer",
                "description": "Risk level 1-3 (passed as --risk).",
                "minimum": 1,
                "maximum": 3,
            },
            "technique": {
                "type": "string",
                "description": (
                    "SQL injection technique letters to use, e.g. 'BEUSTQ' "
                    "(passed as --technique)."
                ),
            },
            "extra_args": {
                "type": "string",
                "description": (
                    "Additional sqlmap arguments as a shell-quoted string "
                    "(shlex-split and appended to the command)."
                ),
            },
        },
        "required": ["url"],
    },
    "risk": True,
    "compute": "executes-target",
    "domain": "web",
}


def _handler(args: dict, sessions) -> str:
    # Lazy import — keeps registry loadable without sqlmap on PATH
    from koma_sec_daemon import sandbox  # noqa: PLC0415

    url = args["url"]

    cmd = ["sqlmap", "-u", url, "--batch", "--disable-coloring"]

    data = args.get("data")
    if data is not None:
        cmd += ["--data", data]

    level = args.get("level")
    if level is not None:
        cmd += ["--level", str(int(level))]

    risk_level = args.get("risk_level")
    if risk_level is not None:
        cmd += ["--risk", str(int(risk_level))]

    technique = args.get("technique")
    if technique is not None:
        cmd += ["--technique", technique]

    extra_args = args.get("extra_args")
    if extra_args:
        try:
            cmd += shlex.split(extra_args)
        except ValueError as exc:
            return f"error: could not parse extra_args: {exc}"

    return sandbox.run(cmd, timeout=300, mem_mb=2048)


# Attach handler to descriptor for registry use
DESCRIPTOR["handler"] = _handler
