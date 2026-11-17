"""The room handle: one adapter bound to one live room, with typed events and recovery by history."""
from __future__ import annotations

import asyncio
import base64
import os
import re
from collections.abc import AsyncIterator, Mapping, Sequence
from typing import TYPE_CHECKING, Any

from .errors import DeliveryUnknown, EventOverflow, IncompatibleAdapter, TransportError
from .room_types import (AttachResult, AvatarRemoveResult, CloseReason, CloseResult, HistoryPage, RoomClosedEvent,
                         RoomEvent, RoomGapEvent, RoomLineEvent, RoomPage, RoomSpend, RoomStatus, RoomStatusEvent,
                         ReportPage, SendResult, SentResult, ShelfItem, ShelfPage, Team, TeamWriteResult, Trajectory)
from .types import (Attachment, Hello, SessionDeleteResult, SessionPage, SessionRenameResult, SessionStopResult,
                    SessionTranscript, SkillDetail, SkillFile, SkillPage, SkillRemoveResult, SkillUpdateResult, SkillWriteResult,
                    UsageReport)

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


def _id_param(name: str, value: str) -> str:
    if not isinstance(value, str) or not 1 <= len(value) <= 64:
        raise ValueError(f"{name} must be a string of 1 to 64 characters")
    return value


def _text_param(name: str, value: object, most: int, *, by_bytes: bool = False) -> str:
    if not isinstance(value, str) or not value or (len(value.encode("utf-8")) if by_bytes else len(value)) > most:
        raise ValueError(f"{name} must be a nonempty string of at most {most} {'bytes' if by_bytes else 'characters'}")
    return value


def _required_tier(tier: object) -> str:
    if not isinstance(tier, str) or not tier:
        raise ValueError("tier is required to change an existing skill")
    return tier


def _skill_patch(patch: Mapping[str, Any]) -> dict[str, Any]:
    if not isinstance(patch, Mapping) or not patch:
        raise ValueError("patch must be a nonempty mapping")
    unknown = set(patch) - {"name", "description", "triggers", "allowed_tools", "instruction"}
    if unknown:
        raise ValueError(f"unknown patch fields: {', '.join(sorted(unknown))}")
    out: dict[str, Any] = {}
    if "name" in patch:
        out["name"] = _skill_name(patch["name"], rule=True)
    if "description" in patch:
        out["description"] = _text_param("description", patch["description"], 2000)
    if "instruction" in patch:
        out["instruction"] = _text_param("instruction", patch["instruction"], 2 * 1024 * 1024, by_bytes=True)
    if "triggers" in patch:
        out["triggers"] = _skill_text_list("triggers", patch["triggers"], 32, 200)
    if "allowed_tools" in patch:
        out["allowed_tools"] = _skill_text_list("allowed_tools", patch["allowed_tools"], 64, 100)
    return out


INLINE_ZIP_BYTES = 29_360_128
_SKILL_NAME = re.compile(r"^[a-z0-9]+(-[a-z0-9]+)*$")
_NEW_SKILL_SCOPES = ("computer", "project")
_USAGE_GROUPS = ("none", "model", "role", "session")
_USAGE_BUCKETS = ("none", "hour", "day", "week")


def _display_name(name: object) -> str:
    """A session name as the adapter validates it: no control characters (checked before trimming), then
    trimmed to 1 to 200 characters."""
    if not isinstance(name, str):
        raise ValueError("name must be a string")
    if any(ord(c) < 0x20 or ord(c) == 0x7f for c in name):
        raise ValueError("name must not contain control characters")
    name = name.strip()
    if not 1 <= len(name) <= 200:
        raise ValueError("name must be 1 to 200 characters")
    return name


def _int_in(name: str, value: object, low: int, high: int) -> int:
    if type(value) is not int or not low <= value <= high:
        raise ValueError(f"{name} must be an integer from {low} to {high}")
    return value


def _list_params(params: dict[str, Any], limit: int | None, cursor: str | None, most: int) -> dict[str, Any]:
    if limit is not None:
        params["limit"] = _int_in("limit", limit, 1, most)
    if cursor is not None:
        if not isinstance(cursor, str) or not 1 <= len(cursor) <= 4096:
            raise ValueError("cursor must be a string from a previous page")
        params["cursor"] = cursor
    return params


def _absolute(name: str, value: object) -> str:
    if not isinstance(value, str) or not os.path.isabs(value):
        raise ValueError(f"{name} must be an absolute path")
    return value


def _skill_name(name: object, *, rule: bool) -> str:
    """`rule=True` applies the stricter name rule for new skills; existing on-disk names are addressed as they are."""
    if not isinstance(name, str) or not 1 <= len(name.encode("utf-8")) <= 64:
        raise ValueError("skill name must be 1 to 64 bytes")
    if rule and (not _SKILL_NAME.match(name) or name in ("skill", "readme")):
        raise ValueError("skill name must be lowercase letters, digits and single hyphens, and not skill or readme")
    return name


def _skill_text_list(name: str, value: object, most: int, longest: int) -> list[str]:
    if (not isinstance(value, (list, tuple)) or len(value) > most
            or not all(isinstance(v, str) and 1 <= len(v) <= longest for v in value)):
        raise ValueError(f"{name} must be a list of at most {most} strings of 1 to {longest} characters")
    return list(value)


def _skill_address(params: dict[str, Any], tier: str | None, workdir: str | None) -> dict[str, Any]:
    if tier is not None:
        if not isinstance(tier, str) or not tier:
            raise ValueError("tier must be a string")
        params["tier"] = tier
    if workdir is not None:
        params["workdir"] = _absolute("workdir", workdir)
    return params


def inline_zip(data: bytes | bytearray | memoryview, *, max_bytes: int = INLINE_ZIP_BYTES) -> str:
    """Base64 text for `Host.skills_install(data_b64=...)`, refusing a zip over the inline cap before encoding.

    The cap is `hello.limits["skills"]["inline_zip_bytes"]` (28 MiB); `Host.skills_install(data=...)` applies
    the adapter's own. A larger zip goes by `path=`, which the adapter opens itself (up to 64 MiB).
    """
    if not isinstance(data, (bytes, bytearray, memoryview)):
        raise ValueError("zip data must be bytes")
    if len(data) > max_bytes:
        raise ValueError(f"zip is {len(data)} bytes; the inline limit is {max_bytes}. Install it by path instead")
    return base64.b64encode(bytes(data)).decode("ascii")


class HostCalls:
    """Calls that work in every binding of an adapter: sessions, usage, skills, rooms on disk and the room template.

    `Host`, `Session` and `Room` share them; each sends them on its own adapter.
    """
    hello: Hello | None

    async def _call(self, method: str, params: dict[str, Any], *, mutation: bool,
                    feature: str | None = None) -> dict[str, Any]:
        raise NotImplementedError

    async def sessions_list(self, *, kind: str = "all", live: bool | None = None, limit: int | None = None,
                            cursor: str | None = None) -> SessionPage:
        """Sessions, newest first. `kind` is `coding`, `room` or `all`. Pass `next_cursor` back to page on."""
        if kind not in ("coding", "room", "all"):
            raise ValueError("kind must be 'coding', 'room' or 'all'")
        params: dict[str, Any] = {"kind": kind}
        if live is not None:
            if type(live) is not bool:
                raise ValueError("live must be a boolean")
            params["live"] = live
        return SessionPage.from_wire(await self._call("sessions.list", _list_params(params, limit, cursor, 500),
                                                      mutation=False))

    async def sessions_rename(self, session_id: str, name: str) -> SessionRenameResult:
        """Rename a room or a session that is not live. Another live coding session is refused (`reason == "live"`)."""
        params = {"session_id": _id_param("session_id", session_id), "name": _display_name(name)}
        return SessionRenameResult.from_wire(await self._call("sessions.rename", params, mutation=True))

    async def sessions_delete(self, session_id: str) -> SessionDeleteResult:
        """Delete a session that is neither live nor locked. Usage rows and a child room are kept."""
        return SessionDeleteResult.from_wire(await self._call(
            "sessions.delete", {"session_id": _id_param("session_id", session_id)}, mutation=True))

    async def sessions_stop(self, session_id: str) -> SessionStopResult:
        """Stop a session's daemon. Every client of it is disconnected; a parked approval is lost."""
        return SessionStopResult.from_wire(await self._call(
            "sessions.stop", {"session_id": _id_param("session_id", session_id)}, mutation=True))

    async def sessions_transcript(self, session_id: str, *, since: int = 0,
                                  limit: int | None = None) -> SessionTranscript:
        """The stored messages of a coding session, without starting its daemon. Secrets are scrubbed.

        Pass the last message `id` you saw as `since` to read on; `next_since` is set when more follow.
        """
        params: dict[str, Any] = {"session_id": _id_param("session_id", session_id)}
        if since != 0:
            params["since"] = _int_in("since", since, 0, 9007199254740991)
        if limit is not None:
            params["limit"] = _int_in("limit", limit, 1, 500)
        return SessionTranscript.from_wire(await self._call("sessions.transcript", params, mutation=False))

    async def usage_query(self, *, from_at: int | None = None, to_at: int | None = None,
                          session_id: str | None = None, workdir: str | None = None, group_by: str = "none",
                          bucket: str = "none", tz_offset_minutes: int | None = None,
                          limit: int | None = None) -> UsageReport:
        """Usage over `[from_at, to_at)` in unix seconds (default: the last 30 days), optionally grouped or bucketed.

        Poll no more often than a screen needs: a query holds a read lock on the ledger while it runs.
        """
        params: dict[str, Any] = {}
        for key, value in (("from_at", from_at), ("to_at", to_at)):
            if value is not None:
                params[key] = _int_in(key, value, 0, 9007199254740991)
        if from_at is not None and to_at is not None and to_at <= from_at:
            raise ValueError("to_at must be greater than from_at")
        if session_id is not None:
            params["session_id"] = _id_param("session_id", session_id)
        if workdir is not None:
            params["workdir"] = _absolute("workdir", workdir)
        if group_by not in _USAGE_GROUPS or bucket not in _USAGE_BUCKETS:
            raise ValueError("group_by is none, model, role or session; bucket is none, hour, day or week")
        if group_by != "none":
            params["group_by"] = group_by
        if bucket != "none":
            params["bucket"] = bucket
        if tz_offset_minutes is not None:
            params["tz_offset_minutes"] = _int_in("tz_offset_minutes", tz_offset_minutes, -720, 840)
        if limit is not None:
            params["limit"] = _int_in("limit", limit, 1, 500)
        return UsageReport.from_wire(await self._call("usage.query", params, mutation=False))

    async def skills_list(self, *, scope: str = "all", tier: str | None = None, workdir: str | None = None,
                          include_diagnostics: bool = False, limit: int | None = None,
                          cursor: str | None = None) -> SkillPage:
        """Skills of every tier, shadowed ones included (`effective` tells the winner). Project tiers need `workdir`."""
        if scope not in ("computer", "project", "external", "all"):
            raise ValueError("scope must be 'computer', 'project', 'external' or 'all'")
        params = _skill_address({"scope": scope}, tier, workdir)
        if type(include_diagnostics) is not bool:
            raise ValueError("include_diagnostics must be a boolean")
        if include_diagnostics:
            params["include_diagnostics"] = True
        return SkillPage.from_wire(await self._call("skills.list", _list_params(params, limit, cursor, 1000),
                                                    mutation=False))

    async def skills_get(self, name: str, *, tier: str | None = None, workdir: str | None = None,
                         include_raw: bool = False) -> SkillDetail:
        """One skill. Without `tier` the effective one for `name`. `include_raw` adds the file verbatim."""
        params = _skill_address({"name": _skill_name(name, rule=False)}, tier, workdir)
        if type(include_raw) is not bool:
            raise ValueError("include_raw must be a boolean")
        if include_raw:
            params["include_raw"] = True
        return SkillDetail.from_wire(await self._call("skills.get", params, mutation=False))

    async def skills_read_file(self, name: str, path: str, *, tier: str | None = None, workdir: str | None = None,
                               generation: str | None = None) -> SkillFile:
        """A UTF-8 companion file of a skill, by its path relative to the skill (up to 2 MiB)."""
        params = _skill_address({"name": _skill_name(name, rule=False)}, tier, workdir)
        if not isinstance(path, str) or not path:
            raise ValueError("path must be a relative file path")
        params["path"] = path
        if generation is not None:
            params["generation"] = _id_param("generation", generation)
        return SkillFile.from_wire(await self._call("skills.read_file", params, mutation=False))

    async def skills_create(self, *, scope: str, name: str, description: str, instruction: str,
                            workdir: str | None = None, triggers: Sequence[str] | None = None,
                            allowed_tools: Sequence[str] | None = None) -> SkillWriteResult:
        """Create a skill in the `computer` (`~/.koma/skills`) or `project` (`.agents/skills`) scope."""
        if scope not in _NEW_SKILL_SCOPES:
            raise ValueError("scope must be 'computer' or 'project'")
        params = _skill_address({"scope": scope, "name": _skill_name(name, rule=True)}, None, workdir)
        params["description"] = _text_param("description", description, 2000)
        params["instruction"] = _text_param("instruction", instruction, 2 * 1024 * 1024, by_bytes=True)
        if triggers is not None:
            params["triggers"] = _skill_text_list("triggers", triggers, 32, 200)
        if allowed_tools is not None:
            params["allowed_tools"] = _skill_text_list("allowed_tools", allowed_tools, 64, 100)
        return SkillWriteResult.from_wire(await self._call("skills.create", params, mutation=True))

    async def skills_update(self, *, tier: str, name: str, patch: Mapping[str, Any], workdir: str | None = None,
                            generation: str | None = None) -> SkillUpdateResult:
        """Change fields of a skill (`name`, `description`, `triggers`, `allowed_tools`, `instruction`).

        Pass the `generation` you read; a skill that changed since is refused (`reason == "stale"`).
        """
        params = _skill_address({"name": _skill_name(name, rule=False)}, _required_tier(tier), workdir)
        if generation is not None:
            params["generation"] = _id_param("generation", generation)
        params["patch"] = _skill_patch(patch)
        return SkillUpdateResult.from_wire(await self._call("skills.update", params, mutation=True))

    async def skills_install(self, *, scope: str, workdir: str | None = None, name: str | None = None,
                             data: bytes | None = None, path: str | None = None) -> SkillWriteResult:
        """Install a skill from zip `data` (sent inline, up to the adapter's cap) or an absolute zip `path` on the host."""
        if scope not in _NEW_SKILL_SCOPES:
            raise ValueError("scope must be 'computer' or 'project'")
        if (data is None) == (path is None):
            raise ValueError("give exactly one of data and path")
        params = _skill_address({"scope": scope}, None, workdir)
        if name is not None:
            params["name"] = _skill_name(name, rule=True)
        if data is not None:
            skills = self.hello.limits.get("skills") if self.hello is not None else None
            cap = skills.get("inline_zip_bytes") if isinstance(skills, dict) else None
            params["data_b64"] = inline_zip(data, max_bytes=cap if type(cap) is int and cap > 0 else INLINE_ZIP_BYTES)
        else:
            params["path"] = _absolute("path", path)
        return SkillWriteResult.from_wire(await self._call("skills.install", params, mutation=True))

    async def skills_remove(self, *, tier: str, name: str, workdir: str | None = None,
                            generation: str | None = None) -> SkillRemoveResult:
        params = _skill_address({"name": _skill_name(name, rule=False)}, _required_tier(tier), workdir)
        if generation is not None:
            params["generation"] = _id_param("generation", generation)
        return SkillRemoveResult.from_wire(await self._call("skills.remove", params, mutation=True))

    async def room_list(self, *, parent_session_id: str | None = None, live: bool | None = None,
                        limit: int | None = None, cursor: str | None = None) -> RoomPage:
        """Rooms, newest first; `parent_session_id` selects the room of one coding session."""
        params: dict[str, Any] = {}
        if parent_session_id is not None:
            params["parent_session_id"] = _id_param("parent_session_id", parent_session_id)
        if live is not None:
            if type(live) is not bool:
                raise ValueError("live must be a boolean")
            params["live"] = live
        return RoomPage.from_wire(await self._call("room.list", _list_params(params, limit, cursor, 500),
                                                   mutation=False))

    async def room_team_get(self) -> Team:
        """The template team (`~/.koma/room/`) every new room starts from."""
        return Team.from_wire(await self._call("room.team.get", {"target": "template"}, mutation=False,
                                               feature="room.template"))

    async def room_models_set(self, avatar: str, lane: str, *, model: Any = _OMIT, effort: Any = _OMIT,
                              if_revision: str | None = None) -> TeamWriteResult:
        """Set an avatar lane's model and/or effort in the template. It applies to rooms started afterwards."""
        params = models_set_params("template", avatar, lane, model, effort, if_revision)
        return TeamWriteResult.from_wire(await self._call("room.models.set", params, mutation=True,
                                                          feature="room.template"))

    async def room_avatars_upsert(self, avatar_id: str, *, role: Any = _OMIT, display: Any = _OMIT,
                                  alias: Any = _OMIT, if_revision: str | None = None) -> TeamWriteResult:
        params = avatars_upsert_params("template", avatar_id, role, display, alias, if_revision)
        return TeamWriteResult.from_wire(await self._call("room.avatars.upsert", params, mutation=True,
                                                          feature="room.template"))

    async def room_avatars_remove(self, avatar_id: str, *, if_revision: str | None = None) -> AvatarRemoveResult:
        params = avatars_remove_params("template", avatar_id, if_revision)
        return AvatarRemoveResult.from_wire(await self._call("room.avatars.remove", params, mutation=True,
                                                             feature="room.template"))


class Room(HostCalls):
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

    async def _call(self, method: str, params: dict[str, Any], *, mutation: bool,
                    feature: str | None = None) -> dict[str, Any]:
        return await self._request(method, params, mutation=mutation, feature=feature)

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
