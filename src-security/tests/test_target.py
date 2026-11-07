"""A throwaway HTTP target for tool tests: redirects, slow responses, auth, 5xx."""
from __future__ import annotations

import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *_: object) -> None:  # silence
        pass

    def _send(self, code: int, body: str = "", headers: dict | None = None) -> None:
        raw = body.encode()
        self.send_response(code)
        for k, v in (headers or {}).items():
            self.send_header(k, v)
        self.send_header("Content-Length", str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)

    def do_GET(self) -> None:  # noqa: N802 - stdlib naming
        path = self.path.split("?")[0]
        if path == "/":
            self._send(200, "<a href=\"/other\">other</a> root")
        elif path == "/redirect":
            self._send(302, "", {"Location": "/landed"})
        elif path == "/redirect-away":
            self._send(302, "", {"Location": "http://127.0.0.1:1/x"})
        elif path == "/landed":
            self._send(200, "landed")
        elif path == "/slow":
            time.sleep(2)
            self._send(200, "slow")
        elif path == "/boom":
            self._send(503, "unavailable")
        elif path == "/secret":
            self._send(200, "ok", {"Set-Cookie": "sid=abc123", "Authorization": "Bearer s3cr3t"})
        elif path == "/reflect":
            # Echoes the query back VERBATIM — would execute a payload.
            from urllib.parse import parse_qsl, urlsplit  # noqa: PLC0415

            q = dict(parse_qsl(urlsplit(self.path).query))
            self._send(200, f"<p>{q.get('q', '')}</p>")
        elif path == "/reflect-enc":
            # Echoes it HTML-escaped: reflected but inert. The distinction a
            # version-only detector misses.
            import html  # noqa: PLC0415
            from urllib.parse import parse_qsl, urlsplit  # noqa: PLC0415

            q = dict(parse_qsl(urlsplit(self.path).query))
            self._send(200, f"<p>{html.escape(q.get('q', ''))}</p>")
        elif path == "/big":
            self._send(200, "<p>" + "x" * 200_000 + "</p>")
        else:
            self._send(404, "missing")

    def do_POST(self) -> None:  # noqa: N802 - stdlib naming
        length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(length).decode()
        self._send(200, f"got:{body}")


class Target:
    """Context manager around a server on an ephemeral port."""

    def __init__(self) -> None:
        self.srv = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.port = self.srv.server_address[1]
        self.thread = threading.Thread(target=self.srv.serve_forever, daemon=True)

    @property
    def base(self) -> str:
        return f"http://127.0.0.1:{self.port}"

    def __enter__(self) -> "Target":
        self.thread.start()
        return self

    def __exit__(self, *_: object) -> None:
        self.srv.shutdown()
        self.srv.server_close()
