"""
Board store: transactional records with OS locking and leased claims.

Two processes hit one board (a session and its sub-agents), so the store uses an
`flock` on a lock file for every read-modify-write. Atomic file replacement alone
is not enough: two processes can each read, each append, and the second silently
drops the first. The lock is released by the kernel when the holder dies, so a
crash cannot wedge the board.

Claims are leased, not absolute. A claim carries an expiry; a stale one is
recoverable by another process after it lapses, without a reaper having to run.
`renew` extends it — that is the heartbeat.

Records are versioned per item, not per file, so a writer only has to hold the
lock across the touched item and can still compare-and-set (`expect`).

Identifiers are traversal-safe: a slug plus a short digest, lowercased, with no
dots to climb out of the board directory.
"""

from __future__ import annotations

import errno
import fcntl
import json
import os
import re
import time
from pathlib import Path

# A claim is valid for this long before it is considered abandoned. Long enough to
# cover a slow tool, short enough that a crash does not block the route forever.
LEASE_SECONDS = 180

_SLUG = re.compile(r"[^a-z0-9]+")
# Bumped when the on-disk shape changes; a record from another version is refused
# and rewritten rather than mis-read.
SCHEMA = 1

KINDS = ("asset", "route", "hypothesis", "evidence", "claim", "coverage")


class BoardError(RuntimeError):
    """A board operation could not complete for a reason worth reporting."""


class StaleClaim(BoardError):
    """Someone else holds a live claim on this route."""

    def __init__(self, holder: str, expires: float) -> None:
        super().__init__(f"claimed by {holder} until {expires:.0f}")
        self.holder = holder
        self.expires = expires


class StaleWrite(BoardError):
    """The record moved since the caller read it (`expect` did not match)."""


# An id that already carries its digest: `<slug>-<8 hex>`, no dots.
_SAFE = re.compile(r"^[a-z0-9]+(?:-[a-z0-9]+)*-[0-9a-f]{8}$")


def safe_id(text: str) -> str:
    """
    A traversal-safe identifier.

    Anything outside `[a-z0-9-]` collapses to `-`, and a trailing digest keeps
    `a/b` and `a-b` from colliding after the collapse. No dots means no `..`.

    IDEMPOTENT: an id that already carries a digest is returned unchanged. Without
    that, every layer that re-hashes (`read`, `resolve`, `meta`) shifts the key and
    the record becomes unreachable — a bug that is invisible until something reads
    back what it just wrote.
    """
    import hashlib

    if _SAFE.match(text):
        return text
    slug = _SLUG.sub("-", text.lower()).strip("-")[:60]
    digest = hashlib.sha256(text.encode("utf-8")).hexdigest()[:8]
    return f"{slug or 'x'}-{digest}"


class Board:
    """One board directory. Cheap to construct; the lock is per operation."""

    def __init__(self, root: Path) -> None:
        self.root = Path(root)
        self._lock_path = self.root / ".lock"

    def _locked(self, fn):
        """Run `fn` holding the exclusive lock."""
        self.root.mkdir(parents=True, exist_ok=True)
        with open(self._lock_path, "a+") as fh:
            fcntl.flock(fh, fcntl.LOCK_EX)
            try:
                return fn()
            finally:
                fcntl.flock(fh, fcntl.LOCK_UN)

    def _path(self, kind: str, ident: str) -> Path:
        """Path for an ALREADY-safe id. Callers with a domain id apply `safe_id`."""
        if kind not in KINDS:
            raise BoardError(f"unknown record kind: {kind}")
        return self.root / kind / f"{ident}.json"

    def read(self, kind: str, ident: str) -> dict | None:
        """Read a record by its domain identifier."""
        try:
            raw = self._path(kind, safe_id(ident)).read_text()
        except FileNotFoundError:
            return None
        try:
            rec = json.loads(raw)
        except ValueError:
            return None
        return rec if rec.get("schema") == SCHEMA else None

    def list(self, kind: str) -> list[dict]:
        out = []
        try:
            paths = sorted((self.root / kind).glob("*.json"))
        except FileNotFoundError:
            return out
        for p in paths:
            try:
                rec = json.loads(p.read_text())
            except (OSError, ValueError):
                continue
            if rec.get("schema") == SCHEMA:
                out.append(rec)
        return out

    def put(self, kind: str, ident: str, body: dict, *, expect: int | None = None) -> dict:
        """
        Create or replace a record, bumping its version.

        `expect` is a compare-and-set: pass the version you read. A mismatch raises
        [`StaleWrite`] instead of clobbering another writer's update.
        """

        def apply() -> dict:
            path = self._path(kind, safe_id(ident))
            current = self.read(kind, ident)
            if expect is not None:
                got = (current or {}).get("version")
                if got != expect:
                    raise StaleWrite(f"{ident}: expected v{expect}, found {got}")
            rec = {
                "schema": SCHEMA,
                "id": safe_id(ident),
                "kind": kind,
                "version": (current or {}).get("version", 0) + 1,
                "body": body,
            }
            path.parent.mkdir(parents=True, exist_ok=True)
            tmp = path.with_suffix(".tmp")
            tmp.write_text(json.dumps(rec, sort_keys=True))
            tmp.replace(path)
            return rec

        return self._locked(apply)

    # --- claims -------------------------------------------------------------

    def claim(self, route: str, owner: str, *, lease: float = LEASE_SECONDS) -> dict:
        """
        Claim a route, or raise [`StaleClaim`] when another live claim exists.

        A lapsed claim is won outright: that is the crash-recovery path, and it has
        to work without a reaper.
        """
        ident = safe_id(route)

        def apply() -> dict:
            now = time.time()
            current = self.read("claim", route)
            if current:
                body = current["body"]
                if body.get("owner") != owner and body.get("expires", 0) > now:
                    raise StaleClaim(body.get("owner", "?"), body["expires"])
                if body.get("owner") == owner:
                    # Re-claiming your own route is a renewal.
                    return self._write_claim(ident, owner, now + lease, current)
            return self._write_claim(ident, owner, now + lease, current)

        return self._locked(apply)

    def _write_claim(self, ident: str, owner: str, expires: float, current: dict | None) -> dict:
        # `ident` is ALREADY safe: `_path` does not re-hash, so every caller here
        # must have applied `safe_id` first.
        rec = {
            "schema": SCHEMA,
            "id": ident,
            "kind": "claim",
            "version": (current or {}).get("version", 0) + 1,
            "body": {"route": ident, "owner": owner, "expires": expires},
        }
        path = self._path("claim", ident)
        path.parent.mkdir(parents=True, exist_ok=True)
        tmp = path.with_suffix(".tmp")
        tmp.write_text(json.dumps(rec, sort_keys=True))
        tmp.replace(path)
        return rec

    def renew(self, route: str, owner: str, *, lease: float = LEASE_SECONDS) -> dict:
        """Extend a held claim. Raises [`StaleClaim`] if it went to someone else."""

        def apply() -> dict:
            now = time.time()
            current = self.read("claim", route)
            body = (current or {}).get("body") or {}
            if body.get("owner") != owner and body.get("expires", 0) > now:
                raise StaleClaim(body.get("owner", "?"), body["expires"])
            return self._write_claim(safe_id(route), owner, now + lease, current)

        return self._locked(apply)

    def release(self, route: str, owner: str) -> bool:
        """Drop a claim. False when it was not yours (so nothing was freed)."""

        def apply() -> bool:
            ident = safe_id(route)
            current = self.read("claim", route)
            if not current:
                return False
            if current["body"].get("owner") != owner:
                return False
            try:
                self._path("claim", ident).unlink()
            except FileNotFoundError:
                return False
            return True

        return self._locked(apply)

    def is_claimed(self, route: str) -> tuple[bool, str | None]:
        """Is a LIVE claim held on this route, and by whom?"""
        # `read` hashes its argument, so the route must go in raw exactly once.
        rec = self.read("claim", route)
        if not rec:
            return False, None
        body = rec["body"]
        if body.get("expires", 0) <= time.time():
            return False, body.get("owner")
        return True, body.get("owner")


def dispatch_gate(b: Board, route: str, owner: str) -> str | None:
    """
    The pre-dispatch duplicate check. `None` means go; a string is the refusal.

    Called BEFORE a tool runs, so the check is part of dispatch rather than an
    instruction in a prompt: two lanes given the same route cannot both fire.
    Re-entering your own live claim is allowed — that is the normal retry case.
    """
    held, holder = b.is_claimed(route)
    if held and holder != owner:
        return (
            f"route {route} is already claimed by {holder}; "
            "read the board before firing again"
        )
    try:
        b.claim(route, owner)
    except StaleClaim as exc:
        return f"route {route} is claimed ({exc})"
    return None


def default_root(session_dir: Path) -> Path:
    """Where a board lives: under the parent session, beside its other state."""
    return Path(session_dir) / "board"


def _errno_is_lock_contention(exc: OSError) -> bool:
    return exc.errno in (errno.EAGAIN, errno.EACCES)


def present_in(root: Path) -> bool:
    return os.path.isdir(root)
