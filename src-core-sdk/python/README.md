# Python SDK (Python 3.11+)

Install and configure an SDK-capable Koma binary separately; see [installation and safety](../README.md).
These are local packages, not a claim of PyPI publication. No runtime dependencies or binary downloader.

```sh
python -m pip install /path/to/koma/src-core-sdk/python
koma sdk --help
python /path/to/koma/src-core-sdk/python/examples/bot.py /absolute/workspace
```

```python
import asyncio
from koma_sdk import Client, Disposition, DeliveryUnknown

async def main():
    async with Client(executable="/absolute/path/to/koma") as client:
        print(await client.probe())
        async with await client.create("/absolute/workspace") as session:
            print(session.session_id)  # save for explicit resume
            result = await session.submit("Explain this repository")
            if result.disposition is Disposition.STARTED:
                turn = await session.wait_turn(result.turn_id, timeout=120)
                print(turn.state, (await session.get_state()).messages)

asyncio.run(main())
```

`configure(providers=[...], models={...})` on `Client` fills an empty Koma before `create`. The same call on `Session` swaps providers or roles while the session is running; the next `submit` uses them. A new provider needs `api_key`. Omitting it on a later call keeps the stored key. The reply contains names and uuids, never the key.

`create` defaults to `Policy(plan=FORBID, approval=REJECT)`. Reject denies gates requiring approval, not every tool, and is not a sandbox.
`resume(id)` preserves cwd/policy; an explicit `policy=` requests an idle-guarded change.
External approvals require `external_approvals=True` and normally `Policy(approval=ApprovalPolicy.DEFAULT)`.
Use `state.pending_approval.call_id` with `session.approve(call_id, True/False)`; plan/mission gates remain unsupported and visible in `state.gate`.

`enqueue(text)` reports admission only (up to five queued inputs, FIFO with existing coalescing); it reserves no new turn.
`interrupt(turn_id)` and `interrupt_and_submit(turn_id, text)` are targeted. Replacement results preserve `interrupted_turn_id` even on partial failure.
Attachments are ordered `ImageFile('/absolute/chart.png')` or `PastedText('context', 'label')` objects passed as `attachments=[...]`.
Paths refer to the daemon host. See the shared guide for byte/count limits.

`async for event in session.events()` has one consumer. Handle `StateEvent` replacements, `OutputAppendEvent` proven UTF-8 suffixes, and `TurnEvent` observations.
Revisions and offsets are connection-local; full state replaces prior state. The iterator reconciles mismatches by state readback. Events can precede method results.
Buffers are bounded (256 events / 64 MiB, 64 pending requests, 64 KiB stderr tail, 40 MiB serialized frames). Overflow clears buffered events and pauses event buffering. Event consumption raises `EventOverflow`; pending requests, polling waits, and control methods remain usable. Polling-only `submit` / `wait_turn` usage is supported for long streams.
After catching `EventOverflow`, start a new `session.events()` iterator. It enables buffering, reads authoritative state, and yields `StateEvent(resynced=True)` before later output. Queued observations superseded by that state are ignored. Overflow during recovery raises `EventOverflow` again. Polling waits do not consume an active iterator's events. Malformed frames and transport failures still close the connection.

Mutation timeouts, cancellation after writing, malformed replies and disconnects raise `DeliveryUnknown`, containing available request/session/turn IDs. Never retry automatically: resume/read state first. Cancellation does not interrupt work.
`wait_turn` returns the observed final record, not proof of task success; hidden/newer turns or timeout raise `ObservationUnavailable`. `finished` only means normal execution ended.
Context exit, `close`, `detach` and EOF only close the adapter connection; they do not stop the daemon or running turn. Explicitly interrupt if that is intended.

## Discovery

`Hello` (from `probe()`, `session.hello` or `host.hello`) carries `methods`, `features`, `events` and `limits`. They are empty when the adapter is older than them; such an adapter implements exactly `capabilities`, the ten original methods. `hello.supports(name)` checks `methods`, falling back to `capabilities`. Every call is gated this way: a method the adapter does not list raises `IncompatibleAdapter` before anything is sent.

## Config

`client.host()` returns a `Host` for calls that need no session. It runs its own unbound adapter, started on the first call. An adapter accepts a fixed number of requests (`hello.limits["request_ids"]`, 65,536); `Host` replaces its adapter after 60,000 requests (`recycle_after=`), and replaces a failed adapter on the next call. The call that failed is never retried.

```python
from koma_sdk import Client, RemoteError

async with Client("/absolute/path/to/koma") as client:
    async with client.host() as host:
        view = await host.config_get()          # ConfigView; never contains a key or token
        for p in view.providers:
            print(p.provider_uuid, p.name, p.endpoint, p.key_state)
        print(view.route("main").effective_model, view.main_fallback)

        plan = await host.config_providers_delete(uuid, if_revision=view.revision, dry_run=True)
        print(plan.removed.models_removed, plan.removed.main_reset)   # nothing written
        try:
            done = await host.config_providers_delete(uuid, if_revision=plan.config.revision)
        except RemoteError as exc:
            if exc.reason == "stale":            # config.json changed since the dry run
                ...
```

`Host` also has `configure(providers=, models=)`, `config_models_delete(model_uuid, if_revision=, dry_run=)` and `config_roles_unassign(role, if_revision=)` (global scope; an unassigned `compactor`, `awareness` or `planner` then inherits Main, while `safeguard` never does). Mutations return the fresh `config` plus `propagation`, which reports what happened after the commit (`bound`, `broadcast`, `rebind`, `rooms`); a `failed` value there does not mean the write failed. See [`examples/config.py`](examples/config.py).

`endpoint` values have URL credentials, query and fragment removed (`endpoint_redacted`); never pass one back to `configure`. A list with `*_truncated` set is incomplete.

## Session settings

A `Session` adds the calls that act on its own coding session:

```python
info = await session.info()                 # SessionInfo: name, effort, routes, limits, daemon
opts = await session.effort_get()           # EffortOptions(state=READY|LOADING|UNSUPPORTED, options, selected)
await session.effort_set("high", if_main_model=opts.main_model_uuid)
await session.limits_set(max_output_tokens=4096)   # 0 = auto; context_model_alias="" clears
limits = await session.limits_get()
view = await session.config_get()           # view="session" (with its Main pin) or view="global"
await session.clear_main_pin()              # clears this session's Main pin; effort resets
```

A `Session` also has the global config writes `config_providers_delete`, `config_models_delete` and `config_roles_unassign`, with the same parameters as on `Host`. On a session they go through its daemon, and the result carries `session_info`. `config_roles_unassign` is always global scope; only `clear_main_pin` touches the session's own Main.

`session.session_info` holds the newest `SessionInfo` seen. It arrives beside `state` in `create`, `resume` and `get_state()` results, and in the results above. It is not ordered by the state revision: a newer one replaces the held one only when its `observed_at_ms` is higher. A change to it emits no event and does not bump the revision; call `info()` or `get_state()` to refresh. It is `None` with an older adapter.

`session.requests_remaining` counts the requests this adapter still accepts. When it reaches 0 calls raise `TransportError` before sending; detach and `resume` the session from a fresh adapter well before that.

## Sessions

`Host` lists and manages sessions of every kind without binding one. Each call is gated on the adapter's `methods`. The session, usage, skill and room-listing calls below, and the `room_team_get`/`room_*` template calls, work in every binding: a `Session` or `Room` has them too and sends them on its own adapter.

```python
page = await host.sessions_list(kind="all", live=None, limit=100)     # SessionPage, newest first
while page.next_cursor:                                               # cursors are opaque; pass them back
    page = await host.sessions_list(limit=100, cursor=page.next_cursor)
await host.sessions_rename(session_id, "nightly build")               # a room, or a session that is not live
await host.sessions_stop(session_id)                                  # disconnects every client of that daemon
await host.sessions_delete(session_id)                                # refused while live, locked or bound
page = await host.sessions_transcript(session_id, since=0, limit=100) # SessionTranscript: stored messages, no daemon started
more = await host.sessions_transcript(session_id, since=page.messages[-1].id)  # read on from the last id you saw
```

A `SessionRow` has `kind` (`coding` or `room`), `live`, `working` (`None` for rooms), `locked`, `bound`, and for linked sessions `parent_session_id` / `room_session_id` / `room_live`. Renaming another *live* coding session is refused with `reason == "live"`: rename it from the adapter bound to it, or stop it first. A delete or stop whose outcome is unknown raises `DeliveryUnknown`; list again before retrying. A delete keeps usage rows and a child room (`orphaned_room_session_id`).

`sessions_transcript` reads the stored messages of a coding session (`id`, `role`, `text`, `created_at`, and `truncated` when `text` was cut) from disk, oldest first, with secrets scrubbed. It starts no daemon, so a session that is not live reads the same as a running one (`live` says which). `since` is the last message `id` you saw; `next_since` is set when more follow. A room is refused (`out_of_range`): use `Room.history`.

`client.create(cwd, name="nightly build")` names the session. It needs the `create.name` feature (otherwise `IncompatibleAdapter`, before anything starts). If naming fails the session still exists: check `session.warnings` for `rename_failed`. See [`examples/sessions.py`](examples/sessions.py).

## Usage

```python
report = await host.usage_query(from_at=start, to_at=end, group_by="model", bucket="day", tz_offset_minutes=0)
report.totals.cost, report.ledger          # LedgerState.MISSING: nothing was ever recorded; not an error
for g in report.groups: ...                # sorted by cost; `groups_truncated` if `limit` cut them
for b in report.buckets: ...               # ascending, empty buckets included
```

`group_by` is `none`, `model`, `role` or `session`; `bucket` is `none`, `hour`, `day` or `week`. Times are unix seconds; the default window is the last 30 days. More than 400 days or 1000 buckets is `reason == "range_too_large"`; a query that takes over 2 s is `query_timeout`. Only the roles `main` and `sub:<agent>` are recorded; a room's lanes appear under the room's session id (`chief` is its Main). A query holds a read lock on the ledger while it runs, so poll no more than every 30 seconds. See [`examples/usage.py`](examples/usage.py).

## Skills

```python
page = await host.skills_list(workdir="/abs/project", include_diagnostics=True)   # every tier; `effective` marks the winner
detail = await host.skills_get("code-review", tier="project-agents", workdir="/abs/project", include_raw=True)
made = await host.skills_create(scope="project", workdir="/abs/project", name="triage",
                                description="Triage issues", instruction="Label them.")
await host.skills_update(tier="project-agents", workdir="/abs/project", name="triage",
                         generation=made.skill.generation, patch={"instruction": "Label and assign."})
await host.skills_install(scope="computer", data=open("pack.zip", "rb").read())   # or path="/abs/pack.zip"
await host.skills_remove(tier="project-agents", workdir="/abs/project", name="triage")
```

A skill is addressed by `(tier, name)`; the tiers are `global`, `claude`, `project-agent`, `project-agents` and `extra-root:<n>`. New skills go to scope `computer` (`global`) or `project` (`project-agents`). The `claude` and `extra-root` tiers are read only (`reason == "read_only"`). Project tiers need `workdir` on a `Host` (`workdir_required`). Pass the `generation` you read to `skills_update` and `skills_remove`; a changed skill is `reason == "stale"`. New skill names are lowercase letters, digits and single hyphens, up to 64 bytes, and not `skill` or `readme`; the client checks this before sending.

`skills_install(data=...)` sends the zip inline, base64 encoded: at most 28 MiB (`hello.limits["skills"]["inline_zip_bytes"]`), checked before anything is encoded or sent. `inline_zip(data)` is the same helper if you want the encoded text. A larger zip (up to 64 MiB) goes by `path=`, which must be absolute and readable by the adapter. Results carry `propagation.skills`: `next_rebuild` (a running coding session sees the change at its next system-prompt rebuild) or `room_restart` (a room reads its skills when it starts). See [`examples/skills.py`](examples/skills.py).

## Room

A room is the multi-agent team of a coding session. `client.room_open(parent_session_id)` starts it if needed and attaches; `client.room_attach(room_session_id)` attaches a running one (`reason == "not_running"` otherwise). Each `Room` owns one adapter. **A room has one live viewer**: opening or attaching takes the view from any other client, the terminal included.

```python
async with await client.room_open(parent_id, tail=20) as room:
    room.attached.tail, room.attached.roster, room.attached.status      # render these first
    await room.send("scan /login", to="scout")                          # shell=True runs a shell line; attachments= for images/text
    async for event in room.events():                                   # typed, current epoch only
        match event:
            case RoomLineEvent(line=line): print(line.author, line.text)
            case RoomStatusEvent(status=status): ...
            case RoomClosedEvent(reason=reason): print(reason); break   # the iterator ends here
```

Calls: `send`, `interrupt`, `control(action, avatar, note=)` (`pause`, `resume`, `stop`, `redirect`), `status`, `history`, `shelf_list`, `item`, `trajectory`, `reports`, `spend`, `team_get`, `models_set`, `avatars_upsert`, `avatars_remove`, and `close(stop=False)` (releases the room and the adapter; `aclose()` only drops the connection). `room.requests_remaining` counts the requests the adapter still accepts; attach again from a fresh `Room` before it reaches 0. A room write whose outcome is unknown raises `DeliveryUnknown` and is never retried: a failed send moves the binding to closed, so read `history()` before sending again.

**Events.** `RoomLineEvent` (history lines, in `line_seq` order, each once), `RoomDeltaEvent` (streaming text, never history), `RoomStatusEvent`, `RoomNoticeEvent` (`stalled` means the view is stale; the binding stays open), `RoomGapEvent` and `RoomClosedEvent`. A line is identified by `(epoch, line_seq)`; `line_seq` restarts at 1 in each epoch, and the envelope `seq` is a different counter. `events()` ignores events of an earlier epoch. After a `RoomGapEvent`, or when the client-side queue overflowed (256 events), it fetches the missed lines with `history()` and yields them as `RoomLineEvent(recovered=True)`, then a fresh `RoomStatusEvent`. It never reads coding-session `state`. `history()` holds the room's replay (at most 2000 lines) plus live lines, nothing older.

**Closing.** `RoomClosedEvent.reason` is `taken_over` (another client attached), `daemon_exit` or `stream_error`. The handle reports it (`room.closed_reason`, `room.live`) and does not re-attach by itself, or two clients would take the view from each other forever. Reads (`history`, `shelf_list`, `item`, ...) keep working on a closed room; writes to the room are `reason == "room_closed"`. `await room.reattach()` takes the room back (`room.attach`), `await room.reopen()` starts it again (`room.open` on its parent); each starts a new epoch with a fresh line buffer.

**Team.** `team_get()` returns the effective team for a layer (`room` by default, or `target="template"` for `~/.koma/room/`). `models_set`, `avatars_upsert` and `avatars_remove` take `target="room"` or `"template"` and `if_revision=team.revision`. An omitted `model`, `effort`, `display` or `alias` keeps the layer's value; `None` clears it so the layer below applies. Roles are read only. The result's `applies` says when it takes effect: `live` (the next lane turn), `next_reload` or `next_start`; the template is read once when a room starts. `Host.room_team_get`, `room_models_set`, `room_avatars_upsert` and `room_avatars_remove` edit the template without a room, and `Host.room_list()` lists rooms. Writes with `target="template"` need the `room.template` feature. See [`examples/room.py`](examples/room.py).

## Errors and new values

`RemoteError` exposes `code`, `message`, and from newer adapters `reason`, `binding`, `allowed`, `written` and `detail`. Branch on `reason`, never on `message`; handle an unknown `reason` by its `code`. A call made in the wrong binding has `reason="wrong_binding"` with the adapter's `binding` and the `allowed` ones.

A mutation answered with `delivery:"unknown"` raises `DeliveryUnknown`; its `cause` is the `RemoteError`, whose `written` is `True` when bytes were committed (`reason="unverified"`). Read the config again before doing anything else. `dry_run=True` deletes and all reads are not mutations.

Enum-like strings added after v1 are open: `ConfigFile`, `ConfigViewKind`, `MainFallback`, `KeyState`, `ProviderKind`, `RouteOrigin`, `RouteReason`, `EffortState`, `PropagationStatus`, and the session, usage, skill and room enums (`SessionKind`, `LedgerState`, `SkillScope`, `LineKind`, `LanePhase`, `CloseReason`, `Applies`, ...) map a value this library does not know to their `OTHER` member instead of failing. Treat `OTHER` as "unknown, show it neutrally". The original enums (`Role`, `GateKind`, `TurnStatus`, `Disposition`, policy) and `PublicRole` are closed; an unknown value there is a protocol error.

## Maintainers

```sh
PYTHONPATH=src python -m unittest discover -s tests -v
python -m pip wheel --no-deps --wheel-dir /tmp/koma-wheels .
# Install the resulting wheel in a clean venv and import koma_sdk.
```

Build tooling is setuptools; wheel metadata includes `py.typed`. Tests use the shared fake adapter without providers. Provider-backed examples intentionally submit real inference. Windows/Linux source portability is not runtime verification.
