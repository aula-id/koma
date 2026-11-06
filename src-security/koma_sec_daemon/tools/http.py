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
    from koma_sec_daemon import verdict  # noqa: PLC0415

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
        # A transport failure is not a "not found" — keep them distinct so a
        # report can tell "the target said no" from "we never arrived".
        return verdict.typed(verdict.ERROR, f"{type(exc).__name__}: {exc}")

    # Build status line
    status_line = f"HTTP {resp.status_code} {resp.reason}"

    # Format response headers
    resp_headers = "\n".join(f"{k}: {v}" for k, v in resp.headers.items())

    # Cap body
    body_text = resp.text
    if len(body_text) > _BODY_CAP:
        body_text = body_text[:_BODY_CAP] + "\n…[body truncated]"

    # A redirect is where the boundary could silently widen: the proxy re-checks
    # every CONNECT, so this only has to SHOW the chain for the record. A hop that
    # was refused while following appears in `history` as an error rather than
    # being swallowed.
    hops = [f"{h.status_code} {h.url}" for h in resp.history]
    if follow_redirects and hops:
        status_line += "\nredirect chain:\n  " + "\n  ".join(hops)
        out_of_scope = [h.url for h in resp.history if h.status_code == 403]
        if out_of_scope:
            status_line += "\n  refused by egress: " + ", ".join(out_of_scope)

    # Confirmed only for a real answer; a 5xx is the target failing, not a finding.
    kind = verdict.CONFIRMED if resp.status_code < 500 else verdict.UNPROVEN
    return verdict.typed(kind, f"{status_line}\n{resp_headers}\n\n{body_text}")


# Attach handler to descriptor for registry use
DESCRIPTOR["handler"] = _handler
