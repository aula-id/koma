"""Transport failures are distinct from adapter operation failures."""
from __future__ import annotations


class SDKError(Exception):
    """Base SDK exception."""


class TransportError(SDKError):
    """Adapter transport is unavailable or malformed."""


class ProtocolError(TransportError):
    pass


class IncompatibleAdapter(ProtocolError):
    pass


class EventOverflow(TransportError):
    """The bounded event inbox overflowed. Restart event consumption to reconcile state."""


class RequestTimeout(TransportError):
    pass


class DeliveryUnknown(TransportError):
    """A mutation was written but its result is unknown; NEVER replay blindly."""

    def __init__(self, request_id: str, method: str, *, session_id: str | None = None,
                 turn_id: str | None = None, cause: BaseException | None = None):
        self.request_id = request_id
        self.method = method
        self.session_id = session_id
        self.turn_id = turn_id
        self.cause = cause
        super().__init__(f"Delivery unknown for {method} ({request_id}); reconcile state before retrying")


class RemoteError(SDKError):
    """An adapter error. Branch on `reason` (an open string), never on `message`.

    `reason`, `binding`, `allowed`, `written` and `detail` come from adapters with the
    `errors.reason` feature; v1 errors leave them `None`, `()`, `False` and `None`.
    An unknown `reason` is handled by its `code` and delivery.
    """

    def __init__(self, code: str, message: str, data: object = None):
        self.code = code
        self.message = message
        self.data = data
        extra = data if isinstance(data, dict) else {}
        reason, binding, allowed = extra.get("reason"), extra.get("binding"), extra.get("allowed")
        detail = extra.get("detail")
        self.reason: str | None = reason if isinstance(reason, str) else None
        self.binding: str | None = binding if isinstance(binding, str) else None
        self.allowed: tuple[str, ...] = (tuple(allowed) if isinstance(allowed, list)
                                         and all(isinstance(a, str) for a in allowed) else ())
        self.written: bool = extra.get("written") is True
        self.detail: dict[str, object] | None = detail if isinstance(detail, dict) else None
        super().__init__(f"{code}: {message}")
