"""
sec_remote — stateful pwntools remote session tool.

compute : executes-target
risk    : True
domain  : pwn

pwntools (pwn) is imported LAZILY inside the handler so the registry loads
even when pwntools is not installed (import-time safety for the smoke test).

Actions
-------
open      {host, port}                → open a TCP connection, return session id
send      {session, data}             → raw send (no newline)
sendline  {session, data}             → send data + newline
recv      {session, n?, until?}       → receive bytes; until= recvuntil, else recv(n)
close     {session}                   → close and remove the session
"""

from __future__ import annotations

_RECV_CAP = 100_000

DESCRIPTOR = {
    "name": "sec_remote",
    "description": (
        "Stateful pwntools TCP session. Use action='open' to connect, then "
        "send/sendline/recv to exchange data, and close when done."
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "action": {
                "type": "string",
                "enum": ["open", "send", "sendline", "recv", "close"],
                "description": "Which operation to perform.",
            },
            "host": {
                "type": "string",
                "description": "Hostname or IP (required for action=open).",
            },
            "port": {
                "type": "integer",
                "description": "TCP port (required for action=open).",
            },
            "session": {
                "type": "string",
                "description": "Session id returned by action=open (required for all other actions).",
            },
            "data": {
                "type": "string",
                "description": "Data string to send (required for send/sendline).",
            },
            "n": {
                "type": "integer",
                "description": "Max bytes to receive (action=recv, default 4096).",
            },
            "until": {
                "type": "string",
                "description": "Receive until this delimiter string (action=recv).",
            },
        },
        "required": ["action"],
    },
    "risk": True,
    "compute": "executes-target",
    "domain": "pwn",
}


def _handler(args: dict, sessions) -> str:
    action = args.get("action")

    if action == "open":
        # Lazy import of pwntools
        from pwn import remote  # noqa: PLC0415

        host = args["host"]
        port = int(args["port"])
        try:
            r = remote(host, port, timeout=10)
        except Exception as exc:
            return f"error: {exc}"
        sid = sessions.open(r)
        return f"session {sid} opened to {host}:{port}"

    elif action == "send":
        sid = args["session"]
        data = args["data"]
        try:
            conn = sessions.get(sid)
            conn.send(data.encode())
        except Exception as exc:
            return f"error: {exc}"
        return f"sent {len(data)} bytes"

    elif action == "sendline":
        sid = args["session"]
        data = args["data"]
        try:
            conn = sessions.get(sid)
            conn.sendline(data.encode())
        except Exception as exc:
            return f"error: {exc}"
        return f"sent {len(data)} bytes (with newline)"

    elif action == "recv":
        sid = args["session"]
        try:
            conn = sessions.get(sid)
            if "until" in args and args["until"] is not None:
                raw = conn.recvuntil(args["until"].encode(), timeout=10)
            else:
                n = int(args.get("n") or 4096)
                raw = conn.recv(n, timeout=10)
        except Exception as exc:
            return f"error: {exc}"
        text = raw.decode("utf-8", errors="replace")
        if len(text) > _RECV_CAP:
            text = text[:_RECV_CAP] + "\n…[recv truncated]"
        return text

    elif action == "close":
        sid = args["session"]
        try:
            sessions.close(sid)
        except Exception as exc:
            return f"error: {exc}"
        return "session closed"

    else:
        return f"error: unknown action '{action}'"


# Attach handler to descriptor for registry use
DESCRIPTOR["handler"] = _handler
