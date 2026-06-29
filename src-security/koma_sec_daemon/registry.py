"""
Tool registry — central index of all available security tools.

Each entry in REGISTRY is a descriptor dict:
  {
    "name"        : str,
    "description" : str,
    "parameters"  : dict (JSON Schema object),
    "risk"        : bool,
    "compute"     : "instant-cpu"|"long-cpu"|"gpu"|"network"|"executes-target",
    "domain"      : str,
    "handler"     : callable(args: dict, sessions) -> str,
  }

Only descriptors() (which strips the handler) is safe to send over the wire.
"""

from __future__ import annotations

from koma_sec_daemon.tools.http import DESCRIPTOR as _HTTP_DESC
from koma_sec_daemon.tools.remote import DESCRIPTOR as _REMOTE_DESC

# Wire-safe keys — everything except the callable handler
_WIRE_KEYS = ("name", "description", "parameters", "risk", "compute", "domain")

REGISTRY: dict[str, dict] = {
    _HTTP_DESC["name"]: _HTTP_DESC,
    _REMOTE_DESC["name"]: _REMOTE_DESC,
}


def descriptors() -> list[dict]:
    """Return all tool descriptors with the handler key stripped (wire-safe)."""
    return [
        {k: d[k] for k in _WIRE_KEYS if k in d}
        for d in REGISTRY.values()
    ]


def call(name: str, args: dict, sessions) -> str:
    """
    Look up *name* in the registry and invoke its handler.

    Returns the string result.
    Raises KeyError (with the tool name) if the tool is unknown.
    Any exception from the handler propagates as-is (so a handler's internal
    KeyError — e.g. a missing required arg — is NOT mis-reported as unknown tool).
    """
    # Keep the REGISTRY lookup in its own try/except so only a missing-tool
    # KeyError is caught here; handler exceptions propagate to the caller.
    try:
        descriptor = REGISTRY[name]
    except KeyError:
        raise KeyError(name)
    handler = descriptor["handler"]
    # Call the handler OUTSIDE the try block so any KeyError it raises
    # (e.g. args["url"] on a missing arg) propagates as a plain exception,
    # not as an "unknown tool" false positive.
    return handler(args, sessions)
