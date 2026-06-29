"""
sec_nuclei — nuclei template-based vulnerability scanner tool.

compute : network
risk    : True
domain  : web

No third-party Python imports needed; shells out to the `nuclei` binary via
sandbox.run(). All imports are stdlib or internal.
"""

from __future__ import annotations

import shlex

DESCRIPTOR = {
    "name": "sec_nuclei",
    "description": (
        "Run nuclei template-based vulnerability scan against a URL. "
        "Requires the `nuclei` binary to be installed and available in PATH."
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "url": {
                "type": "string",
                "description": "Target URL to scan (required).",
            },
            "severity": {
                "type": "string",
                "description": (
                    "Comma-separated severity filter passed to -severity "
                    "(e.g. 'critical,high,medium')."
                ),
            },
            "tags": {
                "type": "string",
                "description": (
                    "Comma-separated tag filter passed to -tags "
                    "(e.g. 'cve,sqli,xss')."
                ),
            },
            "templates": {
                "type": "string",
                "description": (
                    "Path to a nuclei template or templates directory "
                    "passed to -t."
                ),
            },
            "extra_args": {
                "type": "string",
                "description": (
                    "Additional raw nuclei arguments as a shell-quoted string "
                    "(split via shlex)."
                ),
            },
        },
        "required": ["url"],
    },
    "risk": True,
    "compute": "network",
    "domain": "web",
}


def _handler(args: dict, sessions) -> str:
    from koma_sec_daemon import sandbox  # noqa: PLC0415

    url = args["url"]

    cmd = ["nuclei", "-u", url, "-silent", "-nc"]

    severity = args.get("severity")
    if severity:
        cmd += ["-severity", severity]

    tags = args.get("tags")
    if tags:
        cmd += ["-tags", tags]

    templates = args.get("templates")
    if templates:
        cmd += ["-t", templates]

    extra_args = args.get("extra_args")
    if extra_args:
        try:
            cmd += shlex.split(extra_args)
        except ValueError as exc:
            return f"error: failed to parse extra_args: {exc}"

    return sandbox.run(cmd, timeout=300)


# Attach handler to descriptor for registry use
DESCRIPTOR["handler"] = _handler
