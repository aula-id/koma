"""Persistent browser daemon for scrapion_agent.

Launches ONE Playwright Firefox instance and keeps it alive for the entire
Koma session.  Communicates with the Rust side via newline-delimited JSON
over a Unix-domain socket.

Usage (internal — invoked by ``__main__.``)::

    python -m scrapion_agent daemon --socket <path> --token <token>

All debug / logging output goes to stderr.  stdout is reserved for the
initial token line and must never be written to after startup.
"""

from __future__ import annotations

import argparse
import asyncio
import collections
import ipaddress
import json
import logging
import os
import re
import secrets
import signal
import socket
import sys
import uuid
from pathlib import Path
from typing import Any, Callable, Coroutine, Deque, Dict, List, Optional, Tuple
from urllib.parse import urlparse

# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

CONSOLE_BUFFER_LIMIT: int = 200
NETWORK_BUFFER_LIMIT: int = 100
EVENT_BUFFER_LIMIT: int = 500

TIMEOUT_NAVIGATE_MS: int = 90_000
TIMEOUT_SCREENSHOT_MS: int = 60_000
TIMEOUT_EVALUATE_MS: int = 30_000
TIMEOUT_INTERACT_MS: int = 30_000

DEFAULT_VIEWPORT_WIDTH: int = 1920
DEFAULT_VIEWPORT_HEIGHT: int = 1080

MAX_SOCKET_BACKLOG: int = 5
READ_BUF_SIZE: int = 64 * 1024

# ---------------------------------------------------------------------------
# Logging — all output to stderr so stdout stays clean for the token line
# ---------------------------------------------------------------------------

logging.basicConfig(
    level=logging.DEBUG,
    format="%(asctime)s [%(levelname)s] %(name)s: %(message)s",
    stream=sys.stderr,
)
log = logging.getLogger("scrapion_agent.daemon")

# ---------------------------------------------------------------------------
# URL validation — reject private / loopback / metadata addresses
# ---------------------------------------------------------------------------

_LOOPBACK_HOSTS = frozenset({"localhost", "127.0.0.1", "::1", "[::1]"})

_PRIVATE_IPV4_PREFIXES: List[Tuple[int, int]] = [
    (10, 8),          # 10.0.0.0/8
    (172, 12),        # 172.16.0.0/12  (bits 172.16–172.31)
    (192, 16),        # 192.168.0.0/16
]

_LINK_LOCAL_PREFIXES: List[Tuple[int, int]] = [
    (169, 24),        # 169.254.0.0/16 (IPv4 link-local + cloud metadata)
]

_PRIVATE_IPV6_PREFIXES: List[Tuple[int, int]] = [
    (0xfc00, 7),      # fc00::/7 (unique-local)
    (0xfe80, 10),     # fe80::/10 (link-local)
]

_METADATA_HOSTS = frozenset({"169.254.169.254"})


def _host_is_blocked(host: str) -> Optional[str]:
    """Return a rejection reason if *host* is blocked, else ``None``."""
    h = host.lower().strip("[]")

    # Plain loopback hostnames
    if h in _LOOPBACK_HOSTS:
        return "loopback address"

    # Cloud metadata
    if h in _METADATA_HOSTS:
        return "cloud metadata address"

    # Try parsing as IP
    try:
        ip = ipaddress.ip_address(h)
    except ValueError:
        return None

    if ip.is_loopback:
        return "loopback address"
    if ip.is_link_local:
        return "link-local address"
    if ip.is_private:
        return "private address"
    if ip.is_reserved:
        return "reserved address"

    return None


def validate_url(url: str) -> Optional[str]:
    """Validate a URL.  Returns an error string if blocked, else ``None``."""
    parsed = urlparse(url)
    if parsed.scheme not in ("http", "https"):
        return f"unsupported scheme '{parsed.scheme}'"
    host = parsed.hostname or ""
    if not host:
        return "no hostname in URL"
    return _host_is_blocked(host)


# ---------------------------------------------------------------------------
# Bounded buffer
# ---------------------------------------------------------------------------

class BoundedBuffer:
    """A thread-safe (asyncio-safe) bounded deque."""

    __slots__ = ("_buf", "_limit")

    def __init__(self, limit: int = EVENT_BUFFER_LIMIT) -> None:
        self._buf: Deque[dict] = collections.deque(maxlen=limit)
        self._limit = limit

    def append(self, entry: dict) -> None:
        self._buf.append(entry)

    def snapshot(self) -> List[dict]:
        return list(self._buf)

    def clear(self) -> None:
        self._buf.clear()

    def __len__(self) -> int:
        return len(self._buf)


# ---------------------------------------------------------------------------
# Per-tab state
# ---------------------------------------------------------------------------

class TabState:
    """Holds the Playwright Page and its event buffers."""

    __slots__ = (
        "tab_id", "page", "console_buf", "network_buf",
        "event_buf", "active",
    )

    def __init__(self, tab_id: str, page: Any) -> None:
        self.tab_id = tab_id
        self.page = page
        self.console_buf = BoundedBuffer(CONSOLE_BUFFER_LIMIT)
        self.network_buf = BoundedBuffer(NETWORK_BUFFER_LIMIT)
        self.event_buf = BoundedBuffer(EVENT_BUFFER_LIMIT)
        self.active = False

    async def url(self) -> str:
        try:
            return self.page.url
        except Exception:
            return ""

    async def title(self) -> str:
        try:
            return await self.page.title()
        except Exception:
            return ""


# ---------------------------------------------------------------------------
# Action handler — implements every daemon action
# ---------------------------------------------------------------------------

ActionFunc = Callable[["Handler", dict], Coroutine[Any, Any, dict]]


class Handler:
    """Dispatch table for daemon actions."""

    def __init__(self) -> None:
        self._actions: Dict[str, ActionFunc] = {
            # tab lifecycle
            "open": self._action_open,
            "list": self._action_list,
            "navigate": self._action_navigate,
            "close": self._action_close,
            "select": self._action_select,
            # page reading
            "page_content": self._action_page_content,
            "screenshot": self._action_screenshot,
            "inspect": self._action_inspect,
            # interaction
            "interact": self._action_interact,
            # javascript
            "evaluate": self._action_evaluate,
            # lifecycle
            "health": self._action_health,
            "shutdown": self._action_shutdown,
        }

        # Browser / context — set during daemon startup
        self._playwright: Any = None
        self._browser: Any = None
        self._context: Any = None
        self._tabs: Dict[str, TabState] = {}
        self._active_tab_id: Optional[str] = None
        self._shutdown_event: Optional[asyncio.Event] = None
        self._shutdown_requested = False

    # -- helpers -----------------------------------------------------------

    def _get_tab(self, params: dict) -> Tuple[TabState, Optional[str]]:
        """Resolve a TabState from params['tab_id'] or active tab.

        Returns (tab, error_string_or_None).
        """
        tab_id = params.get("tab_id")
        if not tab_id:
            tab_id = self._active_tab_id
        if not tab_id or tab_id not in self._tabs:
            return None, f"tab not found: {tab_id}"
        return self._tabs[tab_id], None

    def _ok(self, data: dict) -> dict:
        return {"status": "ok", "data": data}

    def _error(self, msg: str) -> dict:
        return {"status": "error", "error": msg}

    async def dispatch(self, action: str, params: dict) -> dict:
        """Route *action* to the correct handler method."""
        fn = self._actions.get(action)
        if fn is None:
            return self._error(f"unknown action: {action}")
        try:
            return await fn(params)
        except Exception as exc:
            log.exception("action %s failed", action)
            return self._error(str(exc))

    # -- tab lifecycle actions ---------------------------------------------

    async def _action_open(self, params: dict) -> dict:
        url = params.get("url", "")
        if not url:
            return self._error("url is required")
        err = validate_url(url)
        if err:
            return self._error(f"blocked URL: {err}")

        tab_id = uuid.uuid4().hex[:12]
        page = await self._context.new_page()
        await page.goto(url, timeout=TIMEOUT_NAVIGATE_MS, wait_until="networkidle")
        title = await page.title()

        tab = TabState(tab_id, page)
        self._tabs[tab_id] = tab

        # Wire up event listeners
        self._wire_events(tab)

        # Set as active if it's the first tab
        if not self._active_tab_id:
            tab.active = True
            self._active_tab_id = tab_id

        final_url = page.url
        return self._ok({"tab_id": tab_id, "url": final_url, "title": title or ""})

    async def _action_list(self, _params: dict) -> dict:
        tabs = []
        for tid, tab in self._tabs.items():
            tabs.append({
                "tab_id": tid,
                "url": await tab.url(),
                "title": await tab.title(),
                "active": tid == self._active_tab_id,
            })
        return self._ok({"tabs": tabs})

    async def _action_navigate(self, params: dict) -> dict:
        tab, err = self._get_tab(params)
        if err:
            return self._error(err)
        url = params.get("url", "")
        if not url:
            return self._error("url is required")
        err2 = validate_url(url)
        if err2:
            return self._error(f"blocked URL: {err2}")

        await tab.page.goto(url, timeout=TIMEOUT_NAVIGATE_MS, wait_until="networkidle")
        title = await tab.page.title()
        return self._ok({"url": tab.page.url, "title": title or ""})

    async def _action_close(self, params: dict) -> dict:
        tab, err = self._get_tab(params)
        if err:
            return self._error(err)

        try:
            await tab.page.close()
        except Exception:
            pass
        del self._tabs[tab.tab_id]

        # Re-assign active tab
        if self._active_tab_id == tab.tab_id:
            self._active_tab_id = None
            if self._tabs:
                first_id = next(iter(self._tabs))
                self._tabs[first_id].active = True
                self._active_tab_id = first_id

        return self._ok({"closed": True})

    async def _action_select(self, params: dict) -> dict:
        tab_id = params.get("tab_id", "")
        if tab_id not in self._tabs:
            return self._error(f"tab not found: {tab_id}")

        for t in self._tabs.values():
            t.active = False
        self._tabs[tab_id].active = True
        self._active_tab_id = tab_id
        return self._ok({"active": True})

    # -- page reading actions ----------------------------------------------

    async def _action_page_content(self, params: dict) -> dict:
        tab, err = self._get_tab(params)
        if err:
            return self._error(err)

        from markdownify import markdownify as md

        html = await tab.page.content()
        content = md(html, heading_style="ATX")
        title = await tab.title()
        return self._ok({
            "url": tab.page.url,
            "title": title,
            "content": content,
        })

    async def _action_screenshot(self, params: dict) -> dict:
        tab, err = self._get_tab(params)
        if err:
            return self._error(err)

        output_path = params.get("output_path", "")
        if not output_path:
            return self._error("output_path is required")

        width = int(params.get("width", DEFAULT_VIEWPORT_WIDTH))
        height = int(params.get("height", DEFAULT_VIEWPORT_HEIGHT))
        delay_ms = int(params.get("delay_ms", 300))
        full_page = bool(params.get("full_page", True))

        out = Path(output_path)
        out.parent.mkdir(parents=True, exist_ok=True)

        # Set viewport if needed
        await tab.page.set_viewport_size({"width": width, "height": height})
        if delay_ms > 0:
            await asyncio.sleep(delay_ms / 1000.0)

        await tab.page.screenshot(path=str(out), full_page=full_page)

        return self._ok({
            "output_path": str(out.resolve()),
            "width": width,
            "height": height,
        })

    async def _action_inspect(self, params: dict) -> dict:
        tab, err = self._get_tab(params)
        if err:
            return self._error(err)

        what = params.get("what", "html")
        if what == "html":
            html = await tab.page.content()
            # Truncate to 1 MB for safety
            if len(html) > 1_000_000:
                html = html[:1_000_000] + "\n<!-- truncated -->"
            return self._ok({"html": html, "url": tab.page.url, "title": await tab.title()})
        elif what == "console":
            return self._ok({"entries": tab.console_buf.snapshot(), "count": len(tab.console_buf)})
        elif what == "network":
            return self._ok({"entries": tab.network_buf.snapshot(), "count": len(tab.network_buf)})
        else:
            return self._error(f"unknown inspect target: {what}")

    # -- interaction actions -----------------------------------------------

    async def _action_interact(self, params: dict) -> dict:
        tab, err = self._get_tab(params)
        if err:
            return self._error(err)

        action = params.get("action", "")
        action_params = params.get("params", {})

        page = tab.page

        try:
            if action == "click":
                locator = await self._resolve_locator(page, action_params)
                await locator.click(timeout=TIMEOUT_INTERACT_MS)
                return self._ok({"clicked": True})

            elif action == "fill":
                locator = await self._resolve_locator(page, action_params)
                value = action_params.get("value", "")
                await locator.fill(value, timeout=TIMEOUT_INTERACT_MS)
                return self._ok({"filled": True})

            elif action == "press":
                key = action_params.get("key") or action_params.get("value", "")
                if not key:
                    return self._error("key is required for press")
                await page.keyboard.press(key)
                return self._ok({"pressed": key})

            elif action == "select":
                locator = await self._resolve_locator(page, action_params)
                value = action_params.get("value", "")
                await locator.select_option(value, timeout=TIMEOUT_INTERACT_MS)
                return self._ok({"selected": value})

            elif action == "scroll":
                direction = action_params.get("direction", "down")
                amount = int(action_params.get("amount", 500))
                delta_y = amount if direction == "down" else -amount
                await page.mouse.wheel(0, delta_y)
                return self._ok({"scrolled": direction, "amount": amount})

            elif action == "wait":
                wait_what = action_params.get("what", "selector")
                wait_value = action_params.get("value", "")
                timeout_ms = int(action_params.get("timeout_ms", 5000))

                if wait_what == "selector":
                    await page.wait_for_selector(wait_value, timeout=timeout_ms)
                elif wait_what == "url":
                    await page.wait_for_url(wait_value, timeout=timeout_ms)
                elif wait_what == "response":
                    await page.wait_for_response(wait_value, timeout=timeout_ms)
                else:
                    return self._error(f"unknown wait target: {wait_what}")
                return self._ok({"waited": wait_what})

            else:
                return self._error(f"unknown interact action: {action}")
        except Exception as exc:
            return self._error(f"interact.{action} failed: {exc}")

    @staticmethod
    async def _resolve_locator(page: Any, params: dict) -> Any:
        """Build a Playwright Locator from params."""
        locator_type = params.get("locator_type", "css")
        selector = params.get("locator", "")
        if not selector:
            raise ValueError("locator is required")

        if locator_type == "role":
            return page.get_by_role(selector)
        elif locator_type == "text":
            return page.get_by_text(selector)
        elif locator_type == "css":
            return page.locator(selector)
        else:
            return page.locator(selector)

    # -- javascript actions ------------------------------------------------

    async def _action_evaluate(self, params: dict) -> dict:
        tab, err = self._get_tab(params)
        if err:
            return self._error(err)

        script = params.get("script", "")
        if not script:
            return self._error("script is required")

        args = params.get("args", [])

        try:
            result = await asyncio.wait_for(
                tab.page.evaluate(script, args),
                timeout=TIMEOUT_EVALUATE_MS / 1000.0,
            )
        except asyncio.TimeoutError:
            return self._error("evaluate timed out")
        except Exception as exc:
            return self._error(f"evaluate failed: {exc}")

        return self._ok({"result": result})

    # -- lifecycle actions -------------------------------------------------

    async def _action_health(self, _params: dict) -> dict:
        return self._ok({"status": "healthy"})

    async def _action_shutdown(self, _params: dict) -> dict:
        log.info("shutdown requested")
        self._shutdown_requested = True
        if self._shutdown_event:
            self._shutdown_event.set()
        return self._ok({"shutdown": True})

    # -- event wiring ------------------------------------------------------

    def _wire_events(self, tab: TabState) -> None:
        """Attach Playwright event listeners to a tab's page."""

        page = tab.page

        def _on_console(msg: Any) -> None:
            entry = {
                "type": msg.type,
                "text": msg.text,
                "location": str(msg.location) if hasattr(msg, "location") else "",
            }
            tab.console_buf.append(entry)
            tab.event_buf.append({"event": "console", **entry})

        def _on_response(resp: Any) -> None:
            entry = {
                "status": resp.status,
                "url": resp.url,
                "method": resp.request.method if resp.request else "",
            }
            tab.network_buf.append(entry)
            tab.event_buf.append({"event": "response", **entry})

        def _on_request(req: Any) -> None:
            entry = {
                "url": req.url,
                "method": req.method,
                "resource_type": req.resource_type,
            }
            tab.event_buf.append({"event": "request", **entry})

        def _on_popup(popup: Any) -> None:
            tab.event_buf.append({
                "event": "popup",
                "url": popup.url if popup else "",
            })

        def _on_crash() -> None:
            tab.event_buf.append({"event": "crash"})

        def _on_close() -> None:
            tab.event_buf.append({"event": "page_close"})

        def _on_websocket(ws: Any) -> None:
            tab.event_buf.append({
                "event": "websocket",
                "url": ws.url if ws else "",
            })

        page.on("console", _on_console)
        page.on("response", _on_response)
        page.on("request", _on_request)
        page.on("popup", _on_popup)
        page.on("crash", _on_crash)
        page.on("close", _on_close)
        page.on("websocket", _on_websocket)


# ---------------------------------------------------------------------------
# Daemon server — socket listener + event loop
# ---------------------------------------------------------------------------

class DaemonServer:
    """Unix-socket JSON-line daemon that owns the browser lifecycle."""

    def __init__(self, socket_path: str, token: str) -> None:
        self.socket_path = socket_path
        self.token = token
        self.handler = Handler()
        self._server: Optional[asyncio.AbstractServer] = None
        self._clients: set = set()
        self._shutdown_event: asyncio.Event = asyncio.Event()
        self.handler._shutdown_event = self._shutdown_event

    # -- browser lifecycle -------------------------------------------------

    async def _start_browser(self) -> None:
        from playwright.async_api import async_playwright

        self.handler._playwright = await async_playwright().start()
        self.handler._browser = await self.handler._playwright.firefox.launch(
            headless=True,
            args=[
                "--disable-blink-features=AutomationControlled",
                "--disable-dev-shm-usage",
                "--no-sandbox",
                "--start-maximized",
            ],
        )
        self.handler._context = await self.handler._browser.new_context(
            viewport={"width": DEFAULT_VIEWPORT_WIDTH, "height": DEFAULT_VIEWPORT_HEIGHT},
            user_agent=(
                "Mozilla/5.0 (X11; Linux x86_64; rv:128.0) Gecko/20100101 Firefox/128.0"
            ),
        )
        log.info("browser started")

    async def _stop_browser(self) -> None:
        """Gracefully close everything."""
        log.info("stopping browser")
        # Close all pages via context first (handles close() gracefully)
        try:
            if self.handler._context:
                for tab in list(self.handler._tabs.values()):
                    try:
                        await tab.page.close()
                    except Exception:
                        pass
                self.handler._tabs.clear()
                await self.handler._context.close()
        except Exception:
            pass

        try:
            if self.handler._browser:
                await self.handler._browser.close()
        except Exception:
            pass

        try:
            if self.handler._playwright:
                await self.handler._playwright.stop()
        except Exception:
            pass

        log.info("browser stopped")

    # -- client handling ---------------------------------------------------

    async def _handle_client(self, reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
        """Process newline-delimited JSON requests from one client."""
        self._clients.add(writer)
        peer = writer.get_extra_info("peername")
        log.info("client connected: %s", peer)

        try:
            auth_ok = await self._authenticate(reader, writer)
            if not auth_ok:
                return

            # Read lines until EOF or shutdown
            while not self._shutdown_event.is_set():
                line = await reader.readline()
                if not line:
                    break  # client disconnected

                line = line.strip()
                if not line:
                    continue

                try:
                    request = json.loads(line)
                except json.JSONDecodeError as exc:
                    await self._send_response(writer, None, self.handler._error(f"invalid JSON: {exc}"))
                    continue

                req_id = request.get("id")
                action = request.get("action", "")
                params = request.get("params", {})

                response = await self.handler.dispatch(action, params)
                await self._send_response(writer, req_id, response)

                if self.handler._shutdown_requested:
                    break
        except asyncio.CancelledError:
            pass
        except Exception as exc:
            log.exception("client error: %s", exc)
        finally:
            self._clients.discard(writer)
            try:
                writer.close()
                await writer.wait_closed()
            except Exception:
                pass
            log.info("client disconnected: %s", peer)

    async def _authenticate(self, reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> bool:
        """Read and validate the first line as an auth token."""
        line = await reader.readline()
        if not line:
            return False

        received = line.decode().strip()
        if received != self.token:
            resp = self.handler._error("authentication failed")
            resp["id"] = None
            await self._send_response_raw(writer, resp)
            try:
                writer.close()
                await writer.wait_closed()
            except Exception:
                pass
            return False

        # Send auth OK
        await self._send_response_raw(writer, {"id": None, "status": "ok", "data": {"authenticated": True}})
        return True

    @staticmethod
    async def _send_response(writer: asyncio.StreamWriter, req_id: Any, response: dict) -> None:
        response["id"] = req_id
        await DaemonServer._send_response_raw(writer, response)

    @staticmethod
    async def _send_response_raw(writer: asyncio.StreamWriter, response: dict) -> None:
        payload = json.dumps(response, ensure_ascii=False, default=str) + "\n"
        writer.write(payload.encode())
        await writer.drain()

    # -- server main loop --------------------------------------------------

    async def run(self) -> None:
        """Start the daemon: browser, socket, event loop."""
        # Clean up stale socket file
        sock_path = Path(self.socket_path)
        if sock_path.exists():
            sock_path.unlink()
        sock_path.parent.mkdir(parents=True, exist_ok=True)

        await self._start_browser()

        # Create Unix socket server
        self._server = await asyncio.start_unix_server(
            self._handle_client,
            path=self.socket_path,
        )

        # Print token to stdout as the FIRST line (for Rust side to read)
        # Use unbuffered write to ensure it's sent immediately.
        token_line = self.token + "\n"
        stdout = sys.stdout
        stdout.write(token_line)
        stdout.flush()
        log.info("daemon listening on %s", self.socket_path)

        # Install signal handlers
        loop = asyncio.get_running_loop()
        for sig in (signal.SIGTERM, signal.SIGINT):
            loop.add_signal_handler(sig, lambda: asyncio.ensure_future(self._signal_shutdown()))

        # Wait until shutdown
        await self._shutdown_event.wait()

        # Close server socket
        if self._server:
            self._server.close()
            await self._server.wait_closed()

        # Stop browser
        await self._stop_browser()
        log.info("daemon exited")

    async def _signal_shutdown(self) -> None:
        log.info("received shutdown signal")
        self.handler._shutdown_requested = True
        self._shutdown_event.set()


# ---------------------------------------------------------------------------
# CLI entry point
# ---------------------------------------------------------------------------

def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Persistent browser daemon for scrapion_agent",
        prog="python -m scrapion_agent daemon",
    )
    parser.add_argument(
        "--socket",
        required=True,
        help="Path for the Unix-domain socket",
    )
    parser.add_argument(
        "--token",
        default=None,
        help="Auth token (if omitted, a random one is generated)",
    )
    return parser.parse_args()


def main() -> None:
    """Entry point for ``python -m scrapion_agent daemon``."""
    args = parse_args()
    token = args.token or secrets.token_hex(32)

    # All logging goes to stderr
    log.info("starting daemon")

    server = DaemonServer(socket_path=args.socket, token=token)

    try:
        asyncio.run(server.run())
    except KeyboardInterrupt:
        log.info("interrupted")
    except Exception:
        log.exception("fatal error in daemon")
        sys.exit(1)


if __name__ == "__main__":
    main()
