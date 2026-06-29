"""
sec_zap — OWASP ZAP REST API driver tool.

compute : network
risk    : True
domain  : web

requests is imported LAZILY inside the handler so the registry loads even
when the package is not installed (import-time safety for the smoke test).

Actions
-------
spider  {target, apikey?, base?}  → start a spider scan on target URL
ascan   {target, apikey?, base?}  → start an active scan on target URL
alerts  {target, apikey?, base?}  → list alerts for target URL
"""

from __future__ import annotations

DESCRIPTOR = {
    "name": "sec_zap",
    "description": (
        "Drive a running OWASP ZAP daemon via its REST API "
        "(spider, active-scan, or list alerts)."
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "action": {
                "type": "string",
                "enum": ["spider", "ascan", "alerts"],
                "description": "Which ZAP operation to perform.",
            },
            "target": {
                "type": "string",
                "description": "Target URL (required for spider/ascan; used as baseurl for alerts).",
            },
            "apikey": {
                "type": "string",
                "description": "ZAP API key (optional, default empty string).",
                "default": "",
            },
            "base": {
                "type": "string",
                "description": "ZAP daemon base URL (optional, default http://127.0.0.1:8080).",
                "default": "http://127.0.0.1:8080",
            },
        },
        "required": ["action", "target"],
    },
    "risk": True,
    "compute": "network",
    "domain": "web",
}


def _handler(args: dict, sessions) -> str:
    # Lazy import — keeps registry loadable without requests installed
    import requests  # noqa: PLC0415

    action = args.get("action")
    target = args.get("target", "")
    apikey = args.get("apikey", "")
    base = args.get("base", "http://127.0.0.1:8080").rstrip("/")

    if action == "spider":
        url = f"{base}/JSON/spider/action/scan/"
        params = {"url": target, "apikey": apikey}
    elif action == "ascan":
        url = f"{base}/JSON/ascan/action/scan/"
        params = {"url": target, "apikey": apikey}
    elif action == "alerts":
        url = f"{base}/JSON/core/view/alerts/"
        params = {"baseurl": target, "apikey": apikey}
    else:
        return f"error: unknown action '{action}'"

    try:
        resp = requests.get(url, params=params, timeout=30)
    except Exception as exc:
        return f"error: ZAP not reachable: {exc}"

    return resp.text


# Attach handler to descriptor for registry use
DESCRIPTOR["handler"] = _handler
