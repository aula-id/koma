"""
sec_browser — drive a real browser for the things HTTP cannot do.

The guest image has no browser: playwright pulls a ~300 MB driver bundle that would
dominate every run even when a target is pure HTTP. So this tool runs on the
HOST, against a proxy pointed at the guest's egress, and says so in its verdict —
`browser` is a distinct exec class precisely so the distinction is visible rather
than assumed.

Consequence that matters: the proxy is what keeps a browser call inside the scope,
not the container. Every navigation is admitted against the same record, and a
redirect that leaves scope is caught before the follow. A browser tool without
that check would be a hole straight past the boundary.

A browser is expensive, so one is kept per run and reused; it is disposed when the
run ends. Nothing here retries a flaky target silently — a failure is reported with
what stage it reached.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys

_PAGE_CAP = 60_000
DEFAULT_TIMEOUT = 45

# Inline driver: it runs under the daemon's interpreter, so the tool stays one
# frame with no extra files, and add `-m playwright` installs work unchanged.
_DRIVER = r"""
import json, sys
from playwright.sync_api import sync_playwright

spec = json.load(sys.stdin)
with sync_playwright() as p:
    browser = p.chromium.launch(
        proxy={"server": spec["proxy"]} if spec.get("proxy") else None,
        args=["--no-sandbox"],
    )
    ctx = browser.new_context(ignore_https_errors=True)
    page = ctx.new_page()
    try:
        resp = page.goto(spec["url"], wait_until=spec.get("wait_until", "load"),
                         timeout=spec["timeout"] * 1000)
        if spec.get("settle_ms"):
            page.wait_for_timeout(spec["settle_ms"])
        out = {
            "status": resp.status if resp else None,
            "final_url": page.url,
            "title": page.title(),
            "text": page.inner_text("body")[: spec["cap"]],
            "links": page.eval_on_selector_all(
                "a[href]", "els => els.map(e => e.href).slice(0, 200)"
            ),
        }
        if spec.get("screenshot"):
            page.screenshot(path=spec["screenshot"], full_page=True)
            out["screenshot"] = spec["screenshot"]
        print(json.dumps({"ok": True, **out}))
    except Exception as exc:
        print(json.dumps({"ok": False, "stage": "navigate", "error": f"{type(exc).__name__}: {exc}"}))
    finally:
        browser.close()
"""

DESCRIPTOR = {
    "name": "sec_browser",
    "description": (
        "Load a page in a headless browser and return status, final URL, title, "
        "body text, and links. Runs on the host (the guest image has no browser) "
        "against the run's egress proxy, so scope still applies. Use it when a "
        "target needs JavaScript or a redirect chain HTTP cannot follow."
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "url": {"type": "string", "description": "Target URL (required)."},
            "wait_until": {
                "type": "string",
                "enum": ["load", "domcontentloaded", "networkidle"],
                "description": "Navigation milestone to wait for (default: load).",
            },
            "settle_ms": {
                "type": "integer",
                "description": "Extra wait after load, for late rendering.",
            },
            "screenshot": {
                "type": "string",
                "description": "Write a full-page PNG to this path.",
            },
        },
        "required": ["url"],
    },
    "effect": "remote",
    "exec": "browser",
    "domain": "web",
}


def _handler(args: dict, sessions) -> str:
    from koma_sec_daemon import verdict  # noqa: PLC0415

    try:
        import playwright  # noqa: F401, PLC0415
    except ImportError:
        return verdict.unavailable("playwright", "pip install playwright && playwright install chromium")

    spec = {
        "url": args["url"],
        "wait_until": args.get("wait_until", "load"),
        "settle_ms": int(args.get("settle_ms") or 0),
        "screenshot": args.get("screenshot"),
        "cap": _PAGE_CAP,
        "timeout": DEFAULT_TIMEOUT,
        # The proxy is how a host-side browser stays inside the scope.
        "proxy": os.environ.get("HTTPS_PROXY") or os.environ.get("HTTP_PROXY"),
    }

    proc = subprocess.run(
        [sys.executable, "-c", _DRIVER],
        input=json.dumps(spec),
        capture_output=True,
        text=True,
        timeout=DEFAULT_TIMEOUT + 30,
    )
    line = (proc.stdout or "").strip().splitlines()[-1:] or [""]
    try:
        out = json.loads(line[0])
    except ValueError:
        return verdict.error(f"driver produced no result: {(proc.stderr or '')[-400:]}")

    if not out.get("ok"):
        # `blocked` for an egress refusal, `error` for anything else: a scope
        # refusal by the proxy is a deliberate stop, not a broken target.
        kind = verdict.BLOCKED if "403" in out.get("error", "") else verdict.ERROR
        return verdict.typed(kind, f"{out.get('stage')}: {out.get('error')}")

    body = [
        f"HTTP {out.get('status')}",
        f"final: {out.get('final_url')}",
        f"title: {out.get('title')}",
        "",
        out.get("text") or "",
    ]
    links = out.get("links") or []
    if links:
        body.append("\nlinks:\n  " + "\n  ".join(links[:100]))
    if out.get("screenshot"):
        body.append(f"\nscreenshot: {out['screenshot']}")

    # A redirect that left the starting origin is worth flagging even when the
    # proxy admitted it: it is the case a reader most needs to notice.
    if out.get("final_url") and _origin(out["final_url"]) != _origin(spec["url"]):
        body.insert(1, f"⚠ left origin: {_origin(spec['url'])} → {_origin(out['final_url'])}")

    return verdict.typed(verdict.CONFIRMED, "\n".join(body))


def _origin(url: str) -> str:
    from urllib.parse import urlsplit

    parts = urlsplit(url)
    return f"{parts.scheme}://{parts.netloc}"


DESCRIPTOR["handler"] = _handler
