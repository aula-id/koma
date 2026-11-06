"""
sec_xss — probe a URL for reflected XSS with a unique marker.

compute : network
risk    : True
domain  : web

The probe is deliberately narrow and the verdict is deliberately strict, because
per `21` a version match is not a confirmation:

  - A marker that comes back **verbatim** proves reflection, so the verdict is
    `confirmed` — but only when it comes back unescaped. A marker returned as
    `&lt;b&gt;` is reflected and inert, which is `proven-reflected, not executable`.
  - A marker that comes back **encoded** is `not_found`: something reflected it, but
    nothing a browser would run.
  - No marker at all is `not_found` too — the parameter may be ignored entirely.

That distinction is what makes this usable as the blue regression: replay against
the patched target must stop echoing the raw marker while the clean baseline never
echoed it in the first place. A detector that only matched on "did the marker come
back at all" would pass against a fix that merely encoded it — which is the exact
failure mode the plan names.

Goes through the run's egress proxy like every other network tool.
"""

from __future__ import annotations

import json
import os
import re
import secrets

TIMEOUT = 20
BODY_CAP = 200_000


def _spawn():
    from koma_sec_daemon import registry

    return registry.spawn_for_thread()


DESCRIPTOR = {
    "name": "sec_xss",
    "description": (
        "Probe a URL for reflected XSS. Returns `confirmed` when the injected marker "
        "comes back verbatim (would execute), `not_found` when it is absent or "
        "encoded, so a detector can tell a real fix from an escaping fix."
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "url": {
                "type": "string",
                "description": "URL to probe, with or without the query string.",
            },
            "param": {
                "type": "string",
                "description": (
                    "Query parameter to inject into. Appended when absent from the URL."
                ),
            },
            "payload": {
                "type": "string",
                "description": (
                    "Injection template; `{m}` is replaced by a unique marker. "
                    "Default `<b id=\"{m}\">koma</b>`."
                ),
            },
        },
        "required": ["url", "param"],
    },
    "risk": True,
    "compute": "network",
    "domain": "web",
    "network_param": "url",
}


def _inject(url: str, param: str, payload: str) -> str:
    from urllib.parse import parse_qsl, urlencode, urlsplit, urlunsplit  # noqa: PLC0415

    parts = urlsplit(url)
    q = dict(parse_qsl(parts.query, keep_blank_values=True))
    q[param] = payload
    return urlunsplit((parts.scheme, parts.netloc, parts.path or "/", urlencode(q), ""))


def _handler(args: dict, sessions) -> str:
    import requests  # noqa: PLC0415

    marker = f"kx{secrets.token_hex(4)}"
    template = args.get("payload") or '<b id="{m}">koma</b>'
    payload = template.replace("{m}", marker)
    url = _inject(args["url"], args["param"], payload)

    proxy = os.environ.get("HTTPS_PROXY") or os.environ.get("HTTP_PROXY")
    try:
        resp = requests.get(
            url,
            timeout=TIMEOUT,
            proxies={"http": proxy, "https": proxy} if proxy else None,
            allow_redirects=True,
        )
    except requests.RequestException as exc:
        return json.dumps({"verdict": "unavailable", "excerpt": f"{url}: {exc}"})

    if resp.headers.get("X-Koma-Egress"):
        return json.dumps(
            {
                "verdict": "blocked",
                "excerpt": f"egress refused: {resp.headers['X-Koma-Egress']}",
            }
        )

    body = resp.text[:BODY_CAP]
    raw = payload in body
    # The marker alone is not enough — only the RAW payload proves it would run.
    bare_marker = marker in body
    escaped_marker = re.search(
        rf"&lt;b id=(?:&quot;|&#34;|\\\")?{marker}", body, re.IGNORECASE
    )

    if raw:
        verdict = "confirmed"
        note = "payload reflected VERBATIM — would execute"
    elif escaped_marker:
        verdict = "not_found"
        note = "marker came back HTML-encoded — reflected but inert"
    elif bare_marker:
        verdict = "not_found"
        note = "marker present but the payload was rewritten (partial filter?)"
    else:
        verdict = "not_found"
        note = "marker absent — parameter ignored or stripped"

    excerpt = (
        f"{note}\n{url}\nHTTP {resp.status_code}, {len(resp.content)} bytes"
    )
    return json.dumps({"verdict": verdict, "excerpt": excerpt})


def handler(args: dict, sessions) -> str:
    return _handler(args, sessions)
