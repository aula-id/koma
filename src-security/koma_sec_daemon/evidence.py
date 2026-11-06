"""
Evidence handles: bytes on disk that a finding can point at and re-resolve.

A verdict alone is not evidence. `confirmed` only stands if the handle it carries
still resolves to retained bytes, so confirmation and the artifact are checked
together rather than trusted apart.

Handles are opaque and traversal-safe, and the path is resolved from the store
root rather than stored absolute — so a moved session keeps its evidence.

Retention is bounded per store: oldest artifacts go first once the count or the
total size cap is reached, so a long run cannot fill the disk. Redaction is
best-effort and always leaves the raw bytes beside the redacted copy, because a
redaction that quietly dropped the original loses the thing worth keeping.
"""

from __future__ import annotations

import json
import re
from pathlib import Path

from koma_sec_daemon.board import safe_id

# Caps: count first (cheap), then total bytes. Sized for a long engagement.
MAX_ARTIFACTS = 500
MAX_BYTES = 200 * 1024 * 1024

# Patterns worth scrubbing before an artifact reaches a report. Ordered most
# specific first so a token is not half-eaten by a generic rule.
_SECRETS = (
    (re.compile(r"(?i)(authorization:\s*(?:bearer|basic)\s+)\S+"), r"\1[redacted]"),
    (re.compile(r"(?i)((?:set-)?cookie:\s*)\S+"), r"\1[redacted]"),
    (re.compile(r"\beyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]+"), "[jwt]"),
    (re.compile(r"\b(?:AKIA|ASIA)[A-Z0-9]{16}\b"), "[aws-key]"),
    (re.compile(r"\bgh[pousr]_[A-Za-z0-9]{20,}\b"), "[gh-token]"),
    (re.compile(r"\bsk-[A-Za-z0-9]{20,}\b"), "[sk-token]"),
    (re.compile(r"(?i)((?:api[_-]?key|password|passwd|secret|token)\"?\s*[:=]\s*\"?)[^\s\"',;]+"), r"\1[redacted]"),
)


class EvidenceError(RuntimeError):
    """Evidence could not be stored or resolved."""


def redact(text: str) -> str:
    """Scrub the shapes worth scrubbing. Never raises, never truncates."""
    for pattern, repl in _SECRETS:
        text = pattern.sub(repl, text)
    return text


class Store:
    """One evidence directory, with a manifest of handles."""

    def __init__(self, root: Path) -> None:
        self.root = Path(root)

    def _index(self) -> dict:
        try:
            return json.loads((self.root / "index.json").read_text())
        except (OSError, ValueError):
            return {}

    def _save_index(self, index: dict) -> None:
        self.root.mkdir(parents=True, exist_ok=True)
        p = self.root / "index.json"
        tmp = p.with_suffix(".tmp")
        tmp.write_text(json.dumps(index, sort_keys=True))
        tmp.replace(p)

    def put(self, label: str, body: str, *, provenance: str = "") -> str:
        """
        Retain `body` and return its handle.

        Both the raw and a redacted copy are kept. The raw is what re-resolution
        returns, because a redacted copy cannot answer "what did the target
        actually say".
        """
        handle = safe_id(label)
        self.root.mkdir(parents=True, exist_ok=True)
        raw = self.root / f"{handle}.raw"
        red = self.root / f"{handle}.txt"
        raw.write_text(body)
        red.write_text(redact(body))
        index = self._index()
        index[handle] = {
            "label": label,
            "provenance": provenance,
            "bytes": len(body),
            "redacted": redact(body) != body,
        }
        self._save_index(index)
        self._prune()
        return handle

    def resolve(self, handle: str) -> str:
        """The retained bytes. Raises [`EvidenceError`] when they are gone."""
        path = self.root / f"{safe_id(handle)}.raw"
        try:
            return path.read_text()
        except FileNotFoundError:
            raise EvidenceError(f"evidence {handle} no longer resolves") from None

    def resolves(self, handle: str) -> bool:
        try:
            self.resolve(handle)
        except EvidenceError:
            return False
        return True

    def redacted(self, handle: str) -> str:
        try:
            return (self.root / f"{safe_id(handle)}.txt").read_text()
        except FileNotFoundError:
            raise EvidenceError(f"evidence {handle} no longer resolves") from None

    def meta(self, handle: str) -> dict | None:
        return self._index().get(safe_id(handle))

    def _prune(self) -> None:
        """Drop the oldest artifacts past the count cap, then the size cap."""
        index = self._index()
        if len(index) <= MAX_ARTIFACTS and self._total(index) <= MAX_BYTES:
            return
        # Insertion order is the index's order, so the front is the oldest.
        keep = list(index)[-MAX_ARTIFACTS:]
        dropped = [h for h in index if h not in keep]
        for handle in dropped:
            index.pop(handle, None)
            for suffix in (".raw", ".txt"):
                try:
                    (self.root / f"{handle}{suffix}").unlink()
                except FileNotFoundError:
                    pass
        while self._total(index) > MAX_BYTES and index:
            oldest = next(iter(index))
            index.pop(oldest)
            for suffix in (".raw", ".txt"):
                try:
                    (self.root / f"{oldest}{suffix}").unlink()
                except FileNotFoundError:
                    pass
        self._save_index(index)

    @staticmethod
    def _total(index: dict) -> int:
        return sum(int(v.get("bytes", 0)) for v in index.values())
