"""
sec_http — stateless single HTTP request tool.

compute : network
risk    : False
domain  : web

requests is imported LAZILY inside the handler so the registry loads even
when the package is not installed (import-time safety for the smoke test).
"""

from __future__ import annotations

_BODY_CAP = 100_000

DESCRIPTOR = {
    "name": "sec_http",
    "description": (
        "Perform a single HTTP request and return the status line, "
        "response headers, and body (body capped at 100 000 chars). "
        "TLS certificate verification is disabled, so self-signed certs are accepted."
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "url": {
                "type": "string",
                "description": "Target URL (required).",
            },
            "method": {
                "type": "string",
                "description": "HTTP method (default: GET).",
                "default": "GET",
            },
            "headers": {
                "type": "object",
                "description": "Optional request headers as key/value pairs.",
                "additionalProperties": {"type": "string"},
            },
            "body": {
                "type": "string",
                "description": "Optional request body string.",
            },
            "follow_redirects": {
                "type": "boolean",
                "description": "Follow HTTP redirects (default: true).",
                "default": True,
            },
        },
        "required": ["url"],
    },
    "risk": False,
    "compute": "network",
    "domain": "web",
}


def _handler(args: dict, sessions) -> str:
    # Lazy import — keeps registry loadable without requests installed
    import requests  # noqa: PLC0415
    import urllib3  # noqa: PLC0415

    # Pentest targets (HTB/CTF boxes) are routinely self-signed, so TLS cert
    # verification is intentionally disabled — accept any certificate. Silence the
    # resulting urllib3 InsecureRequestWarning so it doesn't pollute the response.
    urllib3.disable_warnings(urllib3.exceptions.InsecureRequestWarning)

    url = args["url"]
    method = args.get("method", "GET").upper()
    headers = args.get("headers") or {}
    body = args.get("body", None)
    follow_redirects = args.get("follow_redirects", True)

    try:
        resp = requests.request(
            method,
            url,
            headers=headers,
            data=body.encode() if body else None,
            allow_redirects=follow_redirects,
            timeout=30,
            verify=False,
        )
    except Exception as exc:
        return f"error: {exc}"

    # Build status line
    status_line = f"HTTP {resp.status_code} {resp.reason}"

    # Format response headers
    resp_headers = "\n".join(f"{k}: {v}" for k, v in resp.headers.items())

    # Cap body
    body_text = resp.text
    if len(body_text) > _BODY_CAP:
        body_text = body_text[:_BODY_CAP] + "\n…[body truncated]"

    return f"{status_line}\n{resp_headers}\n\n{body_text}"


# Attach handler to descriptor for registry use
DESCRIPTOR["handler"] = _handler
