"""
Findings: the claim a run makes, and what has to be true for it to stand.

Two things are enforced rather than trusted:

- A `confirmed` finding needs at least one handle that still RESOLVES. A tool
  verdict is not enough on its own, and neither is a handle that has since been
  pruned away — so confirmation and the bytes are checked together.
- Untested is distinct from unproven. A route never attempted is not a negative
  result, and collapsing the two is how a gap reads as a pass.

The report keeps those five buckets apart for the same reason.
"""

from __future__ import annotations

from koma_sec_daemon import evidence

# Buckets a report separates. `untested` is the one most often lost.
BUCKETS = ("confirmed", "unproven", "disproved", "blocked", "untested")

# Findings that may NOT be reported as confirmed without resolving evidence.
_NEEDS_EVIDENCE = ("confirmed",)


class NotConfirmed(RuntimeError):
    """A finding was called confirmed without evidence that resolves."""


class Finding:
    """One claim. Sensitive `detail` may be redacted; the handle is the proof."""

    __slots__ = ("id", "status", "detail", "handles", "because")

    def __init__(self, ident: str, status: str, detail: str = "", handles=None, because: str = "") -> None:
        if status not in BUCKETS:
            raise ValueError(f"unknown bucket: {status}")
        self.id = ident
        self.status = status
        self.detail = detail
        self.handles: list[str] = list(handles or ())
        self.because = because

    def resolves_all(self, store: evidence.Store) -> bool:
        return all(store.resolves(h) for h in self.handles)

    def check(self, store: evidence.Store) -> None:
        """Raise [`NotConfirmed`] when this finding overclaims."""
        if self.status not in _NEEDS_EVIDENCE:
            return
        if not self.handles:
            raise NotConfirmed(f"{self.id}: confirmed with no evidence handle")
        missing = [h for h in self.handles if not store.resolves(h)]
        if missing:
            raise NotConfirmed(f"{self.id}: evidence no longer resolves: {', '.join(missing)}")

    def as_dict(self, store: evidence.Store | None = None) -> dict:
        out = {
            "id": self.id,
            "status": self.status,
            "detail": self.detail,
            "handles": self.handles,
            "because": self.because,
        }
        if store is not None:
            out["resolves"] = self.resolves_all(store)
        return out


def report(findings: list[Finding], store: evidence.Store) -> dict:
    """
    Group findings into the five buckets, keeping unresolved ones visible.

    A finding whose evidence no longer resolves is moved to `unproven` and marked,
    rather than being dropped: silently losing it would look like it was never
    made.
    """
    out: dict[str, list[dict]] = {b: [] for b in BUCKETS}
    for f in findings:
        if f.status == "confirmed" and not f.resolves_all(store):
            row = f.as_dict(store)
            row["demoted"] = True
            out["unproven"].append(row)
            continue
        out[f.status].append(f.as_dict(store))
    return out
