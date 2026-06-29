"""
sec_crack — hashcat straight wordlist hash-cracking tool.

compute : gpu
risk    : False
domain  : crypto

All hashcat invocation is done through sandbox.run() so the registry loads
cleanly even when hashcat is absent (import-time safety for the smoke test).
"""

from __future__ import annotations

import os
import shlex
import tempfile

DESCRIPTOR = {
    "name": "sec_crack",
    "description": (
        "Crack a hash with hashcat (straight wordlist attack) and return "
        "the cracked plaintext. Requires hashcat installed and a wordlist."
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "hash": {
                "type": "string",
                "description": "The hash string to crack (required).",
            },
            "mode": {
                "type": "integer",
                "description": "Hashcat -m mode number (e.g. 0 for MD5, 1000 for NTLM).",
            },
            "wordlist": {
                "type": "string",
                "description": "Path to the wordlist file (default: /usr/share/wordlists/rockyou.txt).",
                "default": "/usr/share/wordlists/rockyou.txt",
            },
            "extra_args": {
                "type": "string",
                "description": "Optional extra hashcat arguments as a shell-quoted string.",
            },
        },
        "required": ["hash", "mode"],
    },
    "risk": False,
    "compute": "gpu",
    "domain": "crypto",
}


def _handler(args: dict, sessions) -> str:
    from koma_sec_daemon import sandbox  # noqa: PLC0415

    hash_value = args["hash"]
    mode = int(args["mode"])
    wordlist = args.get("wordlist") or "/usr/share/wordlists/rockyou.txt"
    extra_args = args.get("extra_args") or ""

    hp = None
    try:
        with tempfile.NamedTemporaryFile(
            mode="w", suffix=".hash", delete=False
        ) as tf:
            tf.write(hash_value)
            hp = tf.name

        cmd = [
            "hashcat",
            "-m", str(mode),
            "-a", "0",
            hp,
            wordlist,
            "--quiet",
            "--potfile-disable",
            "-o", "-",
        ]
        if extra_args:
            cmd.extend(shlex.split(extra_args))

        out = sandbox.run(cmd, timeout=600)
    finally:
        if hp is not None:
            try:
                os.unlink(hp)
            except OSError:
                pass

    return out


# Attach handler to descriptor for registry use
DESCRIPTOR["handler"] = _handler
