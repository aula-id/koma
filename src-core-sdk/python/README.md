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

## Errors and new values

`RemoteError` exposes `code`, `message`, and from newer adapters `reason`, `binding`, `allowed`, `written` and `detail`. Branch on `reason`, never on `message`; handle an unknown `reason` by its `code`. A call made in the wrong binding has `reason="wrong_binding"` with the adapter's `binding` and the `allowed` ones.

A mutation answered with `delivery:"unknown"` raises `DeliveryUnknown`; its `cause` is the `RemoteError`, whose `written` is `True` when bytes were committed (`reason="unverified"`). Read the config again before doing anything else. `dry_run=True` deletes and all reads are not mutations.

Enum-like strings added after v1 are open: `ConfigFile`, `ConfigViewKind`, `MainFallback`, `KeyState`, `ProviderKind`, `RouteOrigin`, `RouteReason`, `EffortState` and `PropagationStatus` map a value this library does not know to their `OTHER` member instead of failing. Treat `OTHER` as "unknown, show it neutrally". The original enums (`Role`, `GateKind`, `TurnStatus`, `Disposition`, policy) and `PublicRole` are closed; an unknown value there is a protocol error.

## Maintainers

```sh
PYTHONPATH=src python -m unittest discover -s tests -v
python -m pip wheel --no-deps --wheel-dir /tmp/koma-wheels .
# Install the resulting wheel in a clean venv and import koma_sdk.
```

Build tooling is setuptools; wheel metadata includes `py.typed`. Tests use the shared fake adapter without providers. Provider-backed examples intentionally submit real inference. Windows/Linux source portability is not runtime verification.
