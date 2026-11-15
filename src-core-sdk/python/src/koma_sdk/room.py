"""The room handle: one adapter bound to one live room, with typed events and recovery by history."""
from __future__ import annotations

import asyncio
import re
from collections.abc import AsyncIterator, Sequence
from typing import TYPE_CHECKING, Any

from .errors import DeliveryUnknown, EventOverflow, IncompatibleAdapter, TransportError
from .room_types import (AttachResult, AvatarRemoveResult, CloseReason, CloseResult, HistoryPage, RoomClosedEvent,
                         RoomEvent, RoomGapEvent, RoomLineEvent, RoomSpend, RoomStatus, RoomStatusEvent, ReportPage,
                         SendResult, SentResult, ShelfItem, ShelfPage, Team, TeamWriteResult, Trajectory)
from .types import Attachment, Hello

if TYPE_CHECKING:
    from .client import Client, _Wire

_OMIT: Any = object()
_ACTIONS = ("pause", "resume", "stop", "redirect")
_AVATAR_ID = re.compile(r"^[a-z0-9][a-z0-9_-]{0,47}$")
_TARGETS = ("template", "room")


def _envelope(request_id: str, method: str, params: dict[str, Any]) -> dict[str, Any]:
    return {"id": request_id, "method": method, "params": params}


def _bounded(name: str, value: Any, low: int, high: int) -> int:
    if type(value) is not int or not low <= value <= high:
        raise ValueError(f"{name} must be an integer from {low} to {high}")
    return value


def _string(name: str, value: Any, low: int, high: int) -> str:
    if not isinstance(value, str) or not low <= len(value) <= high:
        raise ValueError(f"{name} must be a string of {low} to {high} characters")
    return value


def _paging(params: dict[str, Any], limit: int | None, cursor: str | None, most: int) -> dict[str, Any]:
    if limit is not None:
        params["limit"] = _bounded("limit", limit, 1, most)
    if cursor is not None:
        params["cursor"] = _string("cursor", cursor, 1, 4096)
    return params


def attach_params(first: str, value: str, tail: int | None, wait_ms: int | None, wait_max: int) -> dict[str, Any]:
    params: dict[str, Any] = {first: _string(first, value, 1, 64)}
    if tail is not None:
        params["tail"] = _bounded("tail", tail, 0, 200)
    if wait_ms is not None:
        params["wait_ms"] = _bounded("wait_ms", wait_ms, 0, wait_max)
    return params


def team_target(target: str | None) -> str | None:
    if target is not None and target not in _TARGETS:
        raise ValueError("target must be 'template' or 'room'")
    return target


def _if_revision(params: dict[str, Any], if_revision: str | None) -> dict[str, Any]:
    if if_revision is not None:
        params["if_revision"] = _string("if_revision", if_revision, 1, 64)
    return params


def _effort(value: Any) -> str:
    if (not isinstance(value, str) or not 1 <= len(value) <= 64
            or any(c.isspace() or ord(c) < 0x20 or ord(c) == 0x7f for c in value)):
        raise ValueError("effort must be 1 to 64 characters without whitespace or control characters")
    return value


def models_set_params(target: str, avatar: str, lane: str, model: Any, effort: Any,
                      if_revision: str | None) -> dict[str, Any]:
    """`model` or `effort` omitted keeps the layer's value; `None` clears it so the layer below applies."""
    params: dict[str, Any] = {"target": team_target(target), "avatar": _string("avatar", avatar, 1, 64)}
    if lane not in ("chat", "work"):
        raise ValueError("lane must be 'chat' or 'work'")
    params["lane"] = lane
    if model is _OMIT and effort is _OMIT:
        raise ValueError("give model or effort")
    if model is not _OMIT:
        params["model"] = None if model is None else _string("model", model, 1, 512)
    if effort is not _OMIT:
        params["effort"] = None if effort is None else _effort(effort)
    return _if_revision(params, if_revision)


def avatars_upsert_params(target: str, avatar_id: str, role: Any, display: Any, alias: Any,
                          if_revision: str | None) -> dict[str, Any]:
    """`display` or `alias` omitted keeps the layer's value; `None` clears it. A role cannot be cleared."""
    if not isinstance(avatar_id, str) or not _AVATAR_ID.match(avatar_id):
        raise ValueError("id must match [a-z0-9][a-z0-9_-]{0,47}")
    params: dict[str, Any] = {"target": team_target(target), "id": avatar_id}
    if role is _OMIT and display is _OMIT and alias is _OMIT:
        raise ValueError("give role, display or alias")
    if role is not _OMIT:
        params["role"] = _string("role", role, 1, 64)
    if display is not _OMIT:
        params["display"] = None if display is None else _string("display", display, 1, 64)
    if alias is not _OMIT:
        if alias is not None and (not isinstance(alias, str) or not 1 <= len(alias) <= 3
                                  or not alias.isascii() or not alias.isalnum()):
            raise ValueError("alias must be 1 to 3 ASCII letters or digits")
        params["alias"] = alias
    return _if_revision(params, if_revision)


def avatars_remove_params(target: str, avatar_id: str, if_revision: str | None) -> dict[str, Any]:
    return _if_revision({"target": team_target(target), "id": _string("id", avatar_id, 1, 64)}, if_revision)


class Room:
    """One adapter bound to one room. Obtain it from `Client.room_open` or `Client.room_attach`.

    The room has exactly one live subscriber: opening or attaching takes the view over from any other
    client, the terminal included. When another client takes it over from you, `events()` yields a
    `RoomClosedEvent(reason=TAKEN_OVER)` and ends; the handle never re-attaches by itself. Call `reattach()`
    to take the room back. Reads (`history`, `shelf_list`, `item`, ...) keep working while the binding is closed.

    Writes to the room are never retried automatically. A write whose outcome is unknown raises
    `DeliveryUnknown`; read `history()` and `status()` before deciding anything.
    """
    def __init__(self, client: Client, transport: _Wire, hello: Hello, attached: AttachResult):
        self._client, self._transport, self.hello = client, transport, hello
        self._attached = attached
        self._requests = 1
        self._event_lock = asyncio.Lock()
        self._seen_epoch: str | None = None
        self._last = 0

    @property
    def attached(self) -> AttachResult:
        """The newest attach result: epoch, tail, roster, status and the room's daemon features."""
        return self._attached

    @property
    def session_id(self) -> str:
        return self._attached.room.session_id

    @property
    def epoch(self) -> str:
        return self._transport.room_epoch or self._attached.epoch

    @property
    def closed_reason(self) -> CloseReason | None:
        """Why the binding closed, or `None` while it is live. Set as soon as the adapter reports it."""
        return self._transport.room_closed

    @property
    def live(self) -> bool:
        return self._transport.room_closed is None and self._transport.failure is None and not self._transport._closed

    @property
    def stderr_tail(self) -> str:
        return self._transport.stderr.decode("utf-8", errors="replace")

    @property
    def requests_remaining(self) -> int:
        """Requests this adapter still accepts. At 0, open a fresh `Room` with `Client.room_attach`."""
        return max(0, self.hello.request_ids - self._requests)

    async def __aenter__(self) -> Room:
        return self

    async def __aexit__(self, *exc: Any) -> None:
        await self.aclose()

    async def _request(self, method: str, params: dict[str, Any], *, mutation: bool,
                       feature: str | None = None, timeout: float | None = None) -> dict[str, Any]:
        if not self.hello.supports(method):
            raise IncompatibleAdapter(f"adapter does not support {method}")
        if feature is not None and not self.hello.has_feature(feature):
            raise IncompatibleAdapter(f"adapter lacks feature {feature}")
        if self.requests_remaining == 0:
            raise TransportError("adapter request-id limit reached; attach again from a fresh adapter")
        self._requests += 1
        try:
            return await self._transport.request(method, params, mutation=mutation, envelope=_envelope,
                                                 timeout=timeout or self._client.request_timeout)
        except DeliveryUnknown as exc:
            exc.session_id = self.session_id
            raise

    # Binding

    async def reattach(self, *, tail: int | None = None, wait_ms: int | None = None) -> AttachResult:
        """Take the room back (`room.attach`) after the binding closed. Starts a new epoch with a fresh ring."""
        params = attach_params("session_id", self.session_id, tail, wait_ms, 15000)
        return await self._rebind("room.attach", params, wait_ms or 2000)

    async def reopen(self, *, tail: int | None = None, wait_ms: int | None = None) -> AttachResult:
        """Start the room again (`room.open` on its parent) after it exited. Starts a new epoch."""
        parent = self._attached.room.parent_session_id
        if parent is None:
            raise ValueError("the room has no known parent session; use Client.room_open")
        params = attach_params("parent_session_id", parent, tail, wait_ms, 30000)
        return await self._rebind("room.open", params, wait_ms or 12000)

    async def _rebind(self, method: str, params: dict[str, Any], wait_ms: int) -> AttachResult:
        result = await self._request(method, params, mutation=True,
                                     timeout=max(self._client.request_timeout, wait_ms / 1000 + 10))
        self._attached = AttachResult.from_wire(result)
        return self._attached

    async def close(self, *, stop: bool = False) -> CloseResult:
        """Release the room (`room.close`) and close the adapter. `stop=True` also stops the room's daemon.

        Use `aclose()` to drop the connection without that request.
        """
        try:
            return CloseResult.from_wire(await self._request("room.close", {"stop": stop} if stop else {},
                                                             mutation=True))
        finally:
            await self.aclose()

    async def aclose(self) -> None:
        """Close the adapter connection. It tells the room goodbye; the room itself keeps running."""
        await self._transport.close()
        self._client._transports.discard(self._transport)

    # Writes to the room

    async def send(self, text: str, *, to: str | None = None, shell: bool = False,
                   attachments: Sequence[Attachment] = ()) -> SendResult:
        """Send a message. `to` addresses one avatar, `shell=True` runs `text` as a shell line.

        `acked` in the result is false for a room that cannot acknowledge; such a room refuses attachments.
        The adapter echoes the line as a `RoomLineEvent` with `origin=ADAPTER_ECHO` before this returns.
        """
        if not isinstance(text, str) or not text.strip() or len(text.encode("utf-8")) > 2 * 1024 * 1024:
            raise ValueError("text must be nonblank and at most 2 MiB UTF-8")
        if type(shell) is not bool:
            raise ValueError("shell must be a boolean")
        if len(attachments) > 16:
            raise ValueError("at most 16 attachments")
        if shell and (to is not None or attachments):
            raise ValueError("shell excludes to and attachments")
        params: dict[str, Any] = {"text": text}
        if to is not None:
            params["to"] = _string("to", to, 1, 64)
        if shell:
            params["shell"] = True
        if attachments:
            params["attachments"] = [a.to_wire() for a in attachments]
        return SendResult.from_wire(await self._request("room.send", params, mutation=True))

    async def interrupt(self) -> SentResult:
        """Stop every chat lane. Work lanes continue."""
        return SentResult.from_wire(await self._request("room.interrupt", {}, mutation=True))

    async def control(self, action: str, avatar: str, *, note: str | None = None) -> SentResult:
        """`pause`, `resume`, `stop` or `redirect` (which needs `note`) an avatar's work lane."""
        if action not in _ACTIONS:
            raise ValueError(f"action must be one of {', '.join(_ACTIONS)}")
        params: dict[str, Any] = {"action": action, "avatar": _string("avatar", avatar, 1, 64)}
        if (action == "redirect") != (note is not None):
            raise ValueError("note is required for redirect and allowed only there")
        if note is not None:
            params["note"] = _string("note", note, 1, 8192)
        return SentResult.from_wire(await self._request("room.control", params, mutation=True))

    # Reads

    async def status(self) -> RoomStatus:
        """The latest cached status. `room.status` events carry the same value; do not poll."""
        return RoomStatus.from_wire(await self._request("room.status", {}, mutation=False))

    async def history(self, *, epoch: str | None = None, after_line_seq: int | None = None,
                      before_line_seq: int | None = None, limit: int | None = None,
                      kinds: Sequence[str] | None = None, author: str | None = None) -> HistoryPage:
        """A page of the adapter's line ring (the room's replay plus live lines). At most one bound.

        Without a bound it returns the newest `limit` lines. `epoch` defaults to the current one; a call with
        an epoch that is no longer current fails with `reason == "epoch_changed"`.
        """
        if after_line_seq is not None and before_line_seq is not None:
            raise ValueError("give at most one of after_line_seq and before_line_seq")
        params: dict[str, Any] = {"epoch": self.epoch if epoch is None else _string("epoch", epoch, 1, 64)}
        for key, value in (("after_line_seq", after_line_seq), ("before_line_seq", before_line_seq)):
            if value is not None:
                params[key] = _bounded(key, value, 0, 9007199254740991)
        if limit is not None:
            params["limit"] = _bounded("limit", limit, 1, 200)
        if kinds is not None:
            if isinstance(kinds, str) or not all(isinstance(k, str) and k for k in kinds):
                raise ValueError("kinds must be a list of line kinds")
            params["kinds"] = list(kinds)
        if author is not None:
            params["author"] = _string("author", author, 1, 64)
        return HistoryPage.from_wire(await self._request("room.history", params, mutation=False))

    async def shelf_list(self, *, prefix: str | None = None, limit: int | None = None,
                         cursor: str | None = None) -> ShelfPage:
        params: dict[str, Any] = {}
        if prefix is not None:
            params["prefix"] = _string("prefix", prefix, 0, 200)
        return ShelfPage.from_wire(await self._request("room.shelf.list", _paging(params, limit, cursor, 500),
                                                       mutation=False))

    async def item(self, key: str) -> ShelfItem:
        return ShelfItem.from_wire(await self._request("room.item", {"key": _string("key", key, 1, 512)},
                                                       mutation=False))

    async def trajectory(self, goal: str | None = None) -> Trajectory:
        params = {} if goal is None else {"goal": _string("goal", goal, 1, 512)}
        return Trajectory.from_wire(await self._request("room.trajectory", params, mutation=False))

    async def reports(self, *, include_markdown: bool = False, limit: int | None = None,
                      cursor: str | None = None) -> ReportPage:
        if type(include_markdown) is not bool:
            raise ValueError("include_markdown must be a boolean")
        params: dict[str, Any] = {"include_markdown": True} if include_markdown else {}
        return ReportPage.from_wire(await self._request("room.reports", _paging(params, limit, cursor, 100),
                                                        mutation=False))

    async def spend(self, goal: str | None = None) -> RoomSpend:
        params = {} if goal is None else {"goal": _string("goal", goal, 1, 512)}
        return RoomSpend.from_wire(await self._request("room.spend", params, mutation=False))

    # Team

    async def team_get(self, target: str | None = None) -> Team:
        """The effective team. `target` defaults to `room`; `template` reads `~/.koma/room/`."""
        params = {} if target is None else {"target": team_target(target)}
        return Team.from_wire(await self._request("room.team.get", params, mutation=False,
                                                  feature="room.template" if target == "template" else None))

    async def models_set(self, avatar: str, lane: str, *, model: Any = _OMIT, effort: Any = _OMIT,
                         target: str = "room", if_revision: str | None = None) -> TeamWriteResult:
        """Set an avatar lane's model and/or effort in the `room` or `template` layer. `None` clears it."""
        params = models_set_params(target, avatar, lane, model, effort, if_revision)
        return TeamWriteResult.from_wire(await self._team_write("room.models.set", params))

    async def avatars_upsert(self, avatar_id: str, *, role: Any = _OMIT, display: Any = _OMIT, alias: Any = _OMIT,
                             target: str = "room", if_revision: str | None = None) -> TeamWriteResult:
        """Add an avatar (needs `role`) or change an existing one's `display` and `alias` in a layer."""
        params = avatars_upsert_params(target, avatar_id, role, display, alias, if_revision)
        return TeamWriteResult.from_wire(await self._team_write("room.avatars.upsert", params))

    async def avatars_remove(self, avatar_id: str, *, target: str = "room",
                             if_revision: str | None = None) -> AvatarRemoveResult:
        """Remove the layer's own entry for an avatar; a base avatar cannot be removed."""
        params = avatars_remove_params(target, avatar_id, if_revision)
        return AvatarRemoveResult.from_wire(await self._team_write("room.avatars.remove", params))

    async def _team_write(self, method: str, params: dict[str, Any]) -> dict[str, Any]:
        return await self._request(method, params, mutation=True,
                                   feature="room.template" if params["target"] == "template" else None)

    # Events

    def _sync_epoch(self) -> None:
        t = self._transport
        if self._seen_epoch != t.room_epoch:
            self._seen_epoch, self._last = t.room_epoch, t.room_floor

    async def events(self) -> AsyncIterator[RoomEvent]:
        """Typed events of the current epoch, single consumer.

        Lines arrive once, in `line_seq` order. After a `RoomGapEvent`, or when the client-side queue
        overflowed, the missed lines are fetched with `history()` and yielded as `RoomLineEvent(recovered=True)`,
        followed by a fresh `RoomStatusEvent`; the state of a coding session is never involved. Events of an
        earlier epoch are never yielded. The iterator ends after a `RoomClosedEvent`.
        """
        if self._event_lock.locked():
            raise RuntimeError("only one events iterator may consume a room")
        async with self._event_lock:
            t = self._transport
            while True:
                try:
                    raw = await t.next_event()
                except EventOverflow:
                    async for recovered in self._recover(overflow=True):
                        yield recovered
                    continue
                event = raw.get("_room")
                if event is None or event.epoch != t.room_epoch:
                    continue
                self._sync_epoch()
                if isinstance(event, RoomLineEvent):
                    if event.line.line_seq <= self._last:
                        continue
                    self._last = event.line.line_seq
                    yield event
                elif isinstance(event, RoomGapEvent):
                    yield event
                    async for recovered in self._recover(overflow=False):
                        yield recovered
                elif isinstance(event, RoomClosedEvent):
                    yield event
                    return
                else:
                    yield event

    async def _recover(self, *, overflow: bool) -> AsyncIterator[RoomEvent]:
        t = self._transport
        if overflow and not t.restart_events():
            return
        self._sync_epoch()
        epoch = self._seen_epoch
        if epoch is None:
            return
        while True:
            page = await self.history(epoch=epoch, after_line_seq=self._last, limit=200)
            for line in page.lines:
                if line.line_seq > self._last:
                    self._last = line.line_seq
                    yield RoomLineEvent(t.sequence, epoch, line, True)
            if not page.has_more_after or not page.lines:
                break
        yield RoomStatusEvent(t.sequence, epoch, await self.status())
        if overflow:
            try:
                # Room events carry no revision: keep everything queued; `events()` drops lines already seen.
                t.finish_recovery(-1)
            except EventOverflow:
                pass
