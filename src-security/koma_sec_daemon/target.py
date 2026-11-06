"""Stable identity for a dynamic target.

Two targets can be `localhost:8080` in different sessions and on different
checkouts. A port alone is not identity: the board, the scope record and every
evidence handle key on this fingerprint, so it has to carry the run's context —
session, cwd, verb — not just the host.

Three properties, all load-bearing:

- **Stable.** Same run context and same target ⇒ byte-identical, so a repeated
  call lands on the same record. Built from a sorted, fixed-order list, never from
  a dict's iteration order.
- **Legible.** The verb, host and path stay readable so a human and the model can
  both recognise it; only the context is hashed. The readable half is slugified,
  so `:8080` appears as `-8080` and a `/` as `-`.
- **Bounded.** The cwd is hashed to one digest rather than embedded. A deep
  checkout path would otherwise blow the identifier apart and leak the tree
  layout into every card.

`session` and `cwd` are inputs, never read from the environment here, so the
fingerprint can be computed and tested without a live session.
"""

from __future__ import annotations

import re

# Verb prefixes a cyber tool name carries. Closed, because an unknown prefix means
# the caller is guessing and the fingerprint would be wrong silently.
VERBS = ("red", "blue", "purple", "cyber")

_SLUG = re.compile(r"[^a-z0-9]+")


def _slug(text: str, limit: int = 40) -> str:
    return _SLUG.sub("-", text.lower()).strip("-")[:limit]


def _digest(text: str, n: int = 10) -> str:
    """
    64-bit FNV-1a, truncated. Matches `tool/scope.rs::digest` exactly.

    Deliberately NOT SHA-256: this is a placement key, not a security boundary,
    and the two languages must produce the same value or every record forks. A
    cross-language test pins the vector `1cd4fc81c8` for `/work/repo`.
    """
    h = 0xCBF29CE484222325
    for byte in text.encode("utf-8"):
        h ^= byte
        h = (h * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return format(h, "016x")[:n]


def verb_of(tool: str) -> str | None:
    """The verb a tool name carries (`op_red_scan` → `red`), or None."""
    head = tool.split("_", 1)[0]
    return head if head in VERBS else None


def fingerprint(
    *,
    session: str,
    cwd: str,
    verb: str,
    host: str,
    path: str = "",
    port: int | None = None,
) -> str:
    """
    Build the identifier. Deterministic for the same inputs.

    `path` and `port` are part of identity because `:8080` and `:8443` on one host
    are different targets, and so is `/api` versus `/admin`.
    """
    if verb not in VERBS:
        raise ValueError(f"unknown verb: {verb}")
    if not session:
        raise ValueError("session is required — two sessions on one checkout collide without it")
    if not cwd:
        raise ValueError("cwd is required — a target is only meaningful against a run's tree")

    head = f"{verb}:{host.strip().lower()}"
    if port is not None:
        head += f":{port}"
    if path:
        head += "/" + path.lstrip("/")

    # Sorted, fixed order. Adding a field is a format change: bump FORMAT.
    material = "\n".join(
        ("v1", f"session={session}", f"cwd={_digest(cwd)}", f"target={head}")
    )
    return f"{_slug(head, 60)}@{_digest(material)}"


def record_filename(*, host: str, port: int | None = None) -> str:
    """
    The file name `tool/scope.rs::record_path` produces for the same target.

    Kept here so the two halves can be asserted equal in one place instead of the
    Rust test re-deriving the slug rule.
    """
    name = host.strip().lower()
    if port is not None:
        name += f"-{port}"
    return _slug(name, 60) + ".json"


def store_name(*, session: str, cwd: str, verb: str, host: str, port: int | None = None) -> str:
    """
    Where the run's dynamic target state lives relative to the session dir.

    Keyed by the same fingerprint so a re-resolve of the same host in the same
    session reuses the file instead of forking a second one.
    """
    return f"{fingerprint(session=session, cwd=cwd, verb=verb, host=host, port=port)}.json"
