"""Unit tests for scrapion_agent.daemon — the persistent browser daemon.

All Playwright calls are mocked.  No real Firefox is launched.

Run with:  python -m pytest src-internet/tests/test_daemon.py -v
"""

import asyncio
import collections
import json
import os
import sys
import tempfile
from pathlib import Path
from unittest.mock import AsyncMock, MagicMock, patch

import pytest

IS_WINDOWS = sys.platform == "win32"

# Ensure scrapion_agent is importable.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from scrapion_agent.daemon import (
    BoundedBuffer,
    CONSOLE_BUFFER_LIMIT,
    EVENT_BUFFER_LIMIT,
    Handler,
    TabState,
    validate_url,
    _host_is_blocked,
    parse_args,
    DaemonServer,
)

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _make_mock_page(url: str = "https://example.com") -> MagicMock:
    """Create a mock Playwright Page with common stubs."""
    page = AsyncMock()
    page.url = url
    page.title = AsyncMock(return_value="Example Page")
    page.content = AsyncMock(return_value="<html><body><h1>Hello</h1></body></html>")
    page.goto = AsyncMock()
    page.close = AsyncMock()
    page.screenshot = AsyncMock()
    page.set_viewport_size = AsyncMock()
    page.keyboard = AsyncMock()
    page.mouse = AsyncMock()
    page.evaluate = AsyncMock(return_value={"result": 42})
    page.locator = MagicMock(return_value=AsyncMock())
    page.get_by_role = MagicMock(return_value=AsyncMock())
    page.get_by_text = MagicMock(return_value=AsyncMock())
    page.wait_for_selector = AsyncMock()
    page.wait_for_url = AsyncMock()
    page.wait_for_response = AsyncMock()
    page.add_init_script = AsyncMock()
    # Event listeners — store them for later triggering
    page._listeners: dict = {}
    page.on = MagicMock(side_effect=lambda evt, fn: page._listeners.update({evt: fn}))
    page.viewport_size = {"width": 1920, "height": 1080}
    return page


def _make_handler_with_mock_browser(tmp_path: Path = None) -> Handler:
    """Create a Handler with mocked playwright/browser/context."""
    handler = Handler()
    handler._playwright = AsyncMock()
    handler._browser = AsyncMock()
    handler._context = AsyncMock()
    # new_page returns a mock page
    mock_page = _make_mock_page()
    handler._context.new_page = AsyncMock(return_value=mock_page)
    return handler, mock_page


async def _run_action(handler: Handler, action: str, params: dict = None) -> dict:
    """Dispatch an action and return the response."""
    return await handler.dispatch(action, params or {})


def _sync_run(coro):
    """Run a coroutine synchronously."""
    return asyncio.get_event_loop().run_until_complete(coro)


# ---------------------------------------------------------------------------
# 1. URL validation
# ---------------------------------------------------------------------------

class TestUrlValidation:
    """validate_url blocks private/loopback/metadata addresses."""

    def test_allows_public_url(self):
        assert validate_url("https://example.com") is None

    def test_allows_public_url_with_path(self):
        assert validate_url("https://docs.python.org/3/") is None

    def test_blocks_loopback_localhost(self):
        assert "loopback" in validate_url("http://localhost:8080/")

    def test_blocks_loopback_127(self):
        assert "loopback" in validate_url("http://127.0.0.1/")

    def test_blocks_loopback_ipv6(self):
        assert "loopback" in validate_url("http://[::1]/")

    def test_blocks_private_10(self):
        assert "private" in validate_url("http://10.0.0.1/")

    def test_blocks_private_172_16(self):
        assert "private" in validate_url("http://172.16.0.1/")

    def test_blocks_private_192_168(self):
        assert "private" in validate_url("http://192.168.1.1/")

    def test_blocks_cloud_metadata(self):
        assert "metadata" in validate_url("http://169.254.169.254/latest/meta-data/")

    def test_blocks_link_local(self):
        assert "link-local" in validate_url("http://169.254.1.1/")

    def test_blocks_private_ipv6(self):
        assert "private" in validate_url("http://[fc00::1]/")

    def test_blocks_link_local_ipv6(self):
        assert "link-local" in validate_url("http://[fe80::1]/")

    def test_blocks_non_http_schemes(self):
        assert "unsupported scheme" in validate_url("file:///etc/passwd")
        assert "unsupported scheme" in validate_url("ftp://example.com")

    def test_blocks_no_hostname(self):
        assert "no hostname" in validate_url("https://")

    def test_host_is_blocked_loopback(self):
        assert _host_is_blocked("localhost") is not None
        assert _host_is_blocked("127.0.0.1") is not None

    def test_host_is_blocked_metadata(self):
        assert _host_is_blocked("169.254.169.254") is not None

    def test_host_allows_public(self):
        assert _host_is_blocked("example.com") is None
        assert _host_is_blocked("8.8.8.8") is None


# ---------------------------------------------------------------------------
# 2. BoundedBuffer
# ---------------------------------------------------------------------------

class TestBoundedBuffer:
    """BoundedBuffer enforces max length."""

    def test_append_and_snapshot(self):
        buf = BoundedBuffer(limit=5)
        for i in range(3):
            buf.append({"i": i})
        assert len(buf) == 3
        snap = buf.snapshot()
        assert len(snap) == 3
        assert snap[0]["i"] == 0

    def test_overflow_evicts_oldest(self):
        buf = BoundedBuffer(limit=3)
        for i in range(10):
            buf.append({"i": i})
        snap = buf.snapshot()
        assert len(snap) == 3
        # Last 3 entries
        assert snap[0]["i"] == 7
        assert snap[1]["i"] == 8
        assert snap[2]["i"] == 9

    def test_clear(self):
        buf = BoundedBuffer(limit=10)
        for i in range(5):
            buf.append({"i": i})
        buf.clear()
        assert len(buf) == 0
        assert buf.snapshot() == []

    def test_default_limit(self):
        buf = BoundedBuffer()
        # Default limit should be EVENT_BUFFER_LIMIT
        for i in range(EVENT_BUFFER_LIMIT + 100):
            buf.append({"i": i})
        assert len(buf) == EVENT_BUFFER_LIMIT


# ---------------------------------------------------------------------------
# 3. TabState
# ---------------------------------------------------------------------------

class TestTabState:
    """TabState wraps a page and buffers."""

    def test_tab_creation(self):
        page = _make_mock_page("https://test.com")
        tab = TabState("tab-1", page)
        assert tab.tab_id == "tab-1"
        assert tab.active is False

    def test_tab_url(self):
        page = _make_mock_page("https://test.com")
        tab = TabState("tab-1", page)
        url = _sync_run(tab.url())
        assert url == "https://test.com"

    def test_tab_title(self):
        page = _make_mock_page()
        tab = TabState("tab-1", page)
        title = _sync_run(tab.title())
        assert title == "Example Page"

    def test_tab_buffers(self):
        page = _make_mock_page()
        tab = TabState("tab-1", page)
        tab.console_buf.append({"text": "hello"})
        tab.network_buf.append({"url": "https://x.com"})
        tab.event_buf.append({"event": "console"})
        assert len(tab.console_buf) == 1
        assert len(tab.network_buf) == 1
        assert len(tab.event_buf) == 1


# ---------------------------------------------------------------------------
# 4. Handler — tab lifecycle
# ---------------------------------------------------------------------------

class TestHandlerTabLifecycle:
    """Test open / list / close / navigate / select actions."""

    @pytest.fixture
    def handler_and_page(self):
        handler, page = _make_handler_with_mock_browser()
        return handler, page

    def test_open_tab(self, handler_and_page):
        handler, page = handler_and_page
        result = _sync_run(handler.dispatch("open", {"url": "https://example.com"}))
        assert result["status"] == "ok"
        data = result["data"]
        assert "tab_id" in data
        assert data["url"] == "https://example.com"
        assert data["title"] == "Example Page"
        # Should be active (first tab)
        assert handler._active_tab_id == data["tab_id"]

    def test_open_blocked_url(self, handler_and_page):
        handler, _ = handler_and_page
        result = _sync_run(handler.dispatch("open", {"url": "http://localhost/"}))
        assert result["status"] == "error"
        assert "blocked" in result["error"]

    def test_open_no_url(self, handler_and_page):
        handler, _ = handler_and_page
        result = _sync_run(handler.dispatch("open", {}))
        assert result["status"] == "error"
        assert "url is required" in result["error"]

    def test_list_tabs(self, handler_and_page):
        handler, page = handler_and_page
        open_result = _sync_run(handler.dispatch("open", {"url": "https://example.com"}))
        tab_id = open_result["data"]["tab_id"]
        result = _sync_run(handler.dispatch("list", {}))
        assert result["status"] == "ok"
        assert len(result["data"]["tabs"]) == 1
        assert result["data"]["tabs"][0]["tab_id"] == tab_id
        assert result["data"]["tabs"][0]["active"] is True

    def test_close_tab(self, handler_and_page):
        handler, page = handler_and_page
        open_result = _sync_run(handler.dispatch("open", {"url": "https://example.com"}))
        tab_id = open_result["data"]["tab_id"]
        result = _sync_run(handler.dispatch("close", {"tab_id": tab_id}))
        assert result["status"] == "ok"
        assert result["data"]["closed"] is True
        assert tab_id not in handler._tabs
        assert handler._active_tab_id is None

    def test_close_nonexistent_tab(self, handler_and_page):
        handler, _ = handler_and_page
        result = _sync_run(handler.dispatch("close", {"tab_id": "nope"}))
        assert result["status"] == "error"
        assert "not found" in result["error"]

    def test_navigate(self, handler_and_page):
        handler, page = handler_and_page
        open_result = _sync_run(handler.dispatch("open", {"url": "https://example.com"}))
        tab_id = open_result["data"]["tab_id"]
        result = _sync_run(handler.dispatch("navigate", {"tab_id": tab_id, "url": "https://other.com"}))
        assert result["status"] == "ok"
        assert result["data"]["url"] == "https://example.com"

    def test_navigate_blocked(self, handler_and_page):
        handler, _ = handler_and_page
        open_result = _sync_run(handler.dispatch("open", {"url": "https://example.com"}))
        tab_id = open_result["data"]["tab_id"]
        result = _sync_run(handler.dispatch("navigate", {"tab_id": tab_id, "url": "http://127.0.0.1/"}))
        assert result["status"] == "error"
        assert "blocked" in result["error"]

    def test_select_tab(self, handler_and_page):
        handler, page = handler_and_page
        open_result = _sync_run(handler.dispatch("open", {"url": "https://example.com"}))
        tab_id = open_result["data"]["tab_id"]
        # Open a second tab — need to create a second mock page
        page2 = _make_mock_page("https://two.com")
        handler._context.new_page = AsyncMock(return_value=page2)
        open_result2 = _sync_run(handler.dispatch("open", {"url": "https://two.com"}))
        tab_id2 = open_result2["data"]["tab_id"]

        # tab_id should still be active (first opened)
        assert handler._active_tab_id == tab_id

        # select tab_id2
        result = _sync_run(handler.dispatch("select", {"tab_id": tab_id2}))
        assert result["status"] == "ok"
        assert handler._active_tab_id == tab_id2

    def test_select_nonexistent(self, handler_and_page):
        handler, _ = handler_and_page
        result = _sync_run(handler.dispatch("select", {"tab_id": "nope"}))
        assert result["status"] == "error"

    def test_unknown_action(self, handler_and_page):
        handler, _ = handler_and_page
        result = _sync_run(handler.dispatch("bogus_action", {}))
        assert result["status"] == "error"
        assert "unknown action" in result["error"]

    def test_use_active_tab_implicitly(self, handler_and_page):
        handler, _ = handler_and_page
        open_result = _sync_run(handler.dispatch("open", {"url": "https://example.com"}))
        tab_id = open_result["data"]["tab_id"]
        # page_content without tab_id should use active
        result = _sync_run(handler.dispatch("page_content", {}))
        assert result["status"] == "ok"
        assert "Hello" in result["data"]["content"]


# ---------------------------------------------------------------------------
# 5. Handler — page reading
# ---------------------------------------------------------------------------

class TestHandlerPageReading:

    @pytest.fixture
    def handler_with_tab(self):
        handler, page = _make_handler_with_mock_browser()
        result = _sync_run(handler.dispatch("open", {"url": "https://example.com"}))
        tab_id = result["data"]["tab_id"]
        return handler, tab_id

    def test_page_content(self, handler_with_tab):
        handler, tab_id = handler_with_tab
        result = _sync_run(handler.dispatch("page_content", {"tab_id": tab_id}))
        assert result["status"] == "ok"
        assert "Hello" in result["data"]["content"]
        assert result["data"]["url"] == "https://example.com"

    def test_screenshot(self, handler_with_tab, tmp_path):
        handler, tab_id = handler_with_tab
        output = str(tmp_path / "test.png")
        result = _sync_run(handler.dispatch("screenshot", {
            "tab_id": tab_id,
            "output_path": output,
            "width": 1280,
            "height": 720,
        }))
        assert result["status"] == "ok"
        assert result["data"]["output_path"] == str(Path(output).resolve())
        assert result["data"]["width"] == 1280
        assert result["data"]["height"] == 720

    def test_screenshot_no_output_path(self, handler_with_tab):
        handler, tab_id = handler_with_tab
        result = _sync_run(handler.dispatch("screenshot", {"tab_id": tab_id}))
        assert result["status"] == "error"
        assert "output_path is required" in result["error"]

    def test_inspect_html(self, handler_with_tab):
        handler, tab_id = handler_with_tab
        result = _sync_run(handler.dispatch("inspect", {"tab_id": tab_id, "what": "html"}))
        assert result["status"] == "ok"
        assert "Hello" in result["data"]["html"]

    def test_inspect_console(self, handler_with_tab):
        handler, tab_id = handler_with_tab
        result = _sync_run(handler.dispatch("inspect", {"tab_id": tab_id, "what": "console"}))
        assert result["status"] == "ok"
        assert "entries" in result["data"]
        assert result["data"]["count"] == 0

    def test_inspect_network(self, handler_with_tab):
        handler, tab_id = handler_with_tab
        result = _sync_run(handler.dispatch("inspect", {"tab_id": tab_id, "what": "network"}))
        assert result["status"] == "ok"
        assert "entries" in result["data"]

    def test_inspect_unknown(self, handler_with_tab):
        handler, tab_id = handler_with_tab
        result = _sync_run(handler.dispatch("inspect", {"tab_id": tab_id, "what": "cookies"}))
        assert result["status"] == "error"


# ---------------------------------------------------------------------------
# 6. Handler — interaction
# ---------------------------------------------------------------------------

class TestHandlerInteraction:

    @pytest.fixture
    def handler_with_tab(self):
        handler, page = _make_handler_with_mock_browser()
        result = _sync_run(handler.dispatch("open", {"url": "https://example.com"}))
        tab_id = result["data"]["tab_id"]
        return handler, tab_id

    def test_interact_click(self, handler_with_tab):
        handler, tab_id = handler_with_tab
        result = _sync_run(handler.dispatch("interact", {
            "tab_id": tab_id,
            "action": "click",
            "params": {"locator": "button#submit", "locator_type": "css"},
        }))
        assert result["status"] == "ok"

    def test_interact_fill(self, handler_with_tab):
        handler, tab_id = handler_with_tab
        result = _sync_run(handler.dispatch("interact", {
            "tab_id": tab_id,
            "action": "fill",
            "params": {"locator": "input[name=q]", "locator_type": "css", "value": "test"},
        }))
        assert result["status"] == "ok"

    def test_interact_press(self, handler_with_tab):
        handler, tab_id = handler_with_tab
        result = _sync_run(handler.dispatch("interact", {
            "tab_id": tab_id,
            "action": "press",
            "params": {"key": "Enter"},
        }))
        assert result["status"] == "ok"

    def test_interact_press_no_key(self, handler_with_tab):
        handler, tab_id = handler_with_tab
        result = _sync_run(handler.dispatch("interact", {
            "tab_id": tab_id,
            "action": "press",
            "params": {},
        }))
        assert result["status"] == "error"

    def test_interact_select(self, handler_with_tab):
        handler, tab_id = handler_with_tab
        result = _sync_run(handler.dispatch("interact", {
            "tab_id": tab_id,
            "action": "select",
            "params": {"locator": "select#lang", "value": "en"},
        }))
        assert result["status"] == "ok"

    def test_interact_scroll(self, handler_with_tab):
        handler, tab_id = handler_with_tab
        result = _sync_run(handler.dispatch("interact", {
            "tab_id": tab_id,
            "action": "scroll",
            "params": {"direction": "down", "amount": 500},
        }))
        assert result["status"] == "ok"

    def test_interact_wait_selector(self, handler_with_tab):
        handler, tab_id = handler_with_tab
        result = _sync_run(handler.dispatch("interact", {
            "tab_id": tab_id,
            "action": "wait",
            "params": {"what": "selector", "value": "div.loaded"},
        }))
        assert result["status"] == "ok"

    def test_interact_unknown_action(self, handler_with_tab):
        handler, tab_id = handler_with_tab
        result = _sync_run(handler.dispatch("interact", {
            "tab_id": tab_id,
            "action": "hover",
            "params": {},
        }))
        assert result["status"] == "error"
        assert "unknown interact action" in result["error"]

    def test_interact_no_tab(self):
        handler, _ = _make_handler_with_mock_browser()
        result = _sync_run(handler.dispatch("interact", {
            "action": "click",
            "params": {"locator": "#btn"},
        }))
        assert result["status"] == "error"


# ---------------------------------------------------------------------------
# 7. Handler — evaluate
# ---------------------------------------------------------------------------

class TestHandlerEvaluate:

    @pytest.fixture
    def handler_with_tab(self):
        handler, page = _make_handler_with_mock_browser()
        result = _sync_run(handler.dispatch("open", {"url": "https://example.com"}))
        tab_id = result["data"]["tab_id"]
        return handler, tab_id

    def test_evaluate(self, handler_with_tab):
        handler, tab_id = handler_with_tab
        result = _sync_run(handler.dispatch("evaluate", {
            "tab_id": tab_id,
            "script": "() => document.title",
        }))
        assert result["status"] == "ok"

    def test_evaluate_no_script(self, handler_with_tab):
        handler, tab_id = handler_with_tab
        result = _sync_run(handler.dispatch("evaluate", {"tab_id": tab_id}))
        assert result["status"] == "error"
        assert "script is required" in result["error"]

    def test_evaluate_no_tab(self):
        handler, _ = _make_handler_with_mock_browser()
        result = _sync_run(handler.dispatch("evaluate", {
            "script": "() => 1",
        }))
        assert result["status"] == "error"


# ---------------------------------------------------------------------------
# 8. Handler — shutdown
# ---------------------------------------------------------------------------

class TestHandlerShutdown:

    def test_shutdown(self):
        handler, _ = _make_handler_with_mock_browser()
        handler._shutdown_event = asyncio.Event()
        result = _sync_run(handler.dispatch("shutdown", {}))
        assert result["status"] == "ok"
        assert handler._shutdown_requested is True
        assert handler._shutdown_event.is_set()


# ---------------------------------------------------------------------------
# 9. Event wiring — buffers populate correctly
# ---------------------------------------------------------------------------

class TestEventWiring:

    def test_console_event_goes_to_buffer(self):
        handler, page = _make_handler_with_mock_browser()
        result = _sync_run(handler.dispatch("open", {"url": "https://example.com"}))
        tab_id = result["data"]["tab_id"]
        tab = handler._tabs[tab_id]

        # Simulate a console event
        console_fn = page._listeners.get("console")
        assert console_fn is not None

        mock_msg = MagicMock()
        mock_msg.type = "log"
        mock_msg.text = "Hello from JS"
        mock_msg.location = "test.js:1"
        console_fn(mock_msg)

        snap = tab.console_buf.snapshot()
        assert len(snap) == 1
        assert snap[0]["text"] == "Hello from JS"
        assert snap[0]["type"] == "log"

    def test_response_event_goes_to_network_buffer(self):
        handler, page = _make_handler_with_mock_browser()
        result = _sync_run(handler.dispatch("open", {"url": "https://example.com"}))
        tab_id = result["data"]["tab_id"]
        tab = handler._tabs[tab_id]

        response_fn = page._listeners.get("response")
        assert response_fn is not None

        mock_resp = MagicMock()
        mock_resp.status = 200
        mock_resp.url = "https://example.com/api"
        mock_resp.request = MagicMock()
        mock_resp.request.method = "GET"
        response_fn(mock_resp)

        snap = tab.network_buf.snapshot()
        assert len(snap) == 1
        assert snap[0]["status"] == 200
        assert snap[0]["url"] == "https://example.com/api"

    def test_request_event_goes_to_event_buffer(self):
        handler, page = _make_handler_with_mock_browser()
        result = _sync_run(handler.dispatch("open", {"url": "https://example.com"}))
        tab_id = result["data"]["tab_id"]
        tab = handler._tabs[tab_id]

        request_fn = page._listeners.get("request")
        assert request_fn is not None

        mock_req = MagicMock()
        mock_req.url = "https://example.com/style.css"
        mock_req.method = "GET"
        mock_req.resource_type = "stylesheet"
        request_fn(mock_req)

        snap = tab.event_buf.snapshot()
        assert len(snap) == 1
        assert snap[0]["event"] == "request"
        assert snap[0]["url"] == "https://example.com/style.css"

    def test_popup_event(self):
        handler, page = _make_handler_with_mock_browser()
        result = _sync_run(handler.dispatch("open", {"url": "https://example.com"}))
        tab_id = result["data"]["tab_id"]
        tab = handler._tabs[tab_id]

        popup_fn = page._listeners.get("popup")
        assert popup_fn is not None

        mock_popup = MagicMock()
        mock_popup.url = "https://popup.example.com"
        popup_fn(mock_popup)

        snap = tab.event_buf.snapshot()
        assert any(e["event"] == "popup" for e in snap)

    def test_crash_event(self):
        handler, page = _make_handler_with_mock_browser()
        result = _sync_run(handler.dispatch("open", {"url": "https://example.com"}))
        tab_id = result["data"]["tab_id"]
        tab = handler._tabs[tab_id]

        crash_fn = page._listeners.get("crash")
        assert crash_fn is not None
        crash_fn()

        snap = tab.event_buf.snapshot()
        assert any(e["event"] == "crash" for e in snap)


# ---------------------------------------------------------------------------
# 10. Buffer overflow in console/network
# ---------------------------------------------------------------------------

class TestBufferLimits:
    """Console buffer caps at 200, network at 100."""

    def test_console_buffer_limit(self):
        buf = BoundedBuffer(CONSOLE_BUFFER_LIMIT)
        for i in range(500):
            buf.append({"i": i})
        snap = buf.snapshot()
        assert len(snap) == CONSOLE_BUFFER_LIMIT
        assert snap[0]["i"] == 500 - CONSOLE_BUFFER_LIMIT  # oldest retained

    def test_network_buffer_limit(self):
        from scrapion_agent.daemon import NETWORK_BUFFER_LIMIT
        buf = BoundedBuffer(NETWORK_BUFFER_LIMIT)
        for i in range(500):
            buf.append({"i": i})
        snap = buf.snapshot()
        assert len(snap) == NETWORK_BUFFER_LIMIT
        assert snap[0]["i"] == 500 - NETWORK_BUFFER_LIMIT


# ---------------------------------------------------------------------------
# 11. DaemonServer — auth token validation
# ---------------------------------------------------------------------------

@pytest.mark.skipif(IS_WINDOWS, reason="Unix sockets not available on Windows")
class TestDaemonServerAuth:
    """Test the auth protocol over a real (loopback) Unix socket."""

    def _make_server(self, tmp_path):
        from scrapion_agent.daemon import DaemonServer
        sock_path = str(tmp_path / "test.sock")
        token = "test-token-12345"
        server = DaemonServer(socket_path=sock_path, token=token)
        return server, sock_path, token

    @pytest.mark.asyncio
    async def test_auth_success_and_request(self, tmp_path):
        from scrapion_agent.daemon import DaemonServer

        server, sock_path, token = self._make_server(tmp_path)

        # Mock browser lifecycle
        server.handler._playwright = AsyncMock()
        server.handler._browser = AsyncMock()
        server.handler._context = AsyncMock()

        # Start the server
        server._server = await asyncio.start_unix_server(
            server._handle_client, path=sock_path,
        )

        # Connect as a client
        reader, writer = await asyncio.open_unix_connection(sock_path)

        # Send auth token
        writer.write((token + "\n").encode())
        await writer.drain()

        # Read auth response
        auth_line = await reader.readline()
        auth_resp = json.loads(auth_line)
        assert auth_resp["status"] == "ok"

        # Send a request
        request = {"id": "req-1", "action": "list", "params": {}}
        writer.write((json.dumps(request) + "\n").encode())
        await writer.drain()

        resp_line = await reader.readline()
        resp = json.loads(resp_line)
        assert resp["id"] == "req-1"
        assert resp["status"] == "ok"

        # Send shutdown
        shutdown_req = {"id": "req-2", "action": "shutdown", "params": {}}
        writer.write((json.dumps(shutdown_req) + "\n").encode())
        await writer.drain()
        await reader.readline()

        writer.close()
        await writer.wait_closed()
        server._server.close()
        await server._server.wait_closed()

    @pytest.mark.asyncio
    async def test_auth_rejects_wrong_token(self, tmp_path):
        from scrapion_agent.daemon import DaemonServer

        server, sock_path, token = self._make_server(tmp_path)
        server.handler._playwright = AsyncMock()
        server.handler._browser = AsyncMock()
        server.handler._context = AsyncMock()

        server._server = await asyncio.start_unix_server(
            server._handle_client, path=sock_path,
        )

        reader, writer = await asyncio.open_unix_connection(sock_path)

        # Send wrong token
        writer.write(("wrong-token\n").encode())
        await writer.drain()

        # Read auth response — should be error
        auth_line = await reader.readline()
        auth_resp = json.loads(auth_line)
        assert auth_resp["status"] == "error"
        assert "authentication failed" in auth_resp["error"]

        writer.close()
        await writer.wait_closed()
        server._server.close()
        await server._server.wait_closed()


# ---------------------------------------------------------------------------
# 12. DaemonServer — JSON framing
# ---------------------------------------------------------------------------

@pytest.mark.skipif(IS_WINDOWS, reason="Unix sockets not available on Windows")
class TestDaemonServerFraming:
    """Verify newline-delimited JSON request/response protocol."""

    def _make_server(self, tmp_path):
        from scrapion_agent.daemon import DaemonServer
        sock_path = str(tmp_path / "test.sock")
        token = "test-token-12345"
        server = DaemonServer(socket_path=sock_path, token=token)
        return server, sock_path, token

    @pytest.mark.asyncio
    async def test_malformed_json_handled(self, tmp_path):
        from scrapion_agent.daemon import DaemonServer

        server, sock_path, token = self._make_server(tmp_path)
        server.handler._playwright = AsyncMock()
        server.handler._browser = AsyncMock()
        server.handler._context = AsyncMock()

        server._server = await asyncio.start_unix_server(
            server._handle_client, path=sock_path,
        )

        reader, writer = await asyncio.open_unix_connection(sock_path)

        # Auth
        writer.write((token + "\n").encode())
        await writer.drain()
        await reader.readline()  # auth response

        # Send garbage
        writer.write(b"not json at all\n")
        await writer.drain()

        resp_line = await reader.readline()
        resp = json.loads(resp_line)
        assert resp["status"] == "error"
        assert "invalid JSON" in resp["error"]

        # Shutdown
        writer.write((json.dumps({"id": "x", "action": "shutdown", "params": {}}) + "\n").encode())
        await writer.drain()
        await reader.readline()

        writer.close()
        await writer.wait_closed()
        server._server.close()
        await server._server.wait_closed()


# ---------------------------------------------------------------------------
# 13. Daemon start/shutdown lifecycle (integration-style, mocked browser)
# ---------------------------------------------------------------------------

@pytest.mark.skipif(IS_WINDOWS, reason="Unix sockets not available on Windows")
class TestDaemonLifecycle:
    """Test the full lifecycle with mocked Playwright."""

    @pytest.mark.asyncio
    async def test_full_lifecycle(self, tmp_path):
        from scrapion_agent.daemon import DaemonServer

        sock_path = str(tmp_path / "lifecycle.sock")
        token = "lifecycle-token"
        server = DaemonServer(socket_path=sock_path, token=token)

        # Mock browser
        mock_playwright = AsyncMock()
        mock_browser = AsyncMock()
        mock_context = AsyncMock()
        mock_page = _make_mock_page()
        mock_context.new_page = AsyncMock(return_value=mock_page)

        # Start server in background
        async def _run_server():
            server.handler._playwright = mock_playwright
            server.handler._browser = mock_browser
            server.handler._context = mock_context
            server._server = await asyncio.start_unix_server(
                server._handle_client, path=sock_path,
            )
            # Wait for shutdown
            await server._shutdown_event.wait()
            # Stop
            if server._server:
                server._server.close()
                await server._server.wait_closed()

        task = asyncio.create_task(_run_server())
        await asyncio.sleep(0.1)  # Let server start

        # Connect, auth, open tab, list, close, shutdown
        reader, writer = await asyncio.open_unix_connection(sock_path)

        writer.write((token + "\n").encode())
        await writer.drain()
        await reader.readline()  # auth ok

        # open
        writer.write((json.dumps({"id": "1", "action": "open", "params": {"url": "https://example.com"}}) + "\n").encode())
        await writer.drain()
        resp = json.loads(await reader.readline())
        assert resp["status"] == "ok"
        tab_id = resp["data"]["tab_id"]

        # list
        writer.write((json.dumps({"id": "2", "action": "list", "params": {}}) + "\n").encode())
        await writer.drain()
        resp = json.loads(await reader.readline())
        assert resp["status"] == "ok"
        assert len(resp["data"]["tabs"]) == 1

        # close
        writer.write((json.dumps({"id": "3", "action": "close", "params": {"tab_id": tab_id}}) + "\n").encode())
        await writer.drain()
        resp = json.loads(await reader.readline())
        assert resp["status"] == "ok"

        # shutdown
        writer.write((json.dumps({"id": "4", "action": "shutdown", "params": {}}) + "\n").encode())
        await writer.drain()
        resp = json.loads(await reader.readline())
        assert resp["status"] == "ok"

        writer.close()
        await writer.wait_closed()
        await task


# ---------------------------------------------------------------------------
# 14. __main__.py integration — daemon subcommand detection
# ---------------------------------------------------------------------------

class TestMainDaemonDetection:

    def test_daemon_in_subcommands(self):
        from scrapion_agent.__main__ import _SUBCOMMANDS
        assert "daemon" in _SUBCOMMANDS

    def test_all_original_subcommands_still_present(self):
        from scrapion_agent.__main__ import _SUBCOMMANDS
        assert "page" in _SUBCOMMANDS
        assert "search" in _SUBCOMMANDS
        assert "screenshot" in _SUBCOMMANDS
        assert "daemon" in _SUBCOMMANDS


# ---------------------------------------------------------------------------
# 15. Argument parsing
# ---------------------------------------------------------------------------

class TestDaemonArgs:
    """Test CLI argument parsing for --socket / --tcp-port."""

    def test_socket_and_tcp_port_mutually_exclusive(self):
        with pytest.raises(SystemExit):
            with patch("sys.argv", ["prog", "--socket", "/tmp/s.sock", "--tcp-port", "0"]):
                parse_args()

    def test_neither_socket_nor_tcp_port_fails(self):
        with pytest.raises(SystemExit):
            with patch("sys.argv", ["prog"]):
                parse_args()

    def test_tcp_port_zero(self):
        with patch("sys.argv", ["prog", "--tcp-port", "0"]):
            args = parse_args()
            assert args.tcp_port == 0
            assert args.socket is None

    def test_socket_only(self):
        with patch("sys.argv", ["prog", "--socket", "/tmp/s.sock"]):
            args = parse_args()
            assert args.socket == "/tmp/s.sock"
            assert args.tcp_port is None


# ---------------------------------------------------------------------------
# 16. DaemonServer — TCP transport (platform-independent)
# ---------------------------------------------------------------------------

class TestDaemonServerTCP:
    """Test auth and request/response over a TCP loopback connection."""

    @pytest.mark.asyncio
    async def test_auth_success_and_request(self):
        """Start DaemonServer on TCP, authenticate, send request, shutdown."""
        server = DaemonServer(token="tcp-token-abc", tcp_port=0)
        # Mock browser lifecycle
        server.handler._playwright = AsyncMock()
        server.handler._browser = AsyncMock()
        server.handler._context = AsyncMock()

        # Start the TCP server
        server._server = await asyncio.start_server(
            server._handle_client,
            host="127.0.0.1",
            port=0,
        )
        actual_port = server._server.sockets[0].getsockname()[1]

        try:
            # Connect as a client via TCP
            reader, writer = await asyncio.open_connection("127.0.0.1", actual_port)

            # Send auth token
            writer.write(b"tcp-token-abc\n")
            await writer.drain()

            # Read auth response
            auth_line = await reader.readline()
            auth_resp = json.loads(auth_line)
            assert auth_resp["status"] == "ok"

            # Send a request
            request = {"id": "req-1", "action": "list", "params": {}}
            writer.write((json.dumps(request) + "\n").encode())
            await writer.drain()

            resp_line = await reader.readline()
            resp = json.loads(resp_line)
            assert resp["id"] == "req-1"
            assert resp["status"] == "ok"

            # Send shutdown
            shutdown_req = {"id": "req-2", "action": "shutdown", "params": {}}
            writer.write((json.dumps(shutdown_req) + "\n").encode())
            await writer.drain()
            await reader.readline()

            writer.close()
            await writer.wait_closed()
        finally:
            server._server.close()
            await server._server.wait_closed()

    @pytest.mark.asyncio
    async def test_auth_rejects_wrong_token(self):
        """Wrong token over TCP should be rejected."""
        server = DaemonServer(token="correct-token", tcp_port=0)
        server.handler._playwright = AsyncMock()
        server.handler._browser = AsyncMock()
        server.handler._context = AsyncMock()

        server._server = await asyncio.start_server(
            server._handle_client,
            host="127.0.0.1",
            port=0,
        )
        actual_port = server._server.sockets[0].getsockname()[1]

        try:
            reader, writer = await asyncio.open_connection("127.0.0.1", actual_port)

            # Send wrong token
            writer.write(b"wrong-token\n")
            await writer.drain()

            # Read auth response — should be error
            auth_line = await reader.readline()
            auth_resp = json.loads(auth_line)
            assert auth_resp["status"] == "error"
            assert "authentication failed" in auth_resp["error"]

            writer.close()
            await writer.wait_closed()
        finally:
            server._server.close()
            await server._server.wait_closed()

    @pytest.mark.asyncio
    async def test_malformed_json_handled(self):
        """Malformed JSON over TCP should return error response."""
        server = DaemonServer(token="test-token", tcp_port=0)
        server.handler._playwright = AsyncMock()
        server.handler._browser = AsyncMock()
        server.handler._context = AsyncMock()

        server._server = await asyncio.start_server(
            server._handle_client,
            host="127.0.0.1",
            port=0,
        )
        actual_port = server._server.sockets[0].getsockname()[1]

        try:
            reader, writer = await asyncio.open_connection("127.0.0.1", actual_port)

            # Auth
            writer.write(b"test-token\n")
            await writer.drain()
            await reader.readline()  # auth response

            # Send garbage
            writer.write(b"not json at all\n")
            await writer.drain()

            resp_line = await reader.readline()
            resp = json.loads(resp_line)
            assert resp["status"] == "error"
            assert "invalid JSON" in resp["error"]

            # Shutdown
            writer.write((json.dumps({"id": "x", "action": "shutdown", "params": {}}) + "\n").encode())
            await writer.drain()
            await reader.readline()

            writer.close()
            await writer.wait_closed()
        finally:
            server._server.close()
            await server._server.wait_closed()

    @pytest.mark.asyncio
    async def test_full_lifecycle_tcp(self):
        """Full lifecycle over TCP: auth, open, list, close, shutdown."""
        server = DaemonServer(token="lifecycle-tcp", tcp_port=0)

        # Mock browser
        mock_context = AsyncMock()
        mock_page = _make_mock_page()
        mock_context.new_page = AsyncMock(return_value=mock_page)

        server.handler._playwright = AsyncMock()
        server.handler._browser = AsyncMock()
        server.handler._context = mock_context

        server._server = await asyncio.start_server(
            server._handle_client,
            host="127.0.0.1",
            port=0,
        )
        actual_port = server._server.sockets[0].getsockname()[1]

        task = asyncio.create_task(server.run())
        await asyncio.sleep(0.05)

        try:
            reader, writer = await asyncio.open_connection("127.0.0.1", actual_port)

            writer.write(b"lifecycle-tcp\n")
            await writer.drain()
            await reader.readline()  # auth ok

            # open
            writer.write((json.dumps({"id": "1", "action": "open", "params": {"url": "https://example.com"}}) + "\n").encode())
            await writer.drain()
            resp = json.loads(await reader.readline())
            assert resp["status"] == "ok"
            tab_id = resp["data"]["tab_id"]

            # list
            writer.write((json.dumps({"id": "2", "action": "list", "params": {}}) + "\n").encode())
            await writer.drain()
            resp = json.loads(await reader.readline())
            assert resp["status"] == "ok"
            assert len(resp["data"]["tabs"]) == 1

            # close
            writer.write((json.dumps({"id": "3", "action": "close", "params": {"tab_id": tab_id}}) + "\n").encode())
            await writer.drain()
            resp = json.loads(await reader.readline())
            assert resp["status"] == "ok"

            # shutdown
            writer.write((json.dumps({"id": "4", "action": "shutdown", "params": {}}) + "\n").encode())
            await writer.drain()
            resp = json.loads(await reader.readline())
            assert resp["status"] == "ok"

            writer.close()
            await writer.wait_closed()
            await task
        finally:
            if server._server and server._server.is_serving():
                server._server.close()
                await server._server.wait_closed()
