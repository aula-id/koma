"""Immutable DTOs for the room: listing, runtime, shelf, team and events. Enum-like strings are open."""
from __future__ import annotations
from dataclasses import dataclass
from typing import Any, Union

from .types import (Propagation, _OpenEnum, _count, _flag, _items, _number, _object, _open, _optional_count,
                    _optional_flag, _optional_open, _optional_text, _text, _texts, UsageTotals)


class LineKind(_OpenEnum):
    CHIEF = "chief"
    LANE_CHAT = "lane_chat"
    VISIBLE = "visible"
    USER = "user"
    SHELL = "shell"
    REASONING = "reasoning"
    TOOL_CALL = "tool_call"
    TOOL_RESULT = "tool_result"
    OTHER = "other"

class LaneName(_OpenEnum):
    CHAT = "chat"
    WORK = "work"
    OTHER = "other"

class Audience(_OpenEnum):
    USER = "user"
    TEAM = "team"
    OTHER = "other"

class LineOrigin(_OpenEnum):
    ROOM = "room"
    ADAPTER_ECHO = "adapter_echo"
    OTHER = "other"

class LanePhase(_OpenEnum):
    RUNNING = "running"
    IDLE = "idle"
    PAUSED = "paused"
    SUBMITTED = "submitted"
    DONE = "done"
    FAILED = "failed"
    OTHER = "other"

class CountsBasis(_OpenEnum):
    FIRST_400_ITEMS = "first_400_items"
    DISK_SCAN = "disk_scan"
    OTHER = "other"

class DeltaKind(_OpenEnum):
    TEXT = "text"
    REASONING = "reasoning"
    OTHER = "other"

class NoticeCode(_OpenEnum):
    ROOM = "room"
    STALLED = "stalled"
    RECOVERED = "recovered"
    LAYER_WARNINGS = "layer_warnings"
    OTHER = "other"

class CloseReason(_OpenEnum):
    TAKEN_OVER = "taken_over"
    DAEMON_EXIT = "daemon_exit"
    STREAM_ERROR = "stream_error"
    OTHER = "other"

class TeamTarget(_OpenEnum):
    TEMPLATE = "template"
    ROOM = "room"
    OTHER = "other"

class Origin(_OpenEnum):
    BASE = "base"
    TEMPLATE = "template"
    SESSION = "session"
    ROLE = "role"
    DEFAULT = "default"
    OTHER = "other"

class Layer(_OpenEnum):
    BASE = "base"
    TEMPLATE = "template"
    SESSION = "session"
    OTHER = "other"

class ChoicesSource(_OpenEnum):
    ROOM_DAEMON = "room_daemon"
    CONFIG = "config"
    OTHER = "other"

class Applies(_OpenEnum):
    """When a team write takes effect: `live` (the next lane turn), `next_reload` or `next_start`."""
    LIVE = "live"
    NEXT_RELOAD = "next_reload"
    NEXT_START = "next_start"
    OTHER = "other"


@dataclass(frozen=True)
class RoomInfo:
    session_id: str
    parent_session_id: str | None
    parent_name: str | None
    name: str | None
    workdir: str | None
    live: bool
    locked: bool
    bound: bool
    created_at: int | None
    updated_at: int | None
    @classmethod
    def from_wire(cls, value: Any) -> RoomInfo:
        d = _object(value, "room")
        return cls(_text(d, "session_id", nonempty=True), _optional_text(d, "parent_session_id"),
                   _optional_text(d, "parent_name"), _optional_text(d, "name"), _optional_text(d, "workdir"),
                   _flag(d, "live"), _flag(d, "locked"), _flag(d, "bound"),
                   _optional_count(d, "created_at"), _optional_count(d, "updated_at"))

@dataclass(frozen=True)
class RoomPage:
    rooms: tuple[RoomInfo, ...]
    next_cursor: str | None
    @classmethod
    def from_wire(cls, value: Any) -> RoomPage:
        d = _object(value, "result")
        return cls(tuple(RoomInfo.from_wire(r) for r in _items(d["rooms"], "rooms", 500)),
                   _optional_text(d, "next_cursor"))

@dataclass(frozen=True)
class RoomLine:
    """One line of the room's history. Identified by `(epoch, line_seq)`; `line_seq` restarts at 1 per epoch.

    `text` is set for chief, lane_chat, visible, user, shell and reasoning lines; `call_id`, `tool` and `args`
    for a tool call; `call_id`, `ok` and `result` for its result. An unknown kind is `LineKind.OTHER` with
    the raw JSON in `text`.
    """
    line_seq: int
    kind: LineKind
    author: str
    display: str | None
    alias: str | None
    lane: LaneName | None
    audience: Audience | None
    text: str | None
    call_id: str | None
    tool: str | None
    args: str | None
    ok: bool | None
    result: str | None
    replay: bool
    origin: LineOrigin
    received_at_ms: int
    truncated: bool
    @classmethod
    def from_wire(cls, value: Any) -> RoomLine:
        d = _object(value, "line")
        seq = _count(d, "line_seq")
        if seq < 1:
            raise ValueError("invalid line_seq")
        return cls(seq, _open(LineKind, d, "kind"), _text(d, "author", nonempty=True), _optional_text(d, "display"),
                   _optional_text(d, "alias"), _optional_open(LaneName, d, "lane"),
                   _optional_open(Audience, d, "audience"), _optional_text(d, "text"), _optional_text(d, "call_id"),
                   _optional_text(d, "tool"), _optional_text(d, "args"), _optional_flag(d, "ok"),
                   _optional_text(d, "result"), _flag(d, "replay"), _open(LineOrigin, d, "origin"),
                   _count(d, "received_at_ms"), _flag(d, "truncated"))

@dataclass(frozen=True)
class RoomCounts:
    open: int
    verified: int
    returned: int
    reports: int
    @classmethod
    def from_wire(cls, value: Any) -> RoomCounts:
        d = _object(value, "counts")
        return cls(_count(d, "open"), _count(d, "verified"), _count(d, "returned"), _count(d, "reports"))

@dataclass(frozen=True)
class LaneStatus:
    avatar: str
    display: str
    alias: str
    role: str
    lane: LaneName | None
    phase: LanePhase
    task: str
    started_at_ms: int
    last_event_at_ms: int
    steps: int
    last_tool: str
    error: str
    @classmethod
    def from_wire(cls, value: Any) -> LaneStatus:
        d = _object(value, "lane")
        return cls(_text(d, "avatar", nonempty=True), _text(d, "display"), _text(d, "alias"), _text(d, "role"),
                   _optional_open(LaneName, d, "lane"), _open(LanePhase, d, "phase"), _text(d, "task"),
                   _count(d, "started_at_ms"), _count(d, "last_event_at_ms"), _count(d, "steps"),
                   _text(d, "last_tool"), _text(d, "error"))

@dataclass(frozen=True)
class TimelineEvent:
    at_ms: int
    avatar: str
    marker: str
    label: str
    @classmethod
    def from_wire(cls, value: Any) -> TimelineEvent:
        d = _object(value, "timeline event")
        return cls(_count(d, "at_ms"), _text(d, "avatar"), _text(d, "marker"), _text(d, "label"))

@dataclass(frozen=True)
class RoomStatus:
    """The room's latest status. `fresh` is false until a status arrived after the attach.

    Text fields (`digest`, `timeline`, `trajectory_outline`) are terminal-rendered; `truncated` says one was cut.
    A list whose `*_truncated` flag is set is incomplete.
    """
    session_id: str
    live: bool
    fresh: bool
    age_ms: int | None
    now_at_ms: int
    counts: RoomCounts
    counts_basis: CountsBasis
    paused: tuple[str, ...]
    paused_truncated: bool
    lanes: tuple[LaneStatus, ...]
    lanes_truncated: bool
    events: tuple[TimelineEvent, ...]
    digest: str
    timeline: str
    trajectory_outline: str
    shelf_fingerprint: str
    truncated: bool
    @classmethod
    def from_wire(cls, value: Any) -> RoomStatus:
        d = _object(value, "status")
        return cls(_text(d, "session_id", nonempty=True), _flag(d, "live"), _flag(d, "fresh"),
                   _optional_count(d, "age_ms"), _count(d, "now_at_ms"), RoomCounts.from_wire(d["counts"]),
                   _open(CountsBasis, d, "counts_basis"), _texts(d["paused"], "paused", 64),
                   _flag(d, "paused_truncated"),
                   tuple(LaneStatus.from_wire(l) for l in _items(d["lanes"], "lanes", 128)),
                   _flag(d, "lanes_truncated"),
                   tuple(TimelineEvent.from_wire(e) for e in _items(d["events"], "events", 60)),
                   _text(d, "digest"), _text(d, "timeline"), _text(d, "trajectory_outline"),
                   _text(d, "shelf_fingerprint"), _flag(d, "truncated"))

@dataclass(frozen=True)
class RoomDaemon:
    """`features` is empty for an older room; `sdk_user` and `sdk_reload` are the ones the SDK uses."""
    version: str | None
    features: tuple[str, ...]
    template_revision: str | None

@dataclass(frozen=True)
class HistoryRange:
    """The adapter's line ring at attach: `count` lines, `replayed` of them replayed by the room."""
    first_line_seq: int | None
    last_line_seq: int | None
    count: int
    replayed: int

@dataclass(frozen=True)
class AttachResult:
    """Render `tail`, page older lines with `Room.history(before_line_seq=...)`. `epoch` names this attach."""
    room: RoomInfo
    started: bool
    daemon: RoomDaemon
    epoch: str
    history: HistoryRange
    tail: tuple[RoomLine, ...]
    status: RoomStatus | None
    roster: tuple[str, ...]
    roster_truncated: bool
    @classmethod
    def from_wire(cls, value: Any) -> AttachResult:
        d = _object(value, "attach result")
        daemon, history = _object(d["daemon"], "daemon"), _object(d["history"], "history")
        return cls(RoomInfo.from_wire(d["room"]), _flag(d, "started"),
                   RoomDaemon(_optional_text(daemon, "version"), _texts(daemon["features"], "features", 64),
                              _optional_text(daemon, "template_revision")),
                   _text(d, "epoch", nonempty=True),
                   HistoryRange(_optional_count(history, "first_line_seq"), _optional_count(history, "last_line_seq"),
                                _count(history, "count"), _count(history, "replayed")),
                   tuple(RoomLine.from_wire(l) for l in _items(d["tail"], "tail", 200)),
                   None if d["status"] is None else RoomStatus.from_wire(d["status"]),
                   _texts(d["roster"], "roster", 64), _flag(d, "roster_truncated"))

@dataclass(frozen=True)
class HistoryPage:
    """Lines in ascending `line_seq`. `first_line_seq` and `last_line_seq` describe the whole ring."""
    epoch: str
    first_line_seq: int | None
    last_line_seq: int | None
    lines: tuple[RoomLine, ...]
    has_more_before: bool
    has_more_after: bool
    @classmethod
    def from_wire(cls, value: Any) -> HistoryPage:
        d = _object(value, "history")
        return cls(_text(d, "epoch", nonempty=True), _optional_count(d, "first_line_seq"),
                   _optional_count(d, "last_line_seq"),
                   tuple(RoomLine.from_wire(l) for l in _items(d["lines"], "lines", 200)),
                   _flag(d, "has_more_before"), _flag(d, "has_more_after"))

@dataclass(frozen=True)
class SendResult:
    """`acked` is false when the room predates acknowledgements: the text was written, nothing confirmed it."""
    sent: bool
    acked: bool
    inserts: tuple[str, ...]
    echo_line_seq: int
    @classmethod
    def from_wire(cls, value: Any) -> SendResult:
        d = _object(value, "send result")
        if d["sent"] is not True:
            raise ValueError("send not acknowledged")
        seq = _count(d, "echo_line_seq")
        if seq < 1:
            raise ValueError("invalid echo_line_seq")
        return cls(True, _flag(d, "acked"), _texts(d["inserts"], "inserts", 16), seq)

@dataclass(frozen=True)
class SentResult:
    """The frame was written to the room; the effect shows up as lines and status."""
    sent: bool
    @classmethod
    def from_wire(cls, value: Any) -> SentResult:
        if _object(value, "result")["sent"] is not True:
            raise ValueError("send not acknowledged")
        return cls(True)

@dataclass(frozen=True)
class CloseResult:
    closed: bool
    stopped: bool
    @classmethod
    def from_wire(cls, value: Any) -> CloseResult:
        d = _object(value, "result")
        if d["closed"] is not True:
            raise ValueError("close not acknowledged")
        return cls(True, _flag(d, "stopped"))

# Shelf, trajectory, reports, spend

@dataclass(frozen=True)
class ShelfRow:
    key: str
    revision: int
    updated_at: int
    kind: str
    title: str
    status: str
    @classmethod
    def from_wire(cls, value: Any) -> ShelfRow:
        d = _object(value, "shelf row")
        return cls(*_shelf_args(d))

def _shelf_args(d: dict[str, Any]) -> tuple[Any, ...]:
    return (_text(d, "key", nonempty=True), _count(d, "revision"), _count(d, "updated_at"), _text(d, "kind"),
            _text(d, "title"), _text(d, "status"))

@dataclass(frozen=True)
class ShelfItem(ShelfRow):
    """`json` is the parsed record when the content is JSON; `truncated` says `content` was cut."""
    content: str
    json: dict[str, Any] | None
    truncated: bool
    @classmethod
    def from_wire(cls, value: Any) -> ShelfItem:
        d = _object(value, "shelf item")
        data = d["json"]
        if data is not None and not isinstance(data, dict):
            raise ValueError("invalid json")
        return cls(*_shelf_args(d), _text(d, "content"), data, _flag(d, "truncated"))

@dataclass(frozen=True)
class ShelfPage:
    """`fingerprint` changes when the shelf does: refetch the shelf, trajectory and reports when it moves."""
    rows: tuple[ShelfRow, ...]
    total: int
    next_cursor: str | None
    fingerprint: str
    @classmethod
    def from_wire(cls, value: Any) -> ShelfPage:
        d = _object(value, "result")
        return cls(tuple(ShelfRow.from_wire(r) for r in _items(d["rows"], "rows", 500)), _count(d, "total"),
                   _optional_text(d, "next_cursor"), _text(d, "fingerprint"))

@dataclass(frozen=True)
class GoalNode:
    id: str
    label: str
    kind: str
    status: str | None
    severity: str | None
    note: str | None
    resource_key: str | None
    revision: int | None
    rank: int
    row: int
    @classmethod
    def from_wire(cls, value: Any) -> GoalNode:
        d = _object(value, "node")
        rev, rank, row = d["revision"], d["rank"], d["row"]
        if (rev is not None and type(rev) is not int) or type(rank) is not int or type(row) is not int:
            raise ValueError("invalid node number")
        return cls(_text(d, "id", nonempty=True), _text(d, "label"), _text(d, "kind"), _optional_text(d, "status"),
                   _optional_text(d, "severity"), _optional_text(d, "note"), _optional_text(d, "resource_key"),
                   rev, rank, row)

@dataclass(frozen=True)
class GoalEdge:
    source: str
    target: str
    rel: str | None
    @classmethod
    def from_wire(cls, value: Any) -> GoalEdge:
        d = _object(value, "edge")
        return cls(_text(d, "from", nonempty=True), _text(d, "to", nonempty=True), _optional_text(d, "rel"))

@dataclass(frozen=True)
class Goal:
    """One goal's trajectory graph. `*_truncated` says nodes or edges were capped."""
    goal_key: str
    counts: RoomCounts
    columns: int
    rows: int
    nodes: tuple[GoalNode, ...]
    nodes_truncated: bool
    edges: tuple[GoalEdge, ...]
    edges_truncated: bool
    @classmethod
    def from_wire(cls, value: Any) -> Goal:
        d = _object(value, "goal")
        return cls(_text(d, "goal_key", nonempty=True), RoomCounts.from_wire(d["counts"]), _count(d, "columns"),
                   _count(d, "rows"), tuple(GoalNode.from_wire(n) for n in _items(d["nodes"], "nodes", 2000)),
                   _flag(d, "nodes_truncated"), tuple(GoalEdge.from_wire(e) for e in _items(d["edges"], "edges", 8000)),
                   _flag(d, "edges_truncated"))

@dataclass(frozen=True)
class Trajectory:
    goals: tuple[Goal, ...]
    goals_truncated: bool
    @classmethod
    def from_wire(cls, value: Any) -> Trajectory:
        d = _object(value, "result")
        return cls(tuple(Goal.from_wire(g) for g in _items(d["goals"], "goals", 200)), _flag(d, "goals_truncated"))

@dataclass(frozen=True)
class Report:
    """`severity_summary` and `go_no_go` are free text from the shelf record, not enums."""
    key: str
    revision: int
    updated_at: int
    title: str
    audience: str
    severity_summary: str
    go_no_go: str
    finding_keys: tuple[str, ...]
    finding_keys_truncated: bool
    markdown: str | None
    @classmethod
    def from_wire(cls, value: Any) -> Report:
        d = _object(value, "report")
        return cls(_text(d, "key", nonempty=True), _count(d, "revision"), _count(d, "updated_at"), _text(d, "title"),
                   _text(d, "audience"), _text(d, "severity_summary"), _text(d, "go_no_go"),
                   _texts(d["finding_keys"], "finding_keys", 1000), _flag(d, "finding_keys_truncated"),
                   _optional_text(d, "markdown"))

@dataclass(frozen=True)
class ReportPage:
    reports: tuple[Report, ...]
    next_cursor: str | None
    @classmethod
    def from_wire(cls, value: Any) -> ReportPage:
        d = _object(value, "result")
        return cls(tuple(Report.from_wire(r) for r in _items(d["reports"], "reports", 100)),
                   _optional_text(d, "next_cursor"))

@dataclass(frozen=True)
class SpendEntry:
    calls: int
    prompt_tokens: int
    completion_tokens: int
    cost: float
    @classmethod
    def from_wire(cls, value: Any) -> SpendEntry:
        d = _object(value, "spend")
        return cls(_count(d, "calls"), _count(d, "prompt_tokens"), _count(d, "completion_tokens"), _number(d, "cost"))

@dataclass(frozen=True)
class AvatarSpend(SpendEntry):
    avatar: str
    @classmethod
    def from_wire(cls, value: Any) -> AvatarSpend:
        d = _object(value, "avatar spend")
        e = SpendEntry.from_wire(d)
        return cls(e.calls, e.prompt_tokens, e.completion_tokens, e.cost, _text(d, "avatar", nonempty=True))

@dataclass(frozen=True)
class GoalSpend:
    goal_key: str
    total: SpendEntry
    by_avatar: tuple[AvatarSpend, ...]
    by_avatar_truncated: bool
    @classmethod
    def from_wire(cls, value: Any) -> GoalSpend:
        d = _object(value, "goal spend")
        return cls(_text(d, "goal_key", nonempty=True), SpendEntry.from_wire(d["total"]),
                   tuple(AvatarSpend.from_wire(a) for a in _items(d["by_avatar"], "by_avatar", 64)),
                   _flag(d, "by_avatar_truncated"))

@dataclass(frozen=True)
class LedgerAvatar:
    avatar: str
    totals: UsageTotals

@dataclass(frozen=True)
class SpendLedger:
    """What the usage ledger recorded for the room, by avatar (`chief` is the room's Main)."""
    ledger: str
    totals: UsageTotals
    by_avatar: tuple[LedgerAvatar, ...]
    by_avatar_truncated: bool

@dataclass(frozen=True)
class RoomSpend:
    goals: tuple[GoalSpend, ...]
    goals_truncated: bool
    ledger: SpendLedger
    @classmethod
    def from_wire(cls, value: Any) -> RoomSpend:
        d = _object(value, "result")
        led = _object(d["ledger"], "ledger")
        by_avatar = []
        for item in _items(led["by_avatar"], "by_avatar", 64):
            item = _object(item, "ledger avatar")
            by_avatar.append(LedgerAvatar(_text(item, "avatar", nonempty=True), UsageTotals.from_wire(item["totals"])))
        return cls(tuple(GoalSpend.from_wire(g) for g in _items(d["goals"], "goals", 200)),
                   _flag(d, "goals_truncated"),
                   SpendLedger(_text(led, "ledger", nonempty=True), UsageTotals.from_wire(led["totals"]),
                               tuple(by_avatar), _flag(led, "by_avatar_truncated")))

# Team

@dataclass(frozen=True)
class TeamField:
    """A value and the layer it comes from; `value` is `None` when unset everywhere."""
    value: str | None
    origin: Origin
    @classmethod
    def from_wire(cls, value: Any) -> TeamField:
        d = _object(value, "field")
        return cls(_optional_text(d, "value"), _open(Origin, d, "origin"))

@dataclass(frozen=True)
class LaneView:
    """`resolves` false means the lane's model is not among the choices and the lane uses the Main route."""
    lane: LaneName
    provider_uuid: TeamField
    model: TeamField
    effort: TeamField
    model_label: str | None
    resolves: bool
    @classmethod
    def from_wire(cls, value: Any) -> LaneView:
        d = _object(value, "lane")
        return cls(_open(LaneName, d, "lane"), TeamField.from_wire(d["provider_uuid"]), TeamField.from_wire(d["model"]),
                   TeamField.from_wire(d["effort"]), _optional_text(d, "model_label"), _flag(d, "resolves"))

@dataclass(frozen=True)
class AvatarView:
    id: str
    role: str
    defined_in: Layer
    removable: bool
    display: TeamField
    display_effective: str
    alias: TeamField
    alias_effective: str
    lanes: tuple[LaneView, ...]
    @classmethod
    def from_wire(cls, value: Any) -> AvatarView:
        d = _object(value, "avatar")
        return cls(_text(d, "id", nonempty=True), _text(d, "role", nonempty=True), _open(Layer, d, "defined_in"),
                   _flag(d, "removable"), TeamField.from_wire(d["display"]), _text(d, "display_effective"),
                   TeamField.from_wire(d["alias"]), _text(d, "alias_effective"),
                   tuple(LaneView.from_wire(l) for l in _items(d["lanes"], "lanes", 2)))

@dataclass(frozen=True)
class RoleLane:
    provider_uuid: str | None
    model: str | None
    effort: str | None
    @classmethod
    def from_wire(cls, value: Any) -> RoleLane:
        d = _object(value, "role lane")
        return cls(_optional_text(d, "provider_uuid"), _optional_text(d, "model"), _optional_text(d, "effort"))

@dataclass(frozen=True)
class RoleView:
    """Roles are read-only. `skills_allow` is `None` when unrestricted."""
    id: str
    display: str
    shelf_allow: tuple[str, ...]
    skills_allow: tuple[str, ...] | None
    chat: RoleLane
    work: RoleLane
    defined_in: Layer
    @classmethod
    def from_wire(cls, value: Any) -> RoleView:
        d = _object(value, "role")
        return cls(_text(d, "id", nonempty=True), _text(d, "display"), _texts(d["shelf_allow"], "shelf_allow"),
                   None if d["skills_allow"] is None else _texts(d["skills_allow"], "skills_allow"),
                   RoleLane.from_wire(d["chat"]), RoleLane.from_wire(d["work"]), _open(Layer, d, "defined_in"))

@dataclass(frozen=True)
class ModelChoice:
    id: str
    name: str
    model_id: str
    provider_name: str | None
    provider_uuid: str | None
    @classmethod
    def from_wire(cls, value: Any) -> ModelChoice:
        d = _object(value, "choice")
        return cls(_text(d, "id", nonempty=True), _text(d, "name"), _text(d, "model_id"),
                   _optional_text(d, "provider_name"), _optional_text(d, "provider_uuid"))

@dataclass(frozen=True)
class TeamWarning:
    layer: Layer
    path: str
    message: str
    @classmethod
    def from_wire(cls, value: Any) -> TeamWarning:
        d = _object(value, "warning")
        return cls(_open(Layer, d, "layer"), _text(d, "path"), _text(d, "message"))

@dataclass(frozen=True)
class Team:
    """The effective team for a target. `revision` is the target layer's; pass it as `if_revision` to guard a write.

    For the `room` target, `template_pending_restart` means the template changed since the room started.
    """
    target: TeamTarget
    revision: str
    layer_present: bool
    template_revision: str
    template_pending_restart: bool
    avatars: tuple[AvatarView, ...]
    avatars_truncated: bool
    roles: tuple[RoleView, ...]
    roles_truncated: bool
    choices: tuple[ModelChoice, ...]
    choices_truncated: bool
    choices_source: ChoicesSource
    warnings: tuple[TeamWarning, ...]
    warnings_truncated: bool
    @classmethod
    def from_wire(cls, value: Any) -> Team:
        d = _object(value, "team")
        return cls(_open(TeamTarget, d, "target"), _text(d, "revision", nonempty=True), _flag(d, "layer_present"),
                   _text(d, "template_revision", nonempty=True), _flag(d, "template_pending_restart"),
                   tuple(AvatarView.from_wire(a) for a in _items(d["avatars"], "avatars", 128)),
                   _flag(d, "avatars_truncated"),
                   tuple(RoleView.from_wire(r) for r in _items(d["roles"], "roles", 64)), _flag(d, "roles_truncated"),
                   tuple(ModelChoice.from_wire(c) for c in _items(d["choices"], "choices", 2000)),
                   _flag(d, "choices_truncated"), _open(ChoicesSource, d, "choices_source"),
                   tuple(TeamWarning.from_wire(w) for w in _items(d["warnings"], "warnings", 100)),
                   _flag(d, "warnings_truncated"))

@dataclass(frozen=True)
class TeamWriteResult:
    """`applies` says when the write takes effect; `propagation.room` what happened to a live room."""
    team: Team
    applies: Applies
    propagation: Propagation
    @classmethod
    def from_wire(cls, value: Any) -> TeamWriteResult:
        d = _object(value, "result")
        return cls(Team.from_wire(d["team"]), _open(Applies, d, "applies"), Propagation.from_wire(d["propagation"]))

@dataclass(frozen=True)
class AvatarRemoveResult(TeamWriteResult):
    """`reverted_to` is the layer whose entry now applies (`base`, `template`), or `None` when the avatar is gone."""
    reverted_to: Layer | None
    @classmethod
    def from_wire(cls, value: Any) -> AvatarRemoveResult:
        d = _object(value, "result")
        r = TeamWriteResult.from_wire(d)
        return cls(r.team, r.applies, r.propagation, _optional_open(Layer, d, "reverted_to"))

# Events

@dataclass(frozen=True)
class RoomLineEvent:
    """A history line. `recovered` marks one fetched with `history` after a gap or an overflow."""
    seq: int
    epoch: str
    line: RoomLine
    recovered: bool = False

@dataclass(frozen=True)
class RoomDeltaEvent:
    """Streaming text of a lane. Never history: the complete text follows as a line."""
    seq: int
    epoch: str
    author: str
    lane: LaneName
    kind: DeltaKind
    text: str

@dataclass(frozen=True)
class RoomStatusEvent:
    seq: int
    epoch: str
    status: RoomStatus

@dataclass(frozen=True)
class RoomNoticeEvent:
    """`stalled`: no status for 10 seconds, so the view is stale; the binding stays open."""
    seq: int
    epoch: str
    code: NoticeCode
    text: str

@dataclass(frozen=True)
class RoomClosedEvent:
    """The binding is closed. On `taken_over` another client holds the room; the SDK never re-attaches by itself."""
    seq: int
    epoch: str
    session_id: str
    reason: CloseReason

@dataclass(frozen=True)
class RoomGapEvent:
    """The adapter dropped queued events. `Room.events()` has already recovered the lines from `history`."""
    seq: int
    epoch: str
    after_line_seq: int
    dropped_lines: int
    dropped_deltas: int

RoomEvent = Union[RoomLineEvent, RoomDeltaEvent, RoomStatusEvent, RoomNoticeEvent, RoomClosedEvent, RoomGapEvent]


def decode_room_event(message: dict[str, Any]) -> RoomEvent | None:
    """Decode a `room.*` event frame; `None` for a room event name this library does not know."""
    name, seq, epoch = message["event"], message["seq"], _text(message, "epoch", nonempty=True)
    if name == "room.line":
        return RoomLineEvent(seq, epoch, RoomLine.from_wire(message["line"]))
    if name == "room.delta":
        return RoomDeltaEvent(seq, epoch, _text(message, "author", nonempty=True), _open(LaneName, message, "lane"),
                              _open(DeltaKind, message, "kind"), _text(message, "text"))
    if name == "room.status":
        return RoomStatusEvent(seq, epoch, RoomStatus.from_wire(message["status"]))
    if name == "room.notice":
        return RoomNoticeEvent(seq, epoch, _open(NoticeCode, message, "code"), _text(message, "text"))
    if name == "room.closed":
        return RoomClosedEvent(seq, epoch, _text(message, "session_id", nonempty=True),
                               _open(CloseReason, message, "reason"))
    if name == "room.gap":
        return RoomGapEvent(seq, epoch, _count(message, "after_line_seq"), _count(message, "dropped_lines"),
                            _count(message, "dropped_deltas"))
    return None
