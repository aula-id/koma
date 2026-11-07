"""Typed result verdicts: the daemon side of the phase-2 result contract.

Every operation returns exactly one verdict. The parent parses it with the
matching Rust enum, so the wire spelling here and `verdict.rs` must move
together.

`unavailable` and `error` are the two the daemon raises on its own:

- `unavailable` means a dependency is absent. The install hint rides along so the
  model can be told what to fetch, and `health.py` stays the source of truth for
  whether it really is missing.
- `error` is a crash or a refused dispatch — the tool ran and something broke.
  Kept distinct from `not_found`: "ran, nothing there" must not be reachable by
  the same path as "never got to run".

`untyped` is deliberately absent. A handler that returns a bare string is
wrapped as `{"verdict": "untyped"}` by `typed()`, so the model never sees a
sentinel and the parent never has to special-case a legacy shape.
"""

from __future__ import annotations

import json

# Closed set. Order is not meaningful; the parent's enum decides.
CONFIRMED = "confirmed"
NOT_FOUND = "not_found"
NEEDS_INPUT = "needs_input"
BLOCKED = "blocked"
TIMED_OUT = "timed_out"
UNAVAILABLE = "unavailable"
ERROR = "error"
UNTYPED = "untyped"
# Provenance verdicts. Kept distinct from NOT_FOUND, which says the target
# answered "no"; these say WHY no answer stands.
DISPROVED = "disproved"
UNPROVEN = "unproven"
UNTESTED = "untested"

VERDICTS = frozenset(
    {
        CONFIRMED,
        NOT_FOUND,
        NEEDS_INPUT,
        BLOCKED,
        TIMED_OUT,
        UNAVAILABLE,
        ERROR,
        UNTYPED,
        DISPROVED,
        UNPROVEN,
        UNTESTED,
    }
)

# Excerpt ceiling for the human-readable half of a typed result. Well under the
# parent's 8 KB shape guard so a typed result is never truncated by it; a payload
# larger than this belongs in a handle, not in the transcript.
EXCERPT_LIMIT = 2048


def typed(verdict: str, excerpt: str = "", handle: str | None = None) -> str:
    """
    Serialize one typed result. This is the ONLY shape a handler should return.

    `excerpt` is clipped at EXCERPT_LIMIT with an explicit marker, so the model can
    tell a complete answer from a truncated one. `handle` is where the full bytes
    went; without it a clipped excerpt is unrecoverable.
    """
    if verdict not in VERDICTS:
        raise ValueError(f"unknown verdict: {verdict}")
    if len(excerpt) > EXCERPT_LIMIT:
        cut = len(excerpt) - EXCERPT_LIMIT
        marker = f"\n…[{cut} more chars" + (f", full: {handle}" if handle else "") + "]"
        excerpt = excerpt[:EXCERPT_LIMIT] + marker
    frame = {"verdict": verdict, "excerpt": excerpt}
    if handle:
        frame["handle"] = handle
    return json.dumps(frame, ensure_ascii=False)


def unavailable(missing: str, hint: str = "") -> str:
    """A dependency is absent. `missing` names it; `hint` says how to get it."""
    return typed(UNAVAILABLE, f"{missing} is not installed" + (f" — {hint}" if hint else ""))


def error(message: str) -> str:
    """The tool ran and failed, or the dispatch refused it. Never `not_found`."""
    return typed(ERROR, message)


def is_typed(result: str) -> bool:
    """Does this result already carry a verdict?"""
    try:
        frame = json.loads(result)
    except (TypeError, ValueError):
        return False
    return isinstance(frame, dict) and frame.get("verdict") in VERDICTS


def as_typed(result: str) -> str:
    """
    Wrap a bare handler string as `untyped`, or pass an already-typed result
    through. Applied once at the `call` op boundary so no handler has to know
    whether its siblings were migrated yet.
    """
    if is_typed(result):
        return result
    return typed(UNTYPED, "" if result is None else str(result))
