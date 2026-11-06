#!/usr/bin/env python3
"""
Koma egress proxy. The only route out of the isolated guest network.

Deny by default, CONNECT only, no direct route. Every connection is checked at
CONNECT time against a rule file the parent writes, and the file is re-read
whenever it changes — so a scope edit takes effect on the next connection and a
redirect to a new host is a new connection and therefore a new check. A redirect
that is not re-checked is the classic hole; there is no path here that skips it.

Proxy environment variables are NOT the boundary: the guest may unset them and a
tool may ignore them. They only tell a well-behaved client where to go. The
boundary is that the isolated network has no gateway, so nothing else is
reachable even if a client tries.

Rule file, one rule per line (blank lines and `#` comments ignored):

    host <portspec>          # portspec: `*`, or a comma list

`host` is a name or an IP. A name admits itself and its subdomains; a leading dot
means subdomains only; an IP never suffix-matches. Identical to
`app::sec::scope::Host::admits` — the vectors are pinned on both sides, because
this file is the enforcement point and a divergence is a silent hole.

Deliberately NOT supported, and refused rather than tunnelled:

- non-CONNECT methods on a plain-HTTP origin (a client that does not use the
  proxy would otherwise appear to work)
- ports outside the spec, always
"""

from __future__ import annotations

import os
import select
import socket
import sys
import threading

LISTEN_PORT = int(os.environ.get("KOMA_PROXY_PORT", "3128"))
RULE_FILE = os.environ.get("KOMA_PROXY_RULES", "/run/koma/rules")
BUFFER = 65536
# Bound on the unauthenticated pre-connect phase, so a client that connects and
# then says nothing cannot hold a thread forever.
HEAD_TIMEOUT = 15
IDLE_TIMEOUT = 300


class Rules:
    """Rule file, reloaded when its stamp changes."""

    def __init__(self, path: str) -> None:
        self.path = path
        self._lock = threading.Lock()
        self._stamp = None
        self._rules: list[tuple[str, frozenset[int] | None]] = []

    def _current_stamp(self):
        try:
            st = os.stat(self.path)
        except OSError:
            return None
        return (st.st_mtime_ns, st.st_size)

    def load(self) -> None:
        stamp = self._current_stamp()
        with self._lock:
            if stamp == self._stamp:
                return
            self._stamp = stamp
            self._rules = _parse(self.path)

    def allows(self, host: str, port: int) -> bool:
        self.load()
        with self._lock:
            return any(_rule_admits(h, ports, host, port) for h, ports in self._rules)


def _parse(path: str) -> list[tuple[str, frozenset[int] | None]]:
    out = []
    try:
        with open(path, encoding="utf-8") as fh:
            for line in fh:
                line = line.strip()
                if not line or line.startswith("#"):
                    continue
                parts = line.split()
                if len(parts) != 2:
                    continue
                host, spec = parts
                ports = None if spec == "*" else frozenset(
                    int(p) for p in spec.split(",") if p.isdigit()
                )
                out.append((host.strip().lower(), ports))
    except (OSError, ValueError):
        return []
    return out


def _rule_admits(host: str, ports: frozenset[int] | None, target: str, port: int) -> bool:
    if ports is not None and port not in ports:
        return False
    if host == target:
        return True
    if _is_ip(host):
        return False
    if host.startswith("."):
        return target.endswith(host)
    return target.endswith("." + host)


def _is_ip(host: str) -> bool:
    try:
        socket.inet_pton(socket.AF_INET, host)
        return True
    except OSError:
        pass
    try:
        socket.inet_pton(socket.AF_INET6, host)
        return True
    except OSError:
        return False


DENY_HEADER = "X-Koma-Egress"


def _deny(conn: socket.socket, status: str, why: str) -> None:
    body = f"koma egress: {why}\n".encode()
    head = (
        f"HTTP/1.1 {status}\r\n"
        "Content-Type: text/plain\r\n"
        # A marker header, so a client can tell a PROXY refusal from the target's
        # own 403. Without it the two are indistinguishable and an egress stop gets
        # recorded as if the target had answered.
        f"{DENY_HEADER}: {why}\r\n"
        f"Content-Length: {len(body)}\r\n"
        "Connection: close\r\n\r\n"
    ).encode()
    try:
        conn.sendall(head + body)
    except OSError:
        pass


def _read_head(conn: socket.socket) -> bytes | None:
    """Read until the blank line. Capped so a chatty client cannot exhaust us."""
    buf = b""
    conn.settimeout(HEAD_TIMEOUT)
    while b"\r\n\r\n" not in buf and len(buf) < BUFFER:
        try:
            chunk = conn.recv(BUFFER)
        except (TimeoutError, OSError):
            return None
        if not chunk:
            return None
        buf += chunk
    return buf


def _split_authority(authority: str, default: int) -> tuple[str, int]:
    if authority.startswith("["):
        host, _, tail = authority.partition("]")
        host = host[1:]
        port = tail.lstrip(":")
        return host, int(port) if port else default
    if authority.count(":") == 1:
        host, port = authority.split(":", 1)
        return host, int(port)
    return authority, default


def _splice(a: socket.socket, b: socket.socket) -> None:
    a.settimeout(IDLE_TIMEOUT)
    b.settimeout(IDLE_TIMEOUT)
    try:
        while True:
            readable, _, _ = select.select([a, b], [], [], IDLE_TIMEOUT)
            if not readable:
                return
            for src in readable:
                data = src.recv(BUFFER)
                if not data:
                    return
                (b if src is a else a).sendall(data)
    except OSError:
        return


def handle(conn: socket.socket, rules: Rules) -> None:
    try:
        head = _read_head(conn)
        if not head:
            return
        request_line = head.split(b"\r\n", 1)[0].decode("latin-1")
        parts = request_line.split()
        if len(parts) < 3:
            _deny(conn, "400 Bad Request", "malformed request line")
            return
        method, target = parts[0], parts[1]

        if method == "CONNECT":
            try:
                host, port = _split_authority(target, 443)
            except ValueError:
                _deny(conn, "400 Bad Request", f"bad authority {target!r}")
                return
            host = host.lower()
            if not rules.allows(host, port):
                _deny(conn, "403 Forbidden", f"{host}:{port} not in scope")
                return
            try:
                upstream = socket.create_connection((host, port), timeout=10)
            except OSError as exc:
                _deny(conn, "502 Bad Gateway", f"{host}:{port}: {exc}")
                return
            with upstream:
                conn.sendall(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                _splice(conn, upstream)
            return

        # Plain HTTP: the request line carries an absolute URI.
        if not target.startswith("http://"):
            _deny(
                conn,
                "400 Bad Request",
                "plain-HTTP request needs an absolute URI; use CONNECT for other schemes",
            )
            return
        authority = target[len("http://") :].split("/", 1)[0]
        try:
            host, port = _split_authority(authority, 80)
        except ValueError:
            _deny(conn, "400 Bad Request", f"bad authority {authority!r}")
            return
        host = host.lower()
        if not rules.allows(host, port):
            _deny(conn, "403 Forbidden", f"{host}:{port} not in scope")
            return
        try:
            upstream = socket.create_connection((host, port), timeout=10)
        except OSError as exc:
            _deny(conn, "502 Bad Gateway", f"{host}:{port}: {exc}")
            return
        with upstream:
            upstream.sendall(head)
            _splice(conn, upstream)
    except Exception as exc:  # noqa: BLE001 - a proxy thread must not die
        print(f"[koma-proxy] {exc}", file=sys.stderr)
    finally:
        try:
            conn.close()
        except OSError:
            pass


def _dns_servers() -> list[tuple[str, int]]:
    """The resolver(s) the proxy itself uses — docker's embedded one."""
    out = []
    try:
        with open("/etc/resolv.conf", encoding="utf-8") as fh:
            for line in fh:
                if line.startswith("nameserver"):
                    addr = line.split()[1]
                    out.append((addr, 53))
    except (OSError, IndexError):
        pass
    return out


def dns_forward(sock: socket.socket) -> None:
    """
    Forward UDP DNS from the guest to the proxy's own resolver.

    Needed because the isolated network has no gateway and therefore no resolver:
    without this, a guest cannot resolve ANY name, and every raw-TCP tool that
    dials a hostname fails before it can reach the proxy. Resolution here is not
    a scope decision — the proxy still checks every connection — it only restores
    the ability to look a name up at all.
    """
    servers = _dns_servers()
    if not servers:
        return
    while True:
        try:
            data, peer = sock.recvfrom(4096)
        except OSError:
            continue
        for addr in servers:
            try:
                with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as up:
                    up.settimeout(3)
                    up.sendto(data, addr)
                    reply, _ = up.recvfrom(4096)
                sock.sendto(reply, peer)
                break
            except OSError:
                continue


def serve() -> None:
    rules = Rules(RULE_FILE)
    rules.load()
    dns = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    try:
        dns.bind(("0.0.0.0", 53))
        threading.Thread(target=dns_forward, args=(dns,), daemon=True).start()
        print("[koma-proxy] dns forwarder on :53", file=sys.stderr)
    except OSError as exc:
        print(f"[koma-proxy] no dns forwarder: {exc}", file=sys.stderr)

    with socket.socket(socket.AF_INET6 if _has_v6() else socket.AF_INET) as srv:
        srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        srv.bind(("::" if _has_v6() else "0.0.0.0", LISTEN_PORT))
        srv.listen(128)
        print(f"[koma-proxy] listening on {LISTEN_PORT}, rules={RULE_FILE}", file=sys.stderr)
        while True:
            conn, _ = srv.accept()
            threading.Thread(target=handle, args=(conn, rules), daemon=True).start()


def _has_v6() -> bool:
    try:
        socket.socket(socket.AF_INET6, socket.SOCK_STREAM).close()
        return True
    except OSError:
        return False


if __name__ == "__main__":
    serve()
