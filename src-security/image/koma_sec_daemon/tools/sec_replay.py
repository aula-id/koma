"""
sec_replay — re-send a saved recording against the target.

compute : network
risk    : True
domain  : web

The re-send path, and the one place a detector can be checked: run the same
recording against a patched target and a baseline, then compare. `sec_xss` uses it
that way.

Every request still goes through the run's egress proxy, so a recording captured
elsewhere cannot widen the scope by carrying an out-of-scope URL: the proxy re-checks
each hop. A replayed entry whose host is out of scope is reported per-request rather
than aborting the whole recording, because the useful answer is usually "most of it
replayed and these three did not".

Uses `requests`, imported lazily so the registry loads without it.
"""

from __future__ import annotations

import json
import os
from pathlib import Path

MAX_REPLAY = 100
TIMEOUT = 20
BODY_EXCERPT = 400


def _spawn():
    from koma_sec_daemon import registry

    return registry.spawn_for_thread()


DESCRIPTOR = {
    "name": "sec_replay",
    "description": (
        "Re-send a saved recording against the target and report each response. "
        "The verification path for a proposed fix: run it against the patched "
        "target and compare with the baseline run."
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "name": {
                "type": "string",
                "description": "Recording name, as saved by sec_recording.",
            },
            "target": {
                "type": "string",
                "description": (
                    "Optional base to re-point every entry at, e.g. "
                    "http://staging.local — keeps path and query, replaces origin."
                ),
            },
            "limit": {
                "type": "integer",
                "description": f"Replay at most this many entries. Default {MAX_REPLAY}.",
            },
            "stop_on_error": {
                "type": "boolean",
                "description": "Stop at the first non-2xx/3xx instead of continuing.",
            },
        },
        "required": ["name"],
    },
    "risk": True,
    "compute": "network",
    "domain": "web",
    "network_param": "target",
}


def _dir() -> Path:
    root = Path(os.environ.get("KOMA_BOARD") or "/tmp/koma-board")
    return root / "recording"


def _repoint(url: str, base: str) -> str:
    """Replace the origin of `url` with `base`, keeping path and query."""
    from urllib.parse import urlsplit, urlunsplit  # noqa: PLC0415

    if not base:
        return url
    parts = urlsplit(url)
    b = urlsplit(base if "://" in base else f"http://{base}")
    return urlunsplit((b.scheme, b.netloc, parts.path or "/", parts.query, ""))


def _handler(args: dict, sessions) -> str:
    import requests  # noqa: PLC0415

    name = args["name"]
    path = _dir() / f"{name}.json"
    if not path.exists():
        return json.dumps(
            {"verdict": "not_found", "excerpt": f"no recording named {name!r}"}
        )
    doc = json.loads(path.read_text(encoding="utf-8"))
    entries = doc.get("entries") or []
    limit = int(args.get("limit") or MAX_REPLAY)
    stop = bool(args.get("stop_on_error"))
    target = args.get("target") or ""

    if not entries:
        return json.dumps({"verdict": "unproven", "excerpt": "recording is empty"})

    proxy = os.environ.get("HTTPS_PROXY") or os.environ.get("HTTP_PROXY")
    proxies = {"http": proxy, "https": proxy} if proxy else None

    lines: list[str] = []
    ok = blocked = failed = 0
    for i, e in enumerate(entries[:limit]):
        url = _repoint(str(e.get("url", "")), target)
        try:
            resp = requests.request(
                str(e.get("method") or "GET"),
                url,
                headers=dict(e.get("headers") or {}),
                data=(e.get("body") or None),
                timeout=TIMEOUT,
                proxies=proxies,
                allow_redirects=False,
            )
        except requests.RequestException as exc:
            # A proxy refusal surfaces here as a connection error, so the reason
            # text is what distinguishes "out of scope" from "target down".
            failed += 1
            lines.append(f"{i} {url} ERROR {exc}")
            if stop:
                break
            continue
        marker = resp.headers.get("X-Koma-Egress")
        if marker:
            blocked += 1
            lines.append(f"{i} {url} BLOCKED {marker}")
            if stop:
                break
            continue
        if resp.status_code >= 500:
            failed += 1
        else:
            ok += 1
        body = resp.text[:BODY_EXCERPT].replace("\n", " ")
        lines.append(f"{i} {url} {resp.status_code} {len(resp.content)}b {body}")
        if stop and resp.status_code >= 400:
            break

    verdict = "confirmed" if ok and not blocked and not failed else "unproven"
    head = f"replayed {min(len(entries), limit)}: {ok} ok, {blocked} blocked, {failed} failed"
    return json.dumps({"verdict": verdict, "excerpt": head + "\n\n" + "\n".join(lines)})


def handler(args: dict, sessions) -> str:
    return _handler(args, sessions)
