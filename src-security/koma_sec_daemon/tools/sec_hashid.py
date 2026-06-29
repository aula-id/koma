"""
sec_hashid — identify the likely type(s) of a hash string.

compute : instant-cpu
risk    : False
domain  : crypto

No third-party imports at module level — handler uses sandbox.run() so the
registry loads cleanly even when hashid / nth binaries are absent.
"""

from __future__ import annotations

DESCRIPTOR = {
    "name": "sec_hashid",
    "description": "Identify the likely type(s) of a hash string.",
    "parameters": {
        "type": "object",
        "properties": {
            "hash": {
                "type": "string",
                "description": "The hash string to identify (required).",
            },
        },
        "required": ["hash"],
    },
    "risk": False,
    "compute": "instant-cpu",
    "domain": "crypto",
}


def _handler(args: dict, sessions) -> str:
    from koma_sec_daemon import sandbox  # noqa: PLC0415

    hash_val = args["hash"]

    # Primary: hashid with -m flag (Hashcat mode output)
    primary = sandbox.run(["hashid", "-m", hash_val], timeout=20)

    # Check whether hashid produced real output vs a launch error
    # sandbox.run returns "[error launching process: ...]" or "[timed out ...]" on failure
    if primary and not primary.startswith("["):
        return primary

    # Fallback: nth (Name-That-Hash)
    fallback = sandbox.run(["nth", "-t", hash_val, "-a"], timeout=20)

    if fallback and not fallback.startswith("["):
        return fallback

    # Both failed — return whichever error string we got (prefer the primary)
    return primary or fallback or "error: hashid and nth both unavailable"


# Attach handler to descriptor for registry use
DESCRIPTOR["handler"] = _handler
