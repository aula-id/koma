"""
sec_ffuf — web fuzzer tool backed by the ffuf binary.

compute : network
risk    : True
domain  : web

No third-party Python packages are required; ffuf is an external binary
invoked through sandbox.run.  shlex is stdlib and may be imported at the
top level.
"""

from __future__ import annotations

import shlex

DESCRIPTOR = {
    "name": "sec_ffuf",
    "description": (
        "Fuzz a URL with ffuf (URL must contain the FUZZ keyword)."
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "url": {
                "type": "string",
                "description": "Target URL containing the FUZZ keyword (required).",
            },
            "wordlist": {
                "type": "string",
                "description": "Path to the wordlist file (default: /usr/share/wordlists/dirb/common.txt).",
                "default": "/usr/share/wordlists/dirb/common.txt",
            },
            "match_codes": {
                "type": "string",
                "description": "Comma-separated HTTP status codes to match (default: 200,204,301,302,307,401,403).",
                "default": "200,204,301,302,307,401,403",
            },
            "extra_args": {
                "type": "string",
                "description": "Optional extra ffuf arguments appended verbatim (shell-split).",
            },
        },
        "required": ["url"],
    },
    "risk": True,
    "compute": "network",
    "domain": "web",
}


def _handler(args: dict, sessions) -> str:
    # Lazy import of sandbox — keeps registry loadable when sandbox has
    # optional deps not yet installed.
    from koma_sec_daemon import sandbox  # noqa: PLC0415

    url = args["url"]

    if "FUZZ" not in url:
        return "error: url must contain the FUZZ keyword"

    wordlist = args.get("wordlist", "/usr/share/wordlists/dirb/common.txt")
    match_codes = args.get("match_codes", "200,204,301,302,307,401,403")
    extra_args = args.get("extra_args", "")

    cmd = ["ffuf", "-u", url, "-w", wordlist, "-mc", match_codes, "-s"]

    if extra_args:
        try:
            cmd.extend(shlex.split(extra_args))
        except ValueError as exc:
            return f"error: failed to parse extra_args: {exc}"

    return sandbox.run(cmd, timeout=300)


# Attach handler to descriptor for registry use
DESCRIPTOR["handler"] = _handler
