"""
sec_xss_confirm — headless-browser XSS confirmation tool.

compute : executes-target
risk    : True
domain  : web

playwright is imported LAZILY inside the handler so the registry loads even
when the package is not installed (import-time safety for the smoke test).
"""

from __future__ import annotations

DESCRIPTOR = {
    "name": "sec_xss_confirm",
    "description": (
        "Headless-browser confirmation of XSS: load a URL in Playwright Chromium "
        "and detect a fired javascript dialog (alert/confirm/prompt) or an injected "
        "DOM marker. The visual/DOM-XSS differentiator."
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "url": {
                "type": "string",
                "description": "Target URL (typically already carrying the injection).",
            },
            "payload": {
                "type": "string",
                "description": "Optional payload string appended as a query value if provided.",
            },
            "wait_ms": {
                "type": "integer",
                "description": "Milliseconds to wait after page load for dialogs to fire (default: 2500).",
                "default": 2500,
            },
            "marker": {
                "type": "string",
                "description": "Optional DOM/text marker to search for in the rendered page source.",
            },
        },
        "required": ["url"],
    },
    "risk": True,
    "compute": "executes-target",
    "domain": "web",
}


def _handler(args: dict, sessions) -> str:
    # Lazy import — keeps registry loadable without playwright installed
    try:
        from playwright.sync_api import sync_playwright  # noqa: PLC0415
    except Exception as exc:
        return (
            f"error: playwright not installed "
            f"(pip install playwright && playwright install chromium): {exc}"
        )

    url = args["url"]
    payload = args.get("payload")
    wait_ms = int(args.get("wait_ms") or 2500)
    marker = args.get("marker")

    # Append payload as a query value if provided
    if payload:
        separator = "&" if "?" in url else "?"
        url = f"{url}{separator}xss={payload}"

    fired: list[str] = []
    result = "no dialog fired; XSS not confirmed via dialog"

    try:
        with sync_playwright() as pw:
            browser = pw.chromium.launch(headless=True)
            try:
                page = browser.new_page()

                # Capture javascript dialogs (alert/confirm/prompt)
                def _on_dialog(dialog):
                    fired.append(dialog.message)
                    dialog.dismiss()

                page.on("dialog", _on_dialog)

                # Navigate — catch goto errors but don't abort
                goto_error = None
                try:
                    page.goto(url, wait_until="load", timeout=15000)
                except Exception as exc:
                    goto_error = str(exc)

                # Wait for async payloads to trigger dialogs
                page.wait_for_timeout(wait_ms)

                if fired:
                    result = f"XSS CONFIRMED: javascript dialog fired -> {fired}"
                elif marker and marker in page.content():
                    result = (
                        f"MARKER PRESENT (reflected, dialog NOT fired): {marker}"
                    )
                elif goto_error:
                    result = (
                        f"no dialog fired; XSS not confirmed via dialog "
                        f"(goto error: {goto_error})"
                    )
            finally:
                browser.close()
    except Exception as exc:
        return f"error: {exc}"

    return result


# Attach handler to descriptor for registry use
DESCRIPTOR["handler"] = _handler
