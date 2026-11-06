"""
sec_unmin — JavaScript beautifier / un-minifier tool.

compute : instant-cpu
risk    : False
domain  : web-re

jsbeautifier is imported LAZILY inside the handler so the registry loads even
when the package is not installed (import-time safety for the smoke test).
"""

from __future__ import annotations

DESCRIPTOR = {
    "name": "sec_unmin",
    "description": "Beautify/un-minify JavaScript using jsbeautifier.",
    "parameters": {
        "type": "object",
        "properties": {
            "code": {
                "type": "string",
                "description": "Minified JavaScript source to beautify (required).",
            },
        },
        "required": ["code"],
    },
    "risk": False,
    "compute": "instant-cpu",
    "domain": "web-re",
}


def _handler(args: dict, sessions) -> str:
    # Lazy import — keeps registry loadable without jsbeautifier installed
    try:
        import jsbeautifier  # noqa: PLC0415
    except ImportError:
        return "error: jsbeautifier not installed"

    return jsbeautifier.beautify(args["code"])


# Attach handler to descriptor for registry use
DESCRIPTOR["handler"] = _handler
