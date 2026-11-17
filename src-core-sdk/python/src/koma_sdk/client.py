"""Async adapter/session ownership. One subprocess per connected session."""
from __future__ import annotations

import asyncio
import os
from dataclasses import replace
from collections.abc import AsyncIterator, Mapping, Sequence
from typing import Any

from ._transport import Transport
from .errors import (DeliveryUnknown, IncompatibleAdapter, ProtocolError, RemoteError,
                     RequestTimeout, TransportError)
from .room import (_OMIT, HostCalls, Room, _display_name, _id_param, attach_params, avatars_remove_params,
                   avatars_upsert_params, inline_zip, models_set_params)
from .room_types import (AttachResult, AvatarRemoveResult, CloseReason, CloseResult, HistoryPage, RoomClosedEvent, RoomPage,
                         RoomSpend, RoomStatus, SendResult, SentResult, ReportPage, ShelfItem, ShelfPage, Team,
                         TeamWriteResult, Trajectory, decode_room_event)
from .types import (ApprovalResult, Attachment, Binary, DetachResult, Hello, Policy,
                    ReplacementResult, State, StateEvent, Turn, TurnResult, TurnStatus, Event, OutputAppendEvent, TurnEvent, ApprovalPolicy,
                    ConfigView, EffortOptions, EffortResult, Limits, ModelDeleteResult, ProviderDeleteResult,
                    PublicRole, RoleUnassignResult, ToolPolicyResult, SessionInfo, CreateWarning, _create_warnings,
                    SessionDeleteResult, SessionPage, SessionRenameResult, SessionStopResult, SkillDetail,
                    SkillFile, SkillPage, SkillRemoveResult, SkillUpdateResult, SkillWriteResult, UsageReport)


_ROLES = ("main", "awareness", "safeguard", "compactor", "planner")


def _unassign_params(role: PublicRole | str, if_revision: str | None) -> dict[str, Any]:
    params: dict[str, Any] = {"role": _role_param(role), "scope": "global"}
    if if_revision is not None:
        params["if_revision"] = if_revision
    return params


def _tool_policy_params(deny: Sequence[str], if_revision: str | None) -> dict[str, Any]:
    if isinstance(deny, str) or not all(isinstance(entry, str) for entry in deny):
        raise TypeError("deny must be a list of tool names or globs")
    params: dict[str, Any] = {"deny": list(deny)}
    if if_revision is not None:
        params["if_revision"] = if_revision
    return params


def _configure_params(providers: Sequence[Mapping[str, Any]] | None,
                      models: Mapping[str, Mapping[str, Any]] | None) -> dict[str, Any]:
    if providers is None and models is None:
        raise ValueError("providers or models required")
    params: dict[str, Any] = {}
    if providers is not None:
        if not isinstance(providers, (list, tuple)):
            raise ValueError("providers must be a list")
        cleaned = []
        seen: set[str] = set()
        for item in providers:
            if not isinstance(item, Mapping):
                raise ValueError("provider must be an object")
            name = item.get("name")
            endpoint = item.get("endpoint")
            api_type = item.get("api_type")
            if not isinstance(name, str) or not name.strip():
                raise ValueError("provider name is empty")
            name = name.strip()
            if name in seen:
                raise ValueError(f"duplicate provider name '{name}'")
            seen.add(name)
            if not isinstance(endpoint, str) or not endpoint.strip():
                raise ValueError("endpoint must be an http(s) URL")
            if api_type not in ("openai", "anthropic"):
                raise ValueError("api_type must be 'openai' or 'anthropic'")
            entry: dict[str, Any] = {"name": name, "endpoint": endpoint.strip(), "api_type": api_type}
            if "api_key" in item and item.get("api_key") is not None:
                key = item.get("api_key")
                if not isinstance(key, str) or not key.strip():
                    raise ValueError("api_key must be non-empty when provided")
                entry["api_key"] = key.strip()
            cleaned.append(entry)
        params["providers"] = cleaned
    if models is not None:
        if not isinstance(models, Mapping):
            raise ValueError("models must be an object")
        cleaned_models: dict[str, Any] = {}
        for role, spec in models.items():
            if role not in _ROLES:
                raise ValueError(f"unknown model role '{role}'")
            if not isinstance(spec, Mapping):
                raise ValueError("model assignment must be an object")
            provider = spec.get("provider")
            model_id = spec.get("model_id")
            if not isinstance(provider, str) or not provider.strip() or not isinstance(model_id, str) or not model_id.strip():
                raise ValueError("model provider and model_id are required")
            assignment: dict[str, Any] = {"provider": provider.strip(), "model_id": model_id.strip()}
            if "route" in spec and spec.get("route") is not None:
                route = spec.get("route")
                if not isinstance(route, str):
                    raise ValueError("route must be a string")
                assignment["route"] = route
            cleaned_models[role] = assignment
        params["models"] = cleaned_models
    if not params.get("providers") and not params.get("models"):
        raise ValueError("providers or models required")
    return params


def _validate_configure(result: dict[str, Any]) -> None:
    providers = result["providers"]
    models = result["models"]
    if not isinstance(providers, list) or not isinstance(models, dict):
        raise ValueError("configure result shape")
    for provider in providers:
        if (not isinstance(provider, dict) or not isinstance(provider.get("name"), str) or not provider["name"]
                or not isinstance(provider.get("uuid"), str) or not provider["uuid"]
                or "api_key" in provider or "key" in provider):
            raise ValueError("configure provider")
    for role, spec in models.items():
        if (role not in _ROLES or not isinstance(spec, dict)
                or any(not isinstance(spec.get(field), str) or not spec[field] for field in ("uuid", "model_id", "provider"))
                or "api_key" in spec or "key" in spec):
            raise ValueError("configure model")


def _delete_params(key: str, uuid: str, if_revision: str | None, dry_run: bool) -> dict[str, Any]:
    params: dict[str, Any] = {key: _id_param(key, uuid)}
    if if_revision is not None:
        params["if_revision"] = if_revision
    if type(dry_run) is not bool:
        raise ValueError("dry_run must be a boolean")
    if dry_run:
        params["dry_run"] = True
    return params


def _role_param(role: PublicRole | str) -> str:
    try:
        return PublicRole(role).value
    except ValueError:
        raise ValueError(f"role must be one of {', '.join(_ROLES)}") from None


class ObservationUnavailable(TransportError):
    def __init__(self, turn_id: str, message: str):
        self.turn_id = turn_id
        super().__init__(message)


class _Wire(Transport):
    def __init__(self, *args: Any, **kwargs: Any):
        self.hello: asyncio.Future[Hello] = asyncio.get_running_loop().create_future()
        self.sequence = 0
        # The room binding as the adapter last confirmed it. Set while the frame is read, so that
        # events behind an attach response are judged against the new epoch.
        self.room_epoch: str | None = None
        self.room_floor = 0
        self.room_closed: CloseReason | None = None
        super().__init__(*args, **kwargs)

    def _fail(self, error: TransportError) -> None:
        if not self.hello.done():
            self.hello.set_exception(error)
        super()._fail(error)

    def _accept(self, message: dict[str, Any]) -> None:
        kind = message.get("type")
        if not self.hello.done():
            if kind != "hello":
                raise ProtocolError("first adapter frame must be hello")
            if type(message.get("protocol")) is not int or message["protocol"] != 1:
                raise IncompatibleAdapter(f"unsupported protocol {message.get('protocol')!r}; expected 1")
            b = message["binary"]
            caps, diagnostics = message["capabilities"], message["diagnostics"]
            if (not isinstance(b, dict) or not all(isinstance(b.get(k), str) for k in ("path", "version"))
                or not isinstance(caps, list) or not all(isinstance(c, str) for c in caps)
                or not isinstance(diagnostics, list) or not all(isinstance(d, str) for d in diagnostics)):
                raise ProtocolError("malformed hello")
            additions: dict[str, Any] = {}
            for key in ("methods", "features", "events"):
                if key in message:
                    names = message[key]
                    if not isinstance(names, list) or not all(isinstance(n, str) for n in names):
                        raise ProtocolError("malformed hello")
                    additions[key] = tuple(names)
            if "limits" in message:
                if not isinstance(message["limits"], dict):
                    raise ProtocolError("malformed hello")
                additions["limits"] = message["limits"]
            self.hello.set_result(Hello(1, Binary(b["path"], b["version"]), tuple(caps), tuple(diagnostics),
                                        **additions))
        elif kind == "response":
            request_id = message.get("id")
            if not isinstance(request_id, str) or not request_id:
                raise ProtocolError("response has no request id")
            if ("result" in message) == ("error" in message):
                raise ProtocolError("response must contain exactly result or error")
            p = self.pending.get(request_id)
            if p is None or p.future.done():
                return
            if "error" in message:
                error = message["error"]
                if (not isinstance(error, dict) or not isinstance(error.get("code"), str)
                    or not isinstance(error.get("message"), str)
                    or error.get("delivery") not in ("unknown", "not_sent")):
                    raise ProtocolError("malformed remote error")
                remote = RemoteError(error["code"], error["message"], error)
                p.future.set_exception(self._delivery(request_id, p, remote)
                                       if p.mutation and error["delivery"] == "unknown" else remote)
            elif not isinstance(message["result"], dict):
                raise ProtocolError("response result must be object")
            else:
                # Decode before resolving: malformed dispatched mutation results remain unknown.
                try:
                    _validate_result(p.method, message["result"])
                except (ValueError, KeyError, TypeError) as exc:
                    raise ProtocolError(f"malformed {p.method} result: {exc}") from exc
                self._room_binding(p.method, message["result"])
                p.future.set_result(message["result"])
        elif kind == "event":
            seq = message.get("seq")
            if type(seq) is not int or seq <= self.sequence:
                raise ProtocolError("invalid/nonmonotonic event sequence")
            message = dict(message)
            message["_gap"] = seq != self.sequence + 1
            self.sequence = seq
            if not isinstance(message.get("event"), str):
                raise ProtocolError("missing event name")
            if message["event"] == "state":
                State.from_wire(message["state"])
                if type(message["resynced"]) is not bool:
                    raise ProtocolError("invalid resynced flag")
            elif message["event"].startswith("room."):
                try:
                    decoded = decode_room_event(message)
                except (ValueError, KeyError, TypeError) as exc:
                    raise ProtocolError(f"malformed {message['event']} event: {exc}") from exc
                if decoded is None or decoded.epoch != self.room_epoch:
                    return
                if isinstance(decoded, RoomClosedEvent):
                    self.room_closed = decoded.reason
                message["_room"] = decoded
            elif message["event"] in ("output_append", "turn"):
                if type(message["revision"]) is not int or message["revision"] < 1:
                    raise ProtocolError("invalid event revision")
                if message["event"] == "output_append" and (type(message["offset"]) is not int or message["offset"] < 0 or not isinstance(message["text"], str) or not message["text"]):
                    raise ProtocolError("invalid output append")
                if message["event"] == "turn" and message["turn"] is not None:
                    t = message["turn"]
                    if (not isinstance(t, dict) or not isinstance(t["id"], str) or not t["id"]
                        or (t["error"] is not None and not isinstance(t["error"], str))):
                        raise ProtocolError("invalid turn event")
                    TurnStatus(t["state"])
            self.publish(message)
        else:
            raise ProtocolError(f"unexpected frame type {kind!r}")


    def _room_binding(self, method: str, result: dict[str, Any]) -> None:
        if method in ("room.open", "room.attach"):
            self.room_epoch = result["epoch"]
            self.room_floor = result["history"]["last_line_seq"] or 0
            self.room_closed = None
        elif method == "room.close":
            self.room_epoch = None


def _validate_limits(d: dict[str, Any]) -> None:
    Limits.from_wire(d)
    if "session_info" in d:
        SessionInfo.from_wire(d["session_info"])


# One decoder per method added after v1; enum-like strings in them are open (unknown -> OTHER).
_RESULTS: dict[str, Any] = {
    "config.get": ConfigView.from_wire,
    "config.providers.delete": ProviderDeleteResult.from_wire,
    "config.models.delete": ModelDeleteResult.from_wire,
    "config.roles.unassign": RoleUnassignResult.from_wire,
    "config.tool_policy.set": ToolPolicyResult.from_wire,
    "config.effort.get": EffortOptions.from_wire,
    "config.effort.set": EffortResult.from_wire,
    "config.limits.get": _validate_limits,
    "config.limits.set": _validate_limits,
    "sessions.info": lambda d: SessionInfo.from_wire(d["session_info"]),
    "sessions.list": SessionPage.from_wire,
    "sessions.rename": SessionRenameResult.from_wire,
    "sessions.delete": SessionDeleteResult.from_wire,
    "sessions.stop": SessionStopResult.from_wire,
    "usage.query": UsageReport.from_wire,
    "skills.list": SkillPage.from_wire,
    "skills.get": SkillDetail.from_wire,
    "skills.read_file": SkillFile.from_wire,
    "skills.create": SkillWriteResult.from_wire,
    "skills.update": SkillUpdateResult.from_wire,
    "skills.install": SkillWriteResult.from_wire,
    "skills.remove": SkillRemoveResult.from_wire,
    "room.list": RoomPage.from_wire,
    "room.open": AttachResult.from_wire,
    "room.attach": AttachResult.from_wire,
    "room.close": CloseResult.from_wire,
    "room.send": SendResult.from_wire,
    "room.interrupt": SentResult.from_wire,
    "room.control": SentResult.from_wire,
    "room.status": RoomStatus.from_wire,
    "room.history": HistoryPage.from_wire,
    "room.shelf.list": ShelfPage.from_wire,
    "room.item": ShelfItem.from_wire,
    "room.trajectory": Trajectory.from_wire,
    "room.reports": ReportPage.from_wire,
    "room.spend": RoomSpend.from_wire,
    "room.team.get": Team.from_wire,
    "room.models.set": TeamWriteResult.from_wire,
    "room.avatars.upsert": TeamWriteResult.from_wire,
    "room.avatars.remove": AvatarRemoveResult.from_wire,
}


def _validate_result(method: str, d: dict[str, Any]) -> None:
    if method in ("create", "resume", "state"):
        State.from_wire(d["state"])
        if "session_info" in d:
            SessionInfo.from_wire(d["session_info"])
        _create_warnings(d)
    elif method in ("submit", "enqueue", "interrupt", "interrupt_and_submit"):
        result = TurnResult.from_wire(d)
        allowed = {"submit": {"started", "rejected"}, "enqueue": {"queued", "rejected"},
                   "interrupt": {"interrupted", "rejected"},
                   "interrupt_and_submit": {"started", "rejected"}}[method]
        if result.disposition.value not in allowed:
            raise ValueError("invalid disposition for method")
        if result.disposition.value in ("started", "interrupted") and not result.turn_id:
            raise ValueError("successful operation missing turn id")
        if result.disposition.value == "queued" and result.turn_id is not None:
            raise ValueError("enqueue does not reserve a turn id")
        if (result.disposition.value == "rejected") != (result.reason is not None):
            raise ValueError("reason must be present only on rejection")
        if method == "interrupt_and_submit":
            if d["interrupted_turn_id"] is not None and not isinstance(d["interrupted_turn_id"], str):
                raise ValueError("invalid interrupted turn id")
    elif method == "approve":
        if type(d["approved"]) is not bool:
            raise ValueError("approved must be boolean")
    elif method == "detach":
        if d["detached"] is not True:
            raise ValueError("detach not acknowledged")
    elif method == "configure":
        _validate_configure(d)
    elif method in _RESULTS:
        _RESULTS[method](d)


class Client:
    """Owns session adapters, never the daemon. No subprocess is started at construction.

    Normal argv is exactly [executable, 'sdk', 'serve']; adapter_args exists for
    test adapters. No shell interpretation is ever performed.
    """
    def __init__(self, executable: str | os.PathLike[str] = "koma", *,
                 adapter_args: Sequence[str] = ("sdk", "serve"),
                 request_timeout: float = 30.0, close_timeout: float = 2.0,
                 event_capacity: int = 256):
        if request_timeout <= 0 or close_timeout <= 0 or not 1 <= event_capacity <= 256:
            raise ValueError("timeouts must be positive; event_capacity must be 1..256")
        self.executable = os.fspath(executable)
        self.adapter_args = tuple(adapter_args)
        self.request_timeout = request_timeout
        self.close_timeout = close_timeout
        self.event_capacity = event_capacity
        self._sessions: set[Session] = set()
        self._transports: set[_Wire] = set()
        self._closed = False

    async def __aenter__(self) -> Client:
        if self._closed:
            raise TransportError("client closed")
        return self

    async def __aexit__(self, *exc: Any) -> None:
        await self.close()

    async def _connect(self, required: Sequence[str]) -> tuple[_Wire, Hello]:
        if self._closed:
            raise TransportError("client closed")
        try:
            process = await asyncio.create_subprocess_exec(
                self.executable, *self.adapter_args, stdin=asyncio.subprocess.PIPE,
                stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE)
        except OSError as exc:
            raise TransportError(f"cannot launch {self.executable!r}: {exc}") from exc
        t = _Wire(process, event_capacity=self.event_capacity, close_timeout=self.close_timeout)
        self._transports.add(t)
        try:
            if self._closed:
                raise TransportError("client closed during subprocess startup")
            hello = await asyncio.wait_for(asyncio.shield(t.hello), self.request_timeout)
            missing = {method for method in required if not hello.supports(method)}
            if missing:
                raise IncompatibleAdapter(f"missing required capabilities: {', '.join(sorted(missing))}")
            return t, hello
        except BaseException:
            await t.close()
            if t.hello.done() and not t.hello.cancelled():
                t.hello.exception()
            self._transports.discard(t)
            raise

    async def probe(self, *, required_capabilities: Sequence[str] = ("create", "resume", "state", "detach")) -> Hello:
        """Read/validate unsolicited hello, then close that adapter without binding."""
        t, hello = await self._connect(required_capabilities)
        try:
            return hello
        finally:
            await t.close()
            self._transports.discard(t)

    async def create(self, cwd: str | os.PathLike[str], *, model_id: str | None = None,
                     policy: Policy = Policy(), external_approvals: bool = False,
                     name: str | None = None) -> Session:
        """Start a coding session. `name` needs an adapter with the `create.name` feature.

        If naming fails the session is still created: see `session.warnings` (`rename_failed`).
        """
        cwd = os.fspath(cwd)
        if not os.path.isabs(cwd):
            raise ValueError("cwd must be absolute on daemon host")
        if policy.approval == ApprovalPolicy.DEFAULT and not external_approvals:
            raise ValueError("approval default requires external_approvals=True")
        params: dict[str, Any] = {"cwd": cwd, "policy": policy.to_wire(),
                                  "external_approvals": external_approvals}
        if model_id is not None:
            params["model_id"] = model_id
        if name is not None:
            params["name"] = _display_name(name)
        return await self._bind("create", params, feature="create.name" if name is not None else None)

    async def resume(self, session_id: str, *, policy: Policy | None = None,
                     external_approvals: bool = False) -> Session:
        if not session_id:
            raise ValueError("session_id must not be empty")
        params: dict[str, Any] = {"session_id": session_id, "external_approvals": external_approvals}
        if policy is not None:
            if policy.approval == ApprovalPolicy.DEFAULT and not external_approvals:
                raise ValueError("approval default requires external_approvals=True")
            params["policy"] = policy.to_wire()
        return await self._bind("resume", params)

    async def configure(self, *, providers: Sequence[Mapping[str, Any]] | None = None,
                        models: Mapping[str, Mapping[str, Any]] | None = None) -> dict[str, Any]:
        """Write providers and model roles before a session exists. No daemon is started."""
        params = _configure_params(providers, models)
        transport, _hello = await self._connect(("configure",))
        try:
            return await transport.request(
                "configure", params, mutation=True,
                envelope=lambda i, m, p: {"id": i, "method": m, "params": p},
                timeout=self.request_timeout)
        finally:
            await transport.close()
            self._transports.discard(transport)

    def host(self, *, recycle_after: int = 60_000) -> Host:
        """A handle for calls that need no session. Its adapter starts on the first call."""
        if self._closed:
            raise TransportError("client closed")
        return Host(self, recycle_after=recycle_after)

    async def room_open(self, parent_session_id: str, *, tail: int | None = None,
                        wait_ms: int | None = None) -> Room:
        """Start (when needed) and attach the room of a coding session. `wait_ms` is 0 to 30000 (default 12000).

        Takes the room's live view over from any other client. The returned `Room` owns its adapter.
        """
        params = attach_params("parent_session_id", parent_session_id, tail, wait_ms, 30000)
        return await self._bind_room("room.open", params, wait_ms or 12000)

    async def room_attach(self, session_id: str, *, tail: int | None = None, wait_ms: int | None = None) -> Room:
        """Attach a room that is already running, by the room's own session id. `wait_ms` is 0 to 15000.

        Takes the room's live view over from any other client; a room that is not running is
        `RemoteError` with `reason == "not_running"` (use `room_open`).
        """
        params = attach_params("session_id", session_id, tail, wait_ms, 15000)
        return await self._bind_room("room.attach", params, wait_ms or 2000)

    async def _bind_room(self, method: str, params: dict[str, Any], wait_ms: int) -> Room:
        t, hello = await self._connect((method, "room.close", "room.history"))
        try:
            result = await t.request(method, params, mutation=True,
                                     envelope=lambda i, m, p: {"id": i, "method": m, "params": p},
                                     timeout=max(self.request_timeout, wait_ms / 1000 + 10))
            return Room(self, t, hello, AttachResult.from_wire(result))
        except BaseException:
            await t.close()
            self._transports.discard(t)
            raise

    async def _bind(self, method: str, params: dict[str, Any], feature: str | None = None) -> Session:
        t, hello = await self._connect((method, "state", "detach"))
        session = Session(self, t, hello, external_approvals=params["external_approvals"])
        try:
            if feature is not None and not hello.has_feature(feature):
                raise IncompatibleAdapter(f"adapter lacks feature {feature}")
            response = await session._request(method, params, mutation=True)
            session._update_state(State.from_wire(response["state"]))
            session._apply_info(response)
            session.warnings = _create_warnings(response)
            self._sessions.add(session)
            return session
        except BaseException:
            await t.close()
            self._transports.discard(t)
            raise

    async def close(self) -> None:
        self._closed = True
        await asyncio.gather(*(t.close() for t in tuple(self._transports)))
        self._transports.clear()
        self._sessions.clear()


class Host(HostCalls):
    """Calls that need no session (config reads, global config writes) on an unbound adapter.

    The adapter is started on the first call and replaced transparently before its request-id
    limit: an unbound adapter holds no state. A failed adapter is replaced on the next call;
    the call that failed is never retried.
    """
    def __init__(self, client: Client, *, recycle_after: int = 60_000):
        if type(recycle_after) is not int or recycle_after < 1:
            raise ValueError("recycle_after must be a positive integer")
        self._client = client
        self.recycle_after = recycle_after
        self.hello: Hello | None = None
        self._transport: _Wire | None = None
        self._sent = 0
        self._budget = 0
        self._inflight: dict[_Wire, int] = {}
        self._lock = asyncio.Lock()

    async def __aenter__(self) -> Host:
        return self

    async def __aexit__(self, *exc: Any) -> None:
        await self.close()

    @property
    def requests_remaining(self) -> int | None:
        """Requests left on the current adapter before it is replaced; None before the first call."""
        return None if self._transport is None else self._budget - self._sent

    async def _retire(self, transport: _Wire) -> None:
        if self._inflight.get(transport, 0) == 0:
            self._inflight.pop(transport, None)
            await transport.close()
            self._client._transports.discard(transport)

    async def _call(self, method: str, params: dict[str, Any], *, mutation: bool,
                    feature: str | None = None) -> dict[str, Any]:
        async with self._lock:
            t = self._transport
            if t is None or t.failure is not None or t._closed or self._sent >= self._budget:
                self._transport = None
                if t is not None:
                    await self._retire(t)
                t, hello = await self._client._connect(())
                self._transport, self.hello, self._sent = t, hello, 0
                self._budget = min(self.recycle_after, hello.request_ids)
            assert self.hello is not None
            if not self.hello.supports(method):
                raise IncompatibleAdapter(f"adapter does not support {method}")
            if feature is not None and not self.hello.has_feature(feature):
                raise IncompatibleAdapter(f"adapter lacks feature {feature}")
            self._sent += 1
            self._inflight[t] = self._inflight.get(t, 0) + 1
        try:
            return await t.request(method, params, mutation=mutation,
                envelope=lambda i, m, p: {"id": i, "method": m, "params": p},
                timeout=self._client.request_timeout)
        finally:
            self._inflight[t] -= 1
            if t is not self._transport:
                await self._retire(t)

    async def configure(self, *, providers: Sequence[Mapping[str, Any]] | None = None,
                        models: Mapping[str, Mapping[str, Any]] | None = None) -> dict[str, Any]:
        """Write providers and model roles. No daemon is started."""
        return await self._call("configure", _configure_params(providers, models), mutation=True)

    async def config_get(self) -> ConfigView:
        """The global view of config.json. Secrets are never included."""
        return ConfigView.from_wire(await self._call("config.get", {"view": "global"}, mutation=False))

    async def config_providers_delete(self, provider_uuid: str, *, if_revision: str | None = None,
                                      dry_run: bool = False) -> ProviderDeleteResult:
        """Delete a provider and its models. `dry_run=True` reports the effect and writes nothing."""
        params = _delete_params("provider_uuid", provider_uuid, if_revision, dry_run)
        return ProviderDeleteResult.from_wire(
            await self._call("config.providers.delete", params, mutation=not dry_run))

    async def config_models_delete(self, model_uuid: str, *, if_revision: str | None = None,
                                   dry_run: bool = False) -> ModelDeleteResult:
        """Delete a global model entry. `dry_run=True` reports the effect and writes nothing."""
        params = _delete_params("model_uuid", model_uuid, if_revision, dry_run)
        return ModelDeleteResult.from_wire(
            await self._call("config.models.delete", params, mutation=not dry_run))

    async def config_roles_unassign(self, role: PublicRole | str, *,
                                    if_revision: str | None = None) -> RoleUnassignResult:
        """Clear a role globally. An unassigned compactor, awareness or planner inherits Main;
        safeguard never does."""
        return RoleUnassignResult.from_wire(
            await self._call("config.roles.unassign", _unassign_params(role, if_revision), mutation=True))

    async def config_tool_policy_set(self, deny: Sequence[str], *,
                                     if_revision: str | None = None) -> ToolPolicyResult:
        """Replace the hard tool deny list with `deny` (tool names, `*` and `?` globs; `[]` clears it). A denied
        tool is never offered to a model and its calls are refused on every path. Only that key of config.json
        changes, so a key another writer saved is kept."""
        return ToolPolicyResult.from_wire(
            await self._call("config.tool_policy.set", _tool_policy_params(deny, if_revision), mutation=True))

    async def close(self) -> None:
        async with self._lock:
            t, self._transport = self._transport, None
        if t is not None:
            await t.close()
            self._client._transports.discard(t)
            self._inflight.pop(t, None)


_ANY_MAIN: Any = object()


class Session(HostCalls):
    def __init__(self, client: Client, transport: _Wire, hello: Hello, *, external_approvals: bool):
        self._client, self._transport, self.hello = client, transport, hello
        self._state: State | None = None
        self._info: SessionInfo | None = None
        self.warnings: tuple[CreateWarning, ...] = ()
        self._requests = 0
        self._external_approvals = external_approvals
        self._event_lock = asyncio.Lock()
        self._approval_lock = asyncio.Lock()

    @property
    def session_id(self) -> str:
        assert self._state is not None
        return self._state.session_id

    @property
    def stderr_tail(self) -> str:
        return self._transport.stderr.decode("utf-8", errors="replace")

    async def __aenter__(self) -> Session:
        return self

    async def __aexit__(self, *exc: Any) -> None:
        await self.close()

    @property
    def requests_remaining(self) -> int:
        """Requests this adapter still accepts. At 0, resume the session from a fresh adapter."""
        return max(0, self.hello.request_ids - self._requests)

    @property
    def session_info(self) -> SessionInfo | None:
        """The newest `SessionInfo` seen, or None when the adapter does not send it."""
        return self._info

    async def _call(self, method: str, params: dict[str, Any], *, mutation: bool,
                    feature: str | None = None) -> dict[str, Any]:
        if feature is not None and not self.hello.has_feature(feature):
            raise IncompatibleAdapter(f"adapter lacks feature {feature}")
        return await self._request(method, params, mutation=mutation)

    async def _request(self, method: str, params: dict[str, Any], *, mutation: bool) -> dict[str, Any]:
        if not self.hello.supports(method):
            raise IncompatibleAdapter(f"adapter does not support {method}")
        if self.requests_remaining == 0:
            raise TransportError("adapter request-id limit reached; resume from a fresh adapter")
        self._requests += 1
        try:
            return await self._transport.request(method, params, mutation=mutation,
                envelope=lambda i, m, p: {"id": i, "method": m, "params": p},
                timeout=self._client.request_timeout)
        except DeliveryUnknown as exc:
            if self._state is not None:
                exc.session_id = self._state.session_id
            raise

    def _update_state(self, state: State) -> State:
        if self._state is None or state.revision > self._state.revision:
            self._state = state
        return self._state

    def _update_info(self, info: SessionInfo) -> SessionInfo:
        # Ordered by the adapter's observation clock, never by the state revision.
        if self._info is None or info.observed_at_ms > self._info.observed_at_ms:
            self._info = info
        return self._info

    def _apply_info(self, result: dict[str, Any]) -> None:
        if isinstance(result, dict) and "session_info" in result:
            self._update_info(SessionInfo.from_wire(result["session_info"]))

    async def get_state(self) -> State:
        result = await self._request("state", {}, mutation=False)
        state = self._update_state(State.from_wire(result["state"]))
        self._apply_info(result)
        return state

    async def info(self) -> SessionInfo:
        """Fresh session facts (name, effort, routes, limits, daemon); costs no daemon round trip."""
        result = await self._request("sessions.info", {}, mutation=False)
        return self._update_info(SessionInfo.from_wire(result["session_info"]))

    async def config_get(self, *, view: str = "session") -> ConfigView:
        """config.json as this session sees it (`view="session"`, with its Main pin) or globally."""
        if view not in ("session", "global"):
            raise ValueError("view must be 'session' or 'global'")
        return ConfigView.from_wire(await self._request("config.get", {"view": view}, mutation=False))

    async def config_providers_delete(self, provider_uuid: str, *, if_revision: str | None = None,
                                      dry_run: bool = False) -> ProviderDeleteResult:
        """Delete a provider and its models through this session's daemon. `dry_run=True` writes nothing."""
        params = _delete_params("provider_uuid", provider_uuid, if_revision, dry_run)
        result = await self._request("config.providers.delete", params, mutation=not dry_run)
        self._apply_info(result)
        return ProviderDeleteResult.from_wire(result)

    async def config_models_delete(self, model_uuid: str, *, if_revision: str | None = None,
                                   dry_run: bool = False) -> ModelDeleteResult:
        """Delete a global model entry through this session's daemon. `dry_run=True` writes nothing."""
        params = _delete_params("model_uuid", model_uuid, if_revision, dry_run)
        result = await self._request("config.models.delete", params, mutation=not dry_run)
        self._apply_info(result)
        return ModelDeleteResult.from_wire(result)

    async def config_roles_unassign(self, role: PublicRole | str, *,
                                    if_revision: str | None = None) -> RoleUnassignResult:
        """Clear a role globally, as `Host.config_roles_unassign` does. To drop this session's own
        Main pin, use `clear_main_pin`."""
        result = await self._request("config.roles.unassign", _unassign_params(role, if_revision), mutation=True)
        self._apply_info(result)
        return RoleUnassignResult.from_wire(result)

    async def config_tool_policy_set(self, deny: Sequence[str], *,
                                     if_revision: str | None = None) -> ToolPolicyResult:
        """Replace the hard tool deny list, as `Host.config_tool_policy_set` does."""
        result = await self._request("config.tool_policy.set", _tool_policy_params(deny, if_revision),
                                     mutation=True)
        self._apply_info(result)
        return ToolPolicyResult.from_wire(result)

    async def clear_main_pin(self) -> RoleUnassignResult:
        """Clear this session's Main pin, so Main follows the global assignment. Resets effort."""
        result = await self._request("config.roles.unassign", {"role": "main", "scope": "session"}, mutation=True)
        self._apply_info(result)
        return RoleUnassignResult.from_wire(result)

    async def effort_get(self, *, wait_ms: int | None = None) -> EffortOptions:
        """Effort options for the session's Main model; may wait up to `wait_ms` while they load."""
        params: dict[str, Any] = {}
        if wait_ms is not None:
            if type(wait_ms) is not int or not 0 <= wait_ms <= 10000:
                raise ValueError("wait_ms must be an integer from 0 to 10000")
            params["wait_ms"] = wait_ms
        return EffortOptions.from_wire(await self._request("config.effort.get", params, mutation=False))

    async def effort_set(self, effort: str, *, if_main_model: str | None = _ANY_MAIN) -> EffortResult:
        """Set reasoning effort (`"default"` = model default) for this session.

        With `if_main_model`, the call is refused (`main_changed`) unless that is still the Main model.
        """
        if (not isinstance(effort, str) or not 1 <= len(effort) <= 64
                or any(c.isspace() or ord(c) < 0x20 or ord(c) == 0x7f for c in effort)):
            raise ValueError("effort must be 1 to 64 characters without whitespace or control characters")
        params: dict[str, Any] = {"effort": effort}
        if if_main_model is not _ANY_MAIN:
            if if_main_model is not None and not isinstance(if_main_model, str):
                raise ValueError("if_main_model must be a string or None")
            params["if_main_model"] = if_main_model
        result = await self._request("config.effort.set", params, mutation=True)
        self._apply_info(result)
        return EffortResult.from_wire(result)

    async def limits_get(self) -> Limits:
        return Limits.from_wire(await self._request("config.limits.get", {}, mutation=False))

    async def limits_set(self, *, max_output_tokens: int | None = None, context_window_limit: int | None = None,
                         context_model_alias: str | None = None) -> Limits:
        """Change output and context limits from the next model call. `0` means auto; `""` clears the alias."""
        params: dict[str, Any] = {}
        for key, value in (("max_output_tokens", max_output_tokens), ("context_window_limit", context_window_limit)):
            if value is not None:
                if type(value) is not int or value < 0:
                    raise ValueError(f"{key} must be a non-negative integer")
                params[key] = value
        if context_model_alias is not None:
            if not isinstance(context_model_alias, str):
                raise ValueError("context_model_alias must be a string")
            params["context_model_alias"] = context_model_alias
        if not params:
            raise ValueError("give max_output_tokens, context_window_limit or context_model_alias")
        result = await self._request("config.limits.set", params, mutation=True)
        self._apply_info(result)
        return Limits.from_wire(result)

    async def events(self) -> AsyncIterator[Event]:
        """Single consumer; revisions/offsets prevent duplicate output after readback."""
        if self._event_lock.locked():
            raise RuntimeError("only one events iterator may consume a session")
        async with self._event_lock:
            if self._transport.restart_events():
                seq = self._transport.sequence
                state = await self.get_state()
                self._transport.finish_recovery(state.revision)
                yield StateEvent(seq, state, True)
            while True:
                e = await self._transport.next_event()
                if e["_gap"]:
                    yield StateEvent(e["seq"], await self.get_state(), True)
                    continue
                if e["event"] == "state":
                    state = State.from_wire(e["state"])
                    if self._state is None or state.revision > self._state.revision:
                        self._update_state(state)
                        yield StateEvent(e["seq"], state, e["resynced"])
                elif e["event"] == "output_append":
                    current = self._state
                    if current is not None and e["revision"] <= current.revision:
                        continue
                    if current is None or e["revision"] != current.revision + 1 or e["offset"] != len((current.streaming or "").encode("utf-8")):
                        yield StateEvent(e["seq"], await self.get_state(), True)
                    else:
                        self._state = replace(current, revision=e["revision"], streaming=(current.streaming or "") + e["text"])
                        yield OutputAppendEvent(e["seq"], e["revision"], e["offset"], e["text"])
                elif e["event"] == "turn":
                    t = e["turn"]
                    if self._state is not None and e["revision"] == self._state.revision:
                        yield TurnEvent(e["seq"], e["revision"], None if t is None else Turn(t["id"], TurnStatus(t["state"]), t["error"]))

    @staticmethod
    def _input(text: str, attachments: Sequence[Attachment]) -> dict[str, Any]:
        if not text.strip() or len(text.encode("utf-8")) > 2 * 1024 * 1024:
            raise ValueError("text must be nonblank and at most 2 MiB UTF-8")
        if len(attachments) > 16:
            raise ValueError("at most 16 attachments")
        wire = [a.to_wire() for a in attachments]
        # Image size/format is validated on daemon host, not on the SDK host.
        return {"text": text, "attachments": wire}

    async def submit(self, text: str, *, attachments: Sequence[Attachment] = ()) -> TurnResult:
        return TurnResult.from_wire(await self._request("submit", self._input(text, attachments), mutation=True))

    async def enqueue(self, text: str, *, attachments: Sequence[Attachment] = ()) -> TurnResult:
        return TurnResult.from_wire(await self._request("enqueue", self._input(text, attachments), mutation=True))

    async def interrupt(self, turn_id: str) -> TurnResult:
        if not turn_id:
            raise ValueError("turn_id must not be empty")
        return TurnResult.from_wire(await self._request("interrupt", {"turn_id": turn_id}, mutation=True))

    async def interrupt_and_submit(self, turn_id: str, text: str, *,
                                   attachments: Sequence[Attachment] = ()) -> ReplacementResult:
        if not turn_id:
            raise ValueError("turn_id must not be empty")
        params = self._input(text, attachments)
        params["turn_id"] = turn_id
        d = await self._request("interrupt_and_submit", params, mutation=True)
        r = TurnResult.from_wire(d)
        return ReplacementResult(r.disposition, r.turn_id, r.reason, d["interrupted_turn_id"])

    async def configure(self, *, providers: Sequence[Mapping[str, Any]] | None = None,
                        models: Mapping[str, Mapping[str, Any]] | None = None) -> dict[str, Any]:
        """Update providers or model roles on this live session. The next turn uses them."""
        return await self._request("configure", _configure_params(providers, models), mutation=True)

    async def approve(self, call_id: str, approve: bool) -> ApprovalResult:
        if not self._external_approvals:
            raise ValueError("external approvals were not opted in on create/resume")
        if not call_id or type(approve) is not bool:
            raise ValueError("approve requires a pending call ID and a boolean decision")
        async with self._approval_lock:
            result = await self._request("approve", {"call_id": call_id, "approve": approve}, mutation=True)
            return ApprovalResult(result["approved"])

    async def wait_turn(self, turn_id: str, *, timeout: float = 60.0, poll_interval: float = .1) -> Turn:
        """Observe terminal state, not goal success. Does not consume the event queue."""
        if not turn_id or timeout <= 0 or poll_interval <= 0:
            raise ValueError("turn_id and positive timeout/poll_interval required")
        observed = False
        try:
            async with asyncio.timeout(timeout):
                while True:
                    state = await self.get_state()
                    if state.turn is not None and state.turn.id == turn_id:
                        observed = True
                        if state.turn.state is not TurnStatus.RUNNING:
                            return state.turn
                    elif observed or state.turn is not None:
                        raise ObservationUnavailable(turn_id, "requested turn hidden by a newer/different turn")
                    await asyncio.sleep(poll_interval)
        except TimeoutError as exc:
            raise ObservationUnavailable(turn_id, "terminal state not observed before timeout; work may continue") from exc

    async def detach(self) -> DetachResult:
        try:
            result = await self._request("detach", {}, mutation=True)
            return DetachResult(result["detached"])
        finally:
            await self.close()

    async def close(self) -> None:
        await self._transport.close()
        self._client._transports.discard(self._transport)
        self._client._sessions.discard(self)
