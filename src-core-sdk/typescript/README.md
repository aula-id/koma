# JavaScript / TypeScript SDK (Node.js 22+)

Install/configure an SDK-capable Koma binary separately: [shared guide](../README.md).
One ESM package provides JavaScript and TypeScript declarations. No runtime dependencies, native addons, installation hooks or engine downloads. The npm name is not a claim of registry publication.

Consumers install a supplied tarball:

```sh
npm install /path/to/koma-sdk-0.1.0.tgz
koma sdk --help
node examples/bot.mjs /absolute/workspace
```

```js
import { Client } from '@koma/sdk';
const client = new Client({ executable: '/absolute/path/to/koma' });
try {
  console.log(await client.probe());
  const session = await client.create({ cwd: '/absolute/workspace' });
  console.log(session.sessionId); // save for explicit resume
  const result = await session.submit({ text: 'Explain this repository' });
  if (result.disposition === 'started') {
    console.log(await session.waitTurn(result.turn_id, { timeoutMs: 120_000 }));
    console.log((await session.getState()).messages);
  }
} finally { await client.close(); }
```

`client.configure({providers, models})` fills an empty Koma before `create`. `session.configure(...)` swaps providers or roles while the session is running; the next `submit` uses them. A new provider needs `api_key`. Omitting it on a later call keeps the stored key. The reply contains names and uuids, never the key.

`executable` defaults to `koma` on PATH. `create` defaults to `{plan:'forbid',approval:'reject'}`; reject denies approval-requiring gates, not every tool, and is not a sandbox.
`resume({session_id})` preserves policy/cwd. Explicit policy changes honor daemon idle guards. External tool approvals require `external_approvals:true`; normally supply policy `{plan:'forbid',approval:'default'}` too.
Get `pending_approval.call_id` from `getState()` and call `approve(callId, true/false)`; plan/mission gates are visible but unsupported.

`submit({text,attachments})`, `enqueue(...)`, `interrupt(turnId)` and `interruptAndSubmit(turnId,input)` preserve typed daemon dispositions.
Enqueue admits to a five-entry FIFO/coalescing queue; it creates no reserved future turn ID. Replacement retains `interrupted_turn_id` on partial failure.
Attachments are `{kind:'image_file',path:'/absolute/chart.png'}` or `{kind:'pasted_text',text:'context',name:'label'}`. Paths are on the daemon host, not uploads. See the shared guide for limits.

`for await (const event of session.events())` allows one consumer. Handle full `state` replacements, `output_append` suffixes and informational `turn` observations; revisions/UTF-8 offsets prevent duplicate appends. Offset/revision mismatch causes state readback. A sequence gap triggers state readback; nonmonotonic sequences close the adapter. Event overflow clears buffered events, pauses buffering, and raises `SdkError` with code `event_overflow` through event consumption. Pending requests, polling waits, and control methods remain usable, including polling-only `submit` / `waitTurn` calls on long streams.

After catching `event_overflow`, start a new `session.events()` iterator. It enables buffering, reads authoritative state, and yields a state event with `resynced: true` before later output. Queued observations superseded by that state are ignored. Overflow during recovery raises `event_overflow` again. Polling waits do not consume an active iterator's events. Malformed frames and transport failures still close the connection. Events can precede method responses.
Bounded buffers: 256 events / 64 MiB, 64 pending requests, 64 MiB outbound buffering, 64 KiB stderr tail, 40 MiB frames. Keep consuming events during long-running work.

Requests accept `{signal,timeoutMs}`. Cancelling a local await does not interrupt a turn. A dispatched mutation with a lost reply throws `DeliveryUnknownError` containing available `requestId`, `sessionId`, `turnId`; no automatic retries. Resume/read state before deciding what to do.
`waitTurn` reports final observation, not goal success; a newer hidden turn raises `TurnReplacedError`, timeout raises an observation/timeout error. `finished` only means execution ended normally.
`detach`, `close` and stopping event iteration never stop/delete the daemon session. Use explicit targeted interruption when intended.

## Discovery and gating

`probe()` returns hello with `capabilities` (always the ten v1 methods) plus `methods`, `features`, `events` and `limits`. An older adapter sends none of the four; the client then fills in `[]`, `[]`, `[]` and `{}`. `requiredCapabilities` still accepts only the v1 names.

Gate new calls with `supports(method)` on a `Host` or `Session`. It reads `methods`, or `capabilities` when `methods` is absent. A call the adapter does not implement fails locally with `unsupported_method` (`not_sent`) and is never written.

```ts
const host = client.host();
if (await host.supports('config.get')) console.log(await host.getConfig());
```

## Host: calls without a session

`client.host()` returns a handle for calls that need no session. It starts an unbound adapter on first use. Because that adapter holds no state, the handle replaces it once `HOST_RECYCLE_AFTER` (60,000) requests have been sent, before the adapter's per-process limit (`limits.request_ids`, 65,536), and after the adapter failed. In-flight requests finish on the old adapter. A failed call is never retried. Close the handle with `host.close()`; `client.close()` closes it too.

```ts
const host = client.host();
try {
  const view = await host.getConfig();                    // ConfigView, view "global"
  const preview = await host.deleteProvider({ provider_uuid: 'prov-1', if_revision: view.revision, dry_run: true });
  console.log(preview.removed.models_removed);            // nothing written
  const done = await host.deleteProvider({ provider_uuid: 'prov-1', if_revision: view.revision });
  console.log(done.config.revision, done.propagation);
  await host.deleteModel({ model_uuid: 'model-1' });
  await host.unassignRole({ role: 'planner' });           // global scope; changed:false when nobody held it
} finally { await host.close(); }
```

`host` has `configure`, `getConfig`, `deleteProvider`, `deleteModel` and `unassignRole`. Params and results use the wire names from the contract (`provider_uuid`, `if_revision`, `dry_run`, ...). The view never contains a key or token: providers report `has_key` and `key_state`, OAuth connections `has_token`, and endpoints come back with userinfo, query and fragment removed (`endpoint_redacted:true`). Do not feed a redacted endpoint back into `configure`. See `examples/config.ts`.

## Session settings and session_info

A session handle has the same config calls (in a session `getConfig()` defaults to the session view; deletes go through the session's daemon) plus the calls that need a running session. `unassignRole` is global scope unless `scope` is given, on a session as on `host`; `clearMainPin()` drops only the session's own Main pin:

```ts
const session = await client.create({ cwd: '/absolute/workspace' });
console.log(session.sessionInfo);                          // name, effort, routes, limits, daemon, observed_at_ms
const effort = await session.getEffort({ wait_ms: 3000 }); // state "ready" | "loading" | "unsupported"
if (effort.options.includes('high')) await session.setEffort({ effort: 'high', if_main_model: effort.main_model_uuid });
await session.setLimits({ max_output_tokens: 4096 });      // 0 means auto
console.log(await session.getLimits(), (await session.info()).limits);
await session.clearMainPin();                              // clears this session's Main pin; effort resets
console.log(session.requestsRemaining);
```

`session_info` travels beside `state` in `create`, `resume` and `state` results and in session config results. It changes without a state revision and without an event. The session keeps the one with the highest `observed_at_ms`, whatever the state revision. Call `info()` (or `getState()`) for fresh values. `sessionInfo` is `null` with an older adapter.

A session's adapter accepts a fixed number of requests (`requestsRemaining` counts down). Before it reaches zero, close the client and `resume` the session from a new one; the next request past the limit ends the adapter.

## Errors from newer adapters

With feature `errors.reason`, `SdkError` may carry `reason` (branch on it, never on the message), `binding` and `allowed` (on a call made in the wrong binding), `written` (`true` when an `unknown` outcome committed bytes) and `detail`. A mutation whose outcome is unknown throws `DeliveryUnknownError`, as before; config deletes, `unassignRole`, `clearMainPin`, `setEffort` and `setLimits` count as mutations, a delete with `dry_run:true` does not.

```ts
try { await host.deleteProvider({ provider_uuid, if_revision }); }
catch (e) {
  if (e instanceof SdkError && e.reason === 'stale') { /* read again, then decide */ }
  else if (e instanceof DeliveryUnknownError && e.written) { /* committed but unverified: read back */ }
  else throw e;
}
```

Enum-like strings added after v1 (`reason`, `binding`, `key_state`, `origin`, `propagation` values, effort `state`, ...) are open: the client passes unknown values through unchanged, typed as `string`, and callers treat an unknown value as "other". The v1 enums (message `role`, `gate.kind`, `turn.state`, `disposition`, policy) and the five model roles stay closed and still fail validation on an unknown value.

## Maintainers

```sh
npm ci
npm test
npm pack --pack-destination /tmp
# Install that tarball in a clean project, then test the public import.
```

`npm run build` emits ESM JS and `.d.ts` into `dist`. Build before packing; intentionally no install/prepare hooks. Tests use the shared fake adapter, with `PYTHON` override if needed. Real examples intentionally submit provider-backed inference. Windows/Linux runtime verification is separate from source portability.
