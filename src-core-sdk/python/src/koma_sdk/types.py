"""Immutable public protocol DTOs. Unknown additive fields are ignored."""
from __future__ import annotations
from collections.abc import Mapping
from dataclasses import dataclass, field
from enum import Enum
from typing import Any

class PlanPolicy(str, Enum):
    ALLOW = "allow"
    FORBID = "forbid"

class ApprovalPolicy(str, Enum):
    DEFAULT = "default"
    REJECT = "reject"

class TurnStatus(str, Enum):
    RUNNING = "running"
    FINISHED = "finished"
    INTERRUPTED = "interrupted"
    FAILED = "failed"

class Disposition(str, Enum):
    STARTED = "started"
    QUEUED = "queued"
    INTERRUPTED = "interrupted"
    REJECTED = "rejected"

class Role(str, Enum):
    SYSTEM = "system"
    USER = "user"
    ASSISTANT = "assistant"
    TOOL = "tool"

@dataclass(frozen=True)
class Policy:
    plan: PlanPolicy = PlanPolicy.FORBID
    approval: ApprovalPolicy = ApprovalPolicy.REJECT
    def to_wire(self) -> dict[str, str]:
        return {"plan": PlanPolicy(self.plan).value, "approval": ApprovalPolicy(self.approval).value}

@dataclass(frozen=True)
class Binary:
    path: str
    version: str

@dataclass(frozen=True)
class Hello:
    """`methods`, `features`, `events` and `limits` are empty when the adapter predates them."""
    protocol: int
    binary: Binary
    capabilities: tuple[str, ...]
    diagnostics: tuple[str, ...]
    methods: tuple[str, ...] = ()
    features: tuple[str, ...] = ()
    events: tuple[str, ...] = ()
    limits: Mapping[str, Any] = field(default_factory=dict, hash=False)

    def supports(self, method: str) -> bool:
        """Gate on `methods`; an adapter without them implements exactly `capabilities`."""
        return method in (self.methods if self.methods else self.capabilities)

    def has_feature(self, feature: str) -> bool:
        """Features are static per adapter binary; an older adapter has none."""
        return feature in self.features

    @property
    def request_ids(self) -> int:
        """Requests one adapter process accepts; the next one is fatal."""
        value = self.limits.get("request_ids")
        return value if type(value) is int and value > 0 else 65536

@dataclass(frozen=True)
class Turn:
    id: str
    state: TurnStatus
    error: str | None

@dataclass(frozen=True)
class Message:
    role: Role
    content: str

@dataclass(frozen=True)
class PendingApproval:
    call_id: str
    reason: str | None

class GateKind(str, Enum):
    TOOL = "tool"
    PLAN = "plan"
    MISSION = "mission"
    UNKNOWN = "unknown"

@dataclass(frozen=True)
class Gate:
    kind: GateKind
    reason: str | None
    supported: bool

@dataclass(frozen=True)
class State:
    session_id: str
    revision: int
    external_approvals: bool
    pending_approval: PendingApproval | None
    gate: Gate | None
    cwd: str
    model_id: str
    policy: Policy
    turn: Turn | None
    working: bool
    awaiting_approval: bool
    approval_reason: str | None
    pending_count: int
    messages: tuple[Message, ...]
    streaming: str | None
    @classmethod
    def from_wire(cls, d: dict[str, Any]) -> State:
        if not isinstance(d, dict):
            raise ValueError("state must be object")
        for key in ("session_id", "cwd", "model_id"):
            if not isinstance(d[key], str):
                raise ValueError(f"{key} must be string")
        for key in ("working", "awaiting_approval", "external_approvals"):
            if type(d[key]) is not bool:
                raise ValueError(f"{key} must be boolean")
        if type(d["revision"]) is not int or not 1 <= d["revision"] <= 9007199254740991:
            raise ValueError("invalid revision")
        if type(d["pending_count"]) is not int or not 0 <= d["pending_count"] <= 5:
            raise ValueError("invalid queue count")
        for key in ("approval_reason", "streaming"):
            if d[key] is not None and not isinstance(d[key], str):
                raise ValueError(f"invalid {key}")
        if not isinstance(d["messages"], list) or any(not isinstance(m["content"], str) for m in d["messages"]):
            raise ValueError("invalid messages")
        t, a, g = d["turn"], d["pending_approval"], d["gate"]
        if t is not None and (not isinstance(t["id"], str) or not t["id"] or (t["error"] is not None and not isinstance(t["error"], str))):
            raise ValueError("invalid turn")
        if a is not None and (not isinstance(a["call_id"], str) or not a["call_id"] or (a["reason"] is not None and not isinstance(a["reason"], str))):
            raise ValueError("invalid pending approval")
        if g is not None and (type(g["supported"]) is not bool or (g["reason"] is not None and not isinstance(g["reason"], str))):
            raise ValueError("invalid gate")
        return cls(d["session_id"], d["revision"], d["external_approvals"],
                   None if a is None else PendingApproval(a["call_id"], a["reason"]),
                   None if g is None else Gate(GateKind(g["kind"]), g["reason"], g["supported"]),
                   d["cwd"], d["model_id"],
                   Policy(PlanPolicy(d["policy"]["plan"]), ApprovalPolicy(d["policy"]["approval"])),
                   None if t is None else Turn(t["id"], TurnStatus(t["state"]), t["error"]),
                   d["working"], d["awaiting_approval"], d["approval_reason"], d["pending_count"],
                   tuple(Message(Role(m["role"]), m["content"]) for m in d["messages"]), d["streaming"])

@dataclass(frozen=True)
class StateEvent:
    seq: int
    state: State
    resynced: bool = False

@dataclass(frozen=True)
class OutputAppendEvent:
    seq: int
    revision: int
    offset: int
    text: str

@dataclass(frozen=True)
class TurnEvent:
    seq: int
    revision: int
    turn: Turn | None

Event = StateEvent | OutputAppendEvent | TurnEvent

@dataclass(frozen=True)
class Reason:
    code: str
    message: str

@dataclass(frozen=True)
class TurnResult:
    disposition: Disposition
    turn_id: str | None
    reason: Reason | None
    @classmethod
    def from_wire(cls, d: dict[str, Any]) -> TurnResult:
        r = d["reason"]
        if d["turn_id"] is not None and (not isinstance(d["turn_id"], str) or not d["turn_id"]):
            raise ValueError("invalid turn id")
        if r is not None and (not isinstance(r, dict) or not isinstance(r["code"], str) or not isinstance(r["message"], str)):
            raise ValueError("invalid rejection reason")
        return cls(Disposition(d["disposition"]), d["turn_id"], None if r is None else Reason(r["code"], r["message"]))

@dataclass(frozen=True)
class ReplacementResult(TurnResult):
    interrupted_turn_id: str | None

@dataclass(frozen=True)
class ApprovalResult:
    approved: bool

@dataclass(frozen=True)
class DetachResult:
    detached: bool

@dataclass(frozen=True)
class ImageFile:
    path: str
    def to_wire(self) -> dict[str, str]:
        import os
        if not os.path.isabs(self.path):
            raise ValueError("image path must be absolute on daemon host")
        return {"kind": "image_file", "path": self.path}

@dataclass(frozen=True)
class PastedText:
    text: str
    name: str | None = None
    def to_wire(self) -> dict[str, Any]:
        if len(self.text.encode("utf-8")) > 2 * 1024 * 1024:
            raise ValueError("pasted text exceeds 2 MiB")
        return {"kind": "pasted_text", "text": self.text, "name": self.name}

Attachment = ImageFile | PastedText


def _object(value: Any, what: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise ValueError(f"{what} must be object")
    return value

def _text(d: dict[str, Any], key: str, *, nonempty: bool = False) -> str:
    value = d[key]
    if not isinstance(value, str) or (nonempty and not value):
        raise ValueError(f"invalid {key}")
    return value

def _optional_text(d: dict[str, Any], key: str) -> str | None:
    value = d[key]
    if value is not None and not isinstance(value, str):
        raise ValueError(f"invalid {key}")
    return value

def _flag(d: dict[str, Any], key: str) -> bool:
    if type(d[key]) is not bool:
        raise ValueError(f"{key} must be boolean")
    return d[key]

def _count(d: dict[str, Any], key: str) -> int:
    value = d[key]
    if type(value) is not int or value < 0:
        raise ValueError(f"invalid {key}")
    return value

def _optional_count(d: dict[str, Any], key: str) -> int | None:
    return None if d[key] is None else _count(d, key)

def _optional_flag(d: dict[str, Any], key: str) -> bool | None:
    return None if d[key] is None else _flag(d, key)

def _number(d: dict[str, Any], key: str) -> float:
    value = d[key]
    if type(value) not in (int, float) or value < 0:
        raise ValueError(f"invalid {key}")
    return float(value)

def _texts(value: Any, what: str, most: int | None = None) -> tuple[str, ...]:
    if not isinstance(value, list) or not all(isinstance(v, str) for v in value):
        raise ValueError(f"{what} must be a list of strings")
    if most is not None and len(value) > most:
        raise ValueError(f"{what} exceeds {most} items")
    return tuple(value)

def _items(value: Any, what: str, most: int) -> list[Any]:
    if not isinstance(value, list) or len(value) > most:
        raise ValueError(f"{what} must be a list of at most {most} items")
    return value


class _OpenEnum(str, Enum):
    """An enum-like wire string that may gain values: unknown values map to `OTHER`."""
    @classmethod
    def _missing_(cls, value: object) -> Any:
        return cls.__members__.get("OTHER") if isinstance(value, str) else None

class PublicRole(str, Enum):
    """Closed: the five roles the SDK names."""
    MAIN = "main"
    AWARENESS = "awareness"
    SAFEGUARD = "safeguard"
    COMPACTOR = "compactor"
    PLANNER = "planner"

class ConfigFile(_OpenEnum):
    OK = "ok"
    MISSING = "missing"
    OTHER = "other"

class ConfigViewKind(_OpenEnum):
    GLOBAL = "global"
    SESSION = "session"
    OTHER = "other"

class MainFallback(_OpenEnum):
    UNCONFIGURED = "unconfigured"
    PROVIDER_REMOVED = "provider_removed"
    NO_KEY = "no_key"
    NOT_SIGNED_IN = "not_signed_in"
    OTHER = "other"

class KeyState(_OpenEnum):
    SET = "set"
    MISSING = "missing"
    UNREADABLE = "unreadable"
    OTHER = "other"

class ProviderKind(_OpenEnum):
    PROVIDER = "provider"
    OAUTH = "oauth"
    MISSING = "missing"
    OTHER = "other"

class RouteOrigin(_OpenEnum):
    SESSION = "session"
    GLOBAL = "global"
    INHERIT_MAIN = "inherit_main"
    LEGACY = "legacy"
    DEFAULT = "default"
    OTHER = "other"

class RouteReason(_OpenEnum):
    UNSUPPORTED_ROUTE = "unsupported_route"
    CREDENTIALS_UNAVAILABLE = "credentials_unavailable"
    PROVIDER_MISSING = "provider_missing"
    REFERENCE_DELETED = "reference_deleted"
    UNASSIGNED = "unassigned"
    OTHER = "other"

class EffortState(_OpenEnum):
    READY = "ready"
    LOADING = "loading"
    UNSUPPORTED = "unsupported"
    OTHER = "other"

class PropagationStatus(_OpenEnum):
    """Every value any `propagation` key reports; a `failed` value never means the write failed."""
    RELOADED = "reloaded"
    FAILED = "failed"
    NOT_BOUND = "not_bound"
    APPLIED_BY_DAEMON = "applied_by_daemon"
    SENT = "sent"
    SENT_BY_DAEMON = "sent_by_daemon"
    DONE = "done"
    NOT_NEEDED = "not_needed"
    DONE_BY_DAEMON = "done_by_daemon"
    NEXT_START = "next_start"
    NEXT_REBUILD = "next_rebuild"
    ROOM_RESTART = "room_restart"
    NOT_SUPPORTED = "not_supported"
    NOT_LIVE = "not_live"
    OTHER = "other"

def _open(kind: type[Any], d: dict[str, Any], key: str) -> Any:
    if not isinstance(d[key], str):
        raise ValueError(f"invalid {key}")
    return kind(d[key])

def _optional_open(kind: type[Any], d: dict[str, Any], key: str) -> Any:
    return None if d[key] is None else _open(kind, d, key)

def _roles(value: Any) -> tuple[PublicRole, ...]:
    roles = tuple(PublicRole(r) for r in _texts(value, "roles", 5))
    if len(set(roles)) != len(roles):
        raise ValueError("duplicate roles")
    return roles


@dataclass(frozen=True)
class Provider:
    """`endpoint` has URL userinfo, query and fragment removed; never feed a redacted one back to configure."""
    provider_uuid: str
    name: str
    api_type: str
    endpoint: str
    endpoint_redacted: bool
    managed: bool
    has_key: bool
    key_state: KeyState
    model_count: int
    @classmethod
    def from_wire(cls, value: Any) -> Provider:
        d = _object(value, "provider")
        return cls(_text(d, "provider_uuid", nonempty=True), _text(d, "name"), _text(d, "api_type"),
                   _text(d, "endpoint"), _flag(d, "endpoint_redacted"), _flag(d, "managed"),
                   _flag(d, "has_key"), _open(KeyState, d, "key_state"), _count(d, "model_count"))

@dataclass(frozen=True)
class OAuthConnection:
    oauth_uuid: str
    name: str
    provider: str
    has_token: bool
    expires_at: int
    @classmethod
    def from_wire(cls, value: Any) -> OAuthConnection:
        d = _object(value, "oauth connection")
        return cls(_text(d, "oauth_uuid", nonempty=True), _text(d, "name"), _text(d, "provider"),
                   _flag(d, "has_token"), _count(d, "expires_at"))

@dataclass(frozen=True)
class Model:
    """`other_roles` is true when the entry holds a role outside `PublicRole`; such roles are never named."""
    model_uuid: str
    name: str
    model_id: str
    provider_uuid: str
    provider_kind: ProviderKind
    provider_name: str | None
    route: str | None
    roles: tuple[PublicRole, ...]
    other_roles: bool
    source_uuid: str | None
    @classmethod
    def from_wire(cls, value: Any) -> Model:
        d = _object(value, "model")
        return cls(_text(d, "model_uuid", nonempty=True), _text(d, "name"), _text(d, "model_id"),
                   _text(d, "provider_uuid"), _open(ProviderKind, d, "provider_kind"),
                   _optional_text(d, "provider_name"), _optional_text(d, "route"), _roles(d["roles"]),
                   _flag(d, "other_roles"), _optional_text(d, "source_uuid"))

@dataclass(frozen=True)
class RoleRoute:
    role: PublicRole
    origin: RouteOrigin
    model_uuid: str | None
    source_uuid: str | None
    configured_model: str | None
    provider_uuid: str | None
    provider_name: str | None
    effective_model: str | None
    reason: RouteReason | None
    @classmethod
    def from_wire(cls, value: Any) -> RoleRoute:
        d = _object(value, "role route")
        return cls(PublicRole(d["role"]), _open(RouteOrigin, d, "origin"), _optional_text(d, "model_uuid"),
                   _optional_text(d, "source_uuid"), _optional_text(d, "configured_model"),
                   _optional_text(d, "provider_uuid"), _optional_text(d, "provider_name"),
                   _optional_text(d, "effective_model"), _optional_open(RouteReason, d, "reason"))

def _routes(value: Any) -> tuple[RoleRoute, ...]:
    routes = tuple(RoleRoute.from_wire(r) for r in _items(value, "routes", 5))
    if sorted(r.role.value for r in routes) != sorted(r.value for r in PublicRole):
        raise ValueError("routes must hold one entry per public role")
    return routes

@dataclass(frozen=True)
class ConfigView:
    """One consistent snapshot of config.json. Secrets are never included.

    A `*_truncated` list is incomplete; with `models_truncated` the role holders come first.
    """
    revision: str
    file: ConfigFile
    view: ConfigViewKind
    providers: tuple[Provider, ...]
    providers_truncated: bool
    oauth: tuple[OAuthConnection, ...]
    oauth_truncated: bool
    models: tuple[Model, ...]
    models_truncated: bool
    roles: tuple[RoleRoute, ...]
    main_fallback: MainFallback | None
    def route(self, role: PublicRole | str) -> RoleRoute:
        role = PublicRole(role)
        return next(r for r in self.roles if r.role is role)
    @classmethod
    def from_wire(cls, value: Any) -> ConfigView:
        d = _object(value, "config view")
        return cls(_text(d, "revision", nonempty=True), _open(ConfigFile, d, "file"),
                   _open(ConfigViewKind, d, "view"),
                   tuple(Provider.from_wire(p) for p in _items(d["providers"], "providers", 256)),
                   _flag(d, "providers_truncated"),
                   tuple(OAuthConnection.from_wire(o) for o in _items(d["oauth"], "oauth", 64)),
                   _flag(d, "oauth_truncated"),
                   tuple(Model.from_wire(m) for m in _items(d["models"], "models", 2000)),
                   _flag(d, "models_truncated"), _routes(d["roles"]),
                   _optional_open(MainFallback, d, "main_fallback"))

@dataclass(frozen=True)
class Limits:
    """`0` means auto or the catalogue default; `""` clears `context_model_alias`."""
    max_output_tokens: int
    context_window_limit: int
    context_model_alias: str
    effective_window: int | None
    @classmethod
    def from_wire(cls, value: Any) -> Limits:
        d = _object(value, "limits")
        window = d["effective_window"]
        if window is not None and (type(window) is not int or window < 0):
            raise ValueError("invalid effective_window")
        return cls(_count(d, "max_output_tokens"), _count(d, "context_window_limit"),
                   _text(d, "context_model_alias"), window)

@dataclass(frozen=True)
class UsageContext:
    prompt_tokens: int
    effective_window: int
    estimated: bool
    cached_tokens: int | None
    drss_active: bool

@dataclass(frozen=True)
class SessionUsage:
    last_prompt_tokens: int
    last_cached_tokens: int
    output_tokens_total: int
    cost_total: float
    context: UsageContext | None
    @classmethod
    def from_wire(cls, value: Any) -> SessionUsage:
        d = _object(value, "usage")
        cost = d["cost_total"]
        if type(cost) not in (int, float) or cost < 0:
            raise ValueError("invalid cost_total")
        c = d["context"]
        context = None
        if c is not None:
            c = _object(c, "usage context")
            cached = c["cached_tokens"]
            if cached is not None and (type(cached) is not int or cached < 0):
                raise ValueError("invalid cached_tokens")
            context = UsageContext(_count(c, "prompt_tokens"), _count(c, "effective_window"),
                                   _flag(c, "estimated"), cached, _flag(c, "drss_active"))
        return cls(_count(d, "last_prompt_tokens"), _count(d, "last_cached_tokens"),
                   _count(d, "output_tokens_total"), float(cost), context)

@dataclass(frozen=True)
class DaemonInfo:
    version: str
    ipc_protocol: int
    capabilities: tuple[str, ...]

@dataclass(frozen=True)
class SessionInfo:
    """Per-session facts that change without a state revision.

    Not ordered by `State.revision`: the newest is the one with the highest `observed_at_ms`.
    `effort` spells the model default `"default"`.
    """
    name: str
    effort: str
    routes: tuple[RoleRoute, ...]
    limits: Limits
    usage: SessionUsage | None
    daemon: DaemonInfo
    observed_at_ms: int
    def route(self, role: PublicRole | str) -> RoleRoute:
        role = PublicRole(role)
        return next(r for r in self.routes if r.role is role)
    @classmethod
    def from_wire(cls, value: Any) -> SessionInfo:
        d = _object(value, "session_info")
        daemon = _object(d["daemon"], "daemon")
        return cls(_text(d, "name"), _text(d, "effort", nonempty=True), _routes(d["routes"]),
                   Limits.from_wire(d["limits"]),
                   None if d["usage"] is None else SessionUsage.from_wire(d["usage"]),
                   DaemonInfo(_text(daemon, "version"), _count(daemon, "ipc_protocol"),
                              _texts(daemon["capabilities"], "daemon capabilities", 64)),
                   _count(d, "observed_at_ms"))

def _optional_session_info(d: dict[str, Any]) -> SessionInfo | None:
    return SessionInfo.from_wire(d["session_info"]) if "session_info" in d else None

@dataclass(frozen=True)
class Propagation:
    """What happened after a committed write. Absent keys are `None`; none of them is an error.

    `skills` is set by skill writes and `room` by team writes.
    """
    bound: PropagationStatus | None = None
    broadcast: PropagationStatus | None = None
    rebind: PropagationStatus | None = None
    rooms: PropagationStatus | None = None
    skills: PropagationStatus | None = None
    room: PropagationStatus | None = None
    @classmethod
    def from_wire(cls, value: Any) -> Propagation:
        d = _object(value, "propagation")
        return cls(*(None if key not in d else _open(PropagationStatus, d, key)
                     for key in ("bound", "broadcast", "rebind", "rooms", "skills", "room")))

@dataclass(frozen=True)
class ProviderRemoval:
    provider_uuid: str
    models_removed: tuple[str, ...]
    main_reset: bool

@dataclass(frozen=True)
class ProviderDeleteResult:
    """On a dry run nothing was written and `config` is the current view."""
    dry_run: bool
    removed: ProviderRemoval
    config: ConfigView
    propagation: Propagation
    session_info: SessionInfo | None
    @classmethod
    def from_wire(cls, value: Any) -> ProviderDeleteResult:
        d = _object(value, "result")
        r = _object(d["removed"], "removed")
        return cls(_flag(d, "dry_run"),
                   ProviderRemoval(_text(r, "provider_uuid", nonempty=True),
                                   _texts(r["models_removed"], "models_removed"), _flag(r, "main_reset")),
                   ConfigView.from_wire(d["config"]), Propagation.from_wire(d["propagation"]),
                   _optional_session_info(d))

@dataclass(frozen=True)
class ModelRemoval:
    model_uuid: str
    roles_released: tuple[PublicRole, ...]
    main_reset: bool

@dataclass(frozen=True)
class ModelDeleteResult:
    """On a dry run nothing was written and `config` is the current view."""
    dry_run: bool
    removed: ModelRemoval
    config: ConfigView
    propagation: Propagation
    session_info: SessionInfo | None
    @classmethod
    def from_wire(cls, value: Any) -> ModelDeleteResult:
        d = _object(value, "result")
        r = _object(d["removed"], "removed")
        return cls(_flag(d, "dry_run"),
                   ModelRemoval(_text(r, "model_uuid", nonempty=True), _roles(r["roles_released"]),
                                _flag(r, "main_reset")),
                   ConfigView.from_wire(d["config"]), Propagation.from_wire(d["propagation"]),
                   _optional_session_info(d))

@dataclass(frozen=True)
class RoleUnassignResult:
    changed: bool
    previous_model_uuid: str | None
    config: ConfigView
    propagation: Propagation
    session_info: SessionInfo | None
    @classmethod
    def from_wire(cls, value: Any) -> RoleUnassignResult:
        d = _object(value, "result")
        return cls(_flag(d, "changed"), _optional_text(d, "previous_model_uuid"),
                   ConfigView.from_wire(d["config"]), Propagation.from_wire(d["propagation"]),
                   _optional_session_info(d))

@dataclass(frozen=True)
class SkillRootsResult:
    """`roots` are the stored paths, in order. `changed` is false when the list already matched."""
    changed: bool
    roots: tuple[str, ...]
    config: ConfigView
    propagation: Propagation
    session_info: SessionInfo | None
    @classmethod
    def from_wire(cls, value: Any) -> SkillRootsResult:
        d = _object(value, "result")
        return cls(_flag(d, "changed"), _texts(d["roots"], "roots", 64),
                   ConfigView.from_wire(d["config"]), Propagation.from_wire(d["propagation"]),
                   _optional_session_info(d))

@dataclass(frozen=True)
class EffortOptions:
    """`options` starts with `"default"`; `selected` is `"default"` when the model default is used."""
    state: EffortState
    options: tuple[str, ...]
    selected: str
    note: str | None
    main_model_uuid: str | None
    @classmethod
    def from_wire(cls, value: Any) -> EffortOptions:
        d = _object(value, "effort options")
        return cls(_open(EffortState, d, "state"), _texts(d["options"], "options", 64),
                   _text(d, "selected", nonempty=True), _optional_text(d, "note"),
                   _optional_text(d, "main_model_uuid"))

@dataclass(frozen=True)
class EffortResult:
    """`superseded` means another writer set effort after this call; the stored effort is theirs."""
    effort: str
    main_model_uuid: str | None
    superseded: bool
    session_info: SessionInfo | None
    @classmethod
    def from_wire(cls, value: Any) -> EffortResult:
        d = _object(value, "effort result")
        return cls(_text(d, "effort", nonempty=True), _optional_text(d, "main_model_uuid"),
                   _flag(d, "superseded"), _optional_session_info(d))


# Sessions

class SessionKind(_OpenEnum):
    CODING = "coding"
    ROOM = "room"
    OTHER = "other"

@dataclass(frozen=True)
class CreateWarning:
    """A `create` that succeeded but could not finish a step, for example `rename_failed`."""
    code: str
    message: str
    @classmethod
    def from_wire(cls, value: Any) -> CreateWarning:
        d = _object(value, "warning")
        return cls(_text(d, "code", nonempty=True), _text(d, "message"))

def _create_warnings(d: dict[str, Any]) -> tuple[CreateWarning, ...]:
    return tuple(CreateWarning.from_wire(w) for w in _items(d.get("warnings", []), "warnings", 8))

@dataclass(frozen=True)
class SessionRow:
    """`working` is `None` for rooms; `bound` means the adapter you asked holds the session."""
    session_id: str
    name: str | None
    workdir: str | None
    kind: SessionKind
    created_at: int | None
    updated_at: int | None
    live: bool
    working: bool | None
    locked: bool
    bound: bool
    parent_session_id: str | None
    room_session_id: str | None
    room_live: bool
    @classmethod
    def from_wire(cls, value: Any) -> SessionRow:
        d = _object(value, "session")
        return cls(_text(d, "session_id", nonempty=True), _optional_text(d, "name"), _optional_text(d, "workdir"),
                   _open(SessionKind, d, "kind"), _optional_count(d, "created_at"), _optional_count(d, "updated_at"),
                   _flag(d, "live"), _optional_flag(d, "working"), _flag(d, "locked"), _flag(d, "bound"),
                   _optional_text(d, "parent_session_id"), _optional_text(d, "room_session_id"),
                   _flag(d, "room_live"))

@dataclass(frozen=True)
class SessionPage:
    sessions: tuple[SessionRow, ...]
    next_cursor: str | None
    @classmethod
    def from_wire(cls, value: Any) -> SessionPage:
        d = _object(value, "result")
        return cls(tuple(SessionRow.from_wire(r) for r in _items(d["sessions"], "sessions", 500)),
                   _optional_text(d, "next_cursor"))

@dataclass(frozen=True)
class SessionRenameResult:
    session: SessionRow
    @classmethod
    def from_wire(cls, value: Any) -> SessionRenameResult:
        return cls(SessionRow.from_wire(_object(value, "result")["session"]))

@dataclass(frozen=True)
class SessionDeleteResult:
    """`orphaned_room_session_id` names a room whose parent was just deleted; it is not removed."""
    deleted: bool
    kind: SessionKind
    orphaned_room_session_id: str | None
    @classmethod
    def from_wire(cls, value: Any) -> SessionDeleteResult:
        d = _object(value, "result")
        if d["deleted"] is not True:
            raise ValueError("delete not acknowledged")
        return cls(True, _open(SessionKind, d, "kind"), _optional_text(d, "orphaned_room_session_id"))

@dataclass(frozen=True)
class SessionStopResult:
    stopped: bool
    was_live: bool
    @classmethod
    def from_wire(cls, value: Any) -> SessionStopResult:
        d = _object(value, "result")
        if d["stopped"] is not True:
            raise ValueError("stop not acknowledged")
        return cls(True, _flag(d, "was_live"))

# Usage

class LedgerState(_OpenEnum):
    OK = "ok"
    MISSING = "missing"
    OTHER = "other"

class UsageRoleKind(_OpenEnum):
    MAIN = "main"
    SUB = "sub"
    OTHER = "other"

@dataclass(frozen=True)
class UsageTotals:
    """`tokens_in` is the per-call prompt size summed over calls."""
    calls: int
    tokens_in: int
    tokens_cached: int
    tokens_out: int
    cost: float
    @classmethod
    def from_wire(cls, value: Any) -> UsageTotals:
        d = _object(value, "usage totals")
        return cls(*(_count(d, k) for k in ("calls", "tokens_in", "tokens_cached", "tokens_out")), _number(d, "cost"))

@dataclass(frozen=True)
class UsageGroup(UsageTotals):
    """`key` is a model id, a raw role (`main`, `sub:<agent>`) or a session id, by `group_by`."""
    key: str
    label: str | None
    role_kind: UsageRoleKind | None = None
    agent: str | None = None
    kind: SessionKind | None = None
    deleted: bool | None = None
    @classmethod
    def from_wire(cls, value: Any) -> UsageGroup:
        d = _object(value, "usage group")
        t = UsageTotals.from_wire(d)
        return cls(t.calls, t.tokens_in, t.tokens_cached, t.tokens_out, t.cost,
                   _text(d, "key", nonempty=True), _optional_text(d, "label"),
                   _open(UsageRoleKind, d, "role_kind") if "role_kind" in d else None,
                   _optional_text(d, "agent") if "agent" in d else None,
                   _optional_open(SessionKind, d, "kind") if "kind" in d else None,
                   _flag(d, "deleted") if "deleted" in d else None)

@dataclass(frozen=True)
class UsageBucket(UsageTotals):
    start_at: int
    @classmethod
    def from_wire(cls, value: Any) -> UsageBucket:
        d = _object(value, "usage bucket")
        t = UsageTotals.from_wire(d)
        return cls(t.calls, t.tokens_in, t.tokens_cached, t.tokens_out, t.cost, _count(d, "start_at"))

@dataclass(frozen=True)
class UsageReport:
    """`groups` is set when `group_by` was given, `buckets` when `bucket` was; otherwise `None`.

    A `ledger` of `missing` means no usage was ever recorded; it is not an error.
    """
    from_at: int
    to_at: int
    ledger: LedgerState
    totals: UsageTotals
    groups: tuple[UsageGroup, ...] | None
    groups_truncated: bool
    buckets: tuple[UsageBucket, ...] | None
    @classmethod
    def from_wire(cls, value: Any) -> UsageReport:
        d = _object(value, "usage result")
        return cls(_count(d, "from_at"), _count(d, "to_at"), _open(LedgerState, d, "ledger"),
                   UsageTotals.from_wire(d["totals"]),
                   tuple(UsageGroup.from_wire(g) for g in _items(d["groups"], "groups", 500)) if "groups" in d else None,
                   _flag(d, "groups_truncated") if "groups_truncated" in d else False,
                   tuple(UsageBucket.from_wire(b) for b in _items(d["buckets"], "buckets", 1000))
                   if "buckets" in d else None)

# Skills

class SkillScope(_OpenEnum):
    COMPUTER = "computer"
    PROJECT = "project"
    EXTERNAL = "external"
    OTHER = "other"

def _skill_row_args(d: dict[str, Any]) -> tuple[Any, ...]:
    return (_text(d, "name", nonempty=True), _open(SkillScope, d, "scope"), _text(d, "tier", nonempty=True),
            _text(d, "description"), _texts(d["triggers"], "triggers", 64), _flag(d, "triggers_truncated"),
            _texts(d["tags"], "tags", 64), _flag(d, "tags_truncated"), _flag(d, "pinned"), _flag(d, "editable"),
            _text(d, "generation"), _text(d, "path"), _flag(d, "effective"), _optional_text(d, "shadowed_by"))

@dataclass(frozen=True)
class SkillRow:
    """`(tier, name)` addresses a skill; `generation` is the optimistic token for writes.

    `tier` is `global`, `claude`, `project-agent`, `project-agents` or `extra-root:<n>`; `path` is for display.
    """
    name: str
    scope: SkillScope
    tier: str
    description: str
    triggers: tuple[str, ...]
    triggers_truncated: bool
    tags: tuple[str, ...]
    tags_truncated: bool
    pinned: bool
    editable: bool
    generation: str
    path: str
    effective: bool
    shadowed_by: str | None
    @classmethod
    def from_wire(cls, value: Any) -> SkillRow:
        return cls(*_skill_row_args(_object(value, "skill")))

@dataclass(frozen=True)
class SkillDetail(SkillRow):
    """`raw` is `None` unless `include_raw` was asked for, and always `None` in write results."""
    instruction: str
    declared_tools: tuple[str, ...]
    declared_tools_truncated: bool
    companion_files: tuple[str, ...]
    companion_files_truncated: bool
    structured_save_supported: bool
    raw: str | None
    @classmethod
    def from_wire(cls, value: Any) -> SkillDetail:
        d = _object(value, "skill")
        return cls(*_skill_row_args(d), _text(d, "instruction"), _texts(d["declared_tools"], "declared_tools", 64),
                   _flag(d, "declared_tools_truncated"), _texts(d["companion_files"], "companion_files", 1000),
                   _flag(d, "companion_files_truncated"), _flag(d, "structured_save_supported"),
                   _optional_text(d, "raw"))

@dataclass(frozen=True)
class SkillDiagnostic:
    path: str
    error: str

@dataclass(frozen=True)
class SkillPage:
    """`project_workdir` is the canonical workdir the project tiers were read from, if any."""
    project_workdir: str | None
    skills: tuple[SkillRow, ...]
    diagnostics: tuple[SkillDiagnostic, ...]
    diagnostics_truncated: bool
    next_cursor: str | None
    @classmethod
    def from_wire(cls, value: Any) -> SkillPage:
        d = _object(value, "result")
        diagnostics = []
        for item in _items(d["diagnostics"], "diagnostics", 200):
            item = _object(item, "diagnostic")
            diagnostics.append(SkillDiagnostic(_text(item, "path"), _text(item, "error")))
        return cls(_optional_text(d, "project_workdir"),
                   tuple(SkillRow.from_wire(r) for r in _items(d["skills"], "skills", 1000)),
                   tuple(diagnostics), _flag(d, "diagnostics_truncated"), _optional_text(d, "next_cursor"))

@dataclass(frozen=True)
class SkillFile:
    path: str
    text: str
    bytes: int
    @classmethod
    def from_wire(cls, value: Any) -> SkillFile:
        d = _object(value, "result")
        return cls(_text(d, "path", nonempty=True), _text(d, "text"), _count(d, "bytes"))

@dataclass(frozen=True)
class SkillWriteResult:
    """`propagation.skills` is `next_rebuild` (a running session sees the change at its next system-prompt
    rebuild) or `room_restart` (a room reads its skills when it starts)."""
    skill: SkillDetail
    propagation: Propagation
    @classmethod
    def from_wire(cls, value: Any) -> SkillWriteResult:
        d = _object(value, "result")
        return cls(SkillDetail.from_wire(d["skill"]), Propagation.from_wire(d["propagation"]))

@dataclass(frozen=True)
class SkillUpdateResult:
    skill: SkillDetail
    renamed: bool
    propagation: Propagation
    @classmethod
    def from_wire(cls, value: Any) -> SkillUpdateResult:
        d = _object(value, "result")
        return cls(SkillDetail.from_wire(d["skill"]), _flag(d, "renamed"), Propagation.from_wire(d["propagation"]))

@dataclass(frozen=True)
class SkillRemoveResult:
    """`now_effective` is the skill that wins by name after the removal, if one is left."""
    removed: bool
    now_effective: SkillRow | None
    propagation: Propagation
    @classmethod
    def from_wire(cls, value: Any) -> SkillRemoveResult:
        d = _object(value, "result")
        if d["removed"] is not True:
            raise ValueError("remove not acknowledged")
        return cls(True, None if d["now_effective"] is None else SkillRow.from_wire(d["now_effective"]),
                   Propagation.from_wire(d["propagation"]))
