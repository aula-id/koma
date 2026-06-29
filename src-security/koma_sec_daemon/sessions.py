"""
SessionStore — holds live objects (e.g. pwntools remote()) keyed by a
deterministic, incrementing session id (s0, s1, …).

IDs are deterministic on purpose: no random/uuid, so logs and replays are
predictable and easy to grep.
"""
from __future__ import annotations


class SessionStore:
    def __init__(self) -> None:
        self._store: dict[str, object] = {}
        self._n: int = 0

    def open(self, obj) -> str:
        """Store *obj* under a new id and return that id."""
        sid = f"s{self._n}"
        self._n += 1
        self._store[sid] = obj
        return sid

    def get(self, sid: str):
        """Return the object stored under *sid*. Raises KeyError if missing."""
        return self._store[sid]

    def close(self, sid: str) -> None:
        """Remove *sid* from the store and best-effort close the underlying object."""
        obj = self._store.pop(sid)
        try:
            obj.close()
        except Exception:
            pass

    def close_all(self) -> None:
        """Close every open session — called on daemon shutdown."""
        for sid in list(self._store):
            self.close(sid)
