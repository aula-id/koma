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

`host` has `configure`, `getConfig`, `deleteProvider`, `deleteModel` and `unassignRole`, and the calls of the sections below. Params and results use the wire names from the contract (`provider_uuid`, `if_revision`, `dry_run`, ...). The view never contains a key or token: providers report `has_key` and `key_state`, OAuth connections `has_token`, and endpoints come back with userinfo, query and fragment removed (`endpoint_redacted:true`). Do not feed a redacted endpoint back into `configure`. See `examples/config.ts`.

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

## Sessions

The calls below work on a `Host`, and on a `Session` or `Room` handle too (they are available in every binding).

```ts
const host = client.host();
const page = await host.listSessions({ kind: 'all', limit: 100 });   // newest first; pass next_cursor back as cursor
await host.renameSession({ session_id, name: 'Nightly' });
await host.stopSession({ session_id });                              // disconnects every client of that daemon
await host.deleteSession({ session_id });
```

Listings never write lock files. `renameSession` of another live coding session fails with `reason: 'live'`; rename it from an adapter bound to it, or stop it first. `deleteSession` refuses the session this adapter holds (`bound_self`), a live one (`live`) and a locked one (`session_locked`). A failure after the removal started is a `DeliveryUnknownError` with `reason: 'partial_delete'` and `written: true`; repeating the call is valid. `stopSession` of an id that is not running answers `was_live: false`.

`readTranscript({ session_id, since?, limit? })` returns the stored messages of a coding session (`id`, `role`, `text`, `created_at`, and `truncated` when `text` was cut), oldest first, with secrets scrubbed. It reads from disk and starts no daemon, so a session that is not live reads the same as a running one (`live` says which). Pass the last `id` you saw as `since`; `next_since` is set when more follow. A room is refused with `out_of_range`: use `history`.

`create({ cwd, name })` names the session. It needs the adapter feature `create.name`; against an adapter without it the call fails locally with `unsupported_feature` before anything is sent. The name is trimmed and must be 1 to 200 characters without control characters. If the rename fails the session still exists, `session.warnings` holds `rename_failed`, and `session.sessionInfo.name` is the session id. See `examples/sessions.ts`.

## Usage

```ts
const report = await host.queryUsage({ from_at, to_at, group_by: 'model', bucket: 'day' });
console.log(report.ledger, report.totals.cost, report.groups, report.buckets);
```

Defaults are the last 30 days, no groups and no buckets. `ledger` is `'missing'` when nothing was recorded. A window over 400 days or more than 1000 buckets fails with `reason: 'range_too_large'`; a query that runs past its 2 s deadline with `reason: 'query_timeout'`. Only the Main model and sub-agents are recorded; room lanes are recorded under the room's session id. Poll no more often than every 30 s: a reader can make a concurrent insert of the daemon's ledger fail. See `examples/usage.ts`.

## Skills

```ts
const list = await host.listSkills({ scope: 'all', workdir: '/absolute/project', include_diagnostics: true });
const detail = await host.getSkill({ tier: 'global', name: 'code-review', include_raw: true });
const file = await host.readSkillFile({ tier: 'global', name: 'code-review', path: 'notes/checklist.md' });
const made = await host.createSkill({ scope: 'computer', name: 'my-skill', description: '...', instruction: '...' });
await host.updateSkill({ tier: 'global', name: 'my-skill', generation: made.skill.generation, patch: { description: 'new' } });
await host.installSkillZip(zipBytes, { scope: 'project', workdir: '/absolute/project' });
await host.removeSkill({ tier: 'global', name: 'my-skill' });
```

An existing skill is addressed by `(tier, name)`: `global`, `claude`, `project-agent`, `project-agents` or `extra-root:<n>`. New skills take `scope` `'computer'` or `'project'`. `claude` and extra-root tiers are read only (`reason: 'read_only'`). Pass the `generation` you read to `updateSkill` and `removeSkill`; a change in between fails with `reason: 'stale'`. Project tiers need a `workdir` on an unbound adapter (`workdir_required`); a session or room uses its own.

`installSkillZip(bytes, params)` checks the size against `limits.skills.inline_zip_bytes` (28 MiB) before it encodes, and throws `invalid_input` with `reason: 'too_large'` without sending anything. For a larger zip put the file on the adapter's host and call `installSkill({ scope, path: '/absolute/skill.zip' })`; `installSkill` also takes `data_b64`, exactly one of the two. Results carry `propagation.skills`: `next_rebuild` (running coding sessions pick the change up at their next system-prompt rebuild) or `room_restart` (a room's catalogue is frozen at start). See `examples/skills.ts`.

## Rooms

```ts
const rooms = await host.listRooms({ live: true });
const room = await client.roomOpen({ parent_session_id });   // starts the room when needed
// or: await client.roomAttach({ session_id: roomSessionId }); // a room that is already running
console.log(room.epoch, room.attachResult.tail, room.attachResult.roster);

await room.send({ text: 'check the login page', to: 'scout' });  // shell: true sends "!" + text
for await (const event of room.events()) {
  if (event.event === 'room.line') console.log(event.line.author, event.line.text);
  else if (event.event === 'room.closed') console.log(event.reason);   // taken_over | daemon_exit | stream_error
}
await room.close();   // sends room.close (stop: true also stops the room's daemon) and releases the adapter
```

A `Room` owns one adapter, like a `Session`. Methods: `send`, `interrupt`, `control({ action, avatar, note? })`, `status`, `history`, `shelf`, `item`, `trajectory`, `reports`, `spend`, `team`, `setModel`, `upsertAvatar`, `removeAvatar`, `reattach`, `reopen` (start the room again through its parent after its daemon exited), `events`, `close`, plus `epoch`, `sessionId`, `attachResult` and `requestsRemaining`. Reads come from disk and keep working after the live view ended. `send` is never retried: a lost reply throws `DeliveryUnknownError`. A message with attachments needs a room that supports it (`peer_unsupported` otherwise).

**Events.** `room.events()` yields typed room events (`room.line`, `room.delta`, `room.status`, `room.notice`, `room.gap`, `room.closed`) of the current epoch only. Line numbers (`line_seq`) belong to an epoch and are never compared with the envelope `seq`. Lines already seen are not repeated. After `room.gap`, a hole in the event sequence, or a client-side `event_overflow`, the iterator reads the missed lines back with `room.history` and yields them as `room.line` events with `recovered: true`, then the current status as `room.status` with `recovered: true`. It never calls `state`. Page older lines yourself with `room.history({ before_line_seq })`; the ring holds what the daemon replayed (at most 2000 lines) plus later lines.

**One live view.** A room has exactly one live subscriber. `roomOpen`, `roomAttach` and `room.reattach()` take it over, and whoever had it loses its stream without a frame (the terminal shows that the connection closed). The adapter then reports `room.closed` with `reason: 'taken_over'`, the iterator ends, and nothing re-attaches by itself, so two clients never take the room from each other in a loop. Call `reattach()` to take the view back; it starts a new epoch and replaces the line buffer. After `daemon_exit`, start the room again with `roomOpen`. Socket writes (`send`, `interrupt`, `control`) on a closed view fail with `reason: 'room_closed'`.

**Team.** `room.team()` shows the avatars and roles of the room (roles are read only), `room.setModel`, `upsertAvatar` and `removeAvatar` write the room's own layer (`target: 'room'`, the default on a `Room`); pass `target: 'template'` for the template layer. `host.getTeam()`, `host.setTeamModel(...)`, `host.upsertTeamAvatar(...)` and `host.removeTeamAvatar(...)` work on the template (`~/.koma/room`) without a room; a template change applies to rooms started afterwards (`applies: 'next_start'`). A write to a live room applies at the next turn when the room supports reloading (`applies: 'live'`); otherwise the file is written and taken at the next reload (`next_reload`). Pass `if_revision` from `Team.revision` to guard against concurrent edits.

Like a session adapter, a room adapter accepts a fixed number of requests; watch `room.requestsRemaining` and, before it reaches zero, `roomAttach` from a new client (the old one then sees `taken_over` and is closed by `room.close()`). See `examples/room.ts`.

## Errors from newer adapters

With feature `errors.reason`, `SdkError` may carry `reason` (branch on it, never on the message), `binding` and `allowed` (on a call made in the wrong binding), `written` (`true` when an `unknown` outcome committed bytes) and `detail`. A mutation whose outcome is unknown throws `DeliveryUnknownError`, as before; config deletes, `unassignRole`, `clearMainPin`, `setEffort`, `setLimits`, session rename/delete/stop, skill writes, `room.open`, `room.attach` (and `reattach`, `reopen`), `room.close`, `room.send`, `room.interrupt`, `room.control` and team writes count as mutations, a delete with `dry_run:true` does not.

```ts
try { await host.deleteProvider({ provider_uuid, if_revision }); }
catch (e) {
  if (e instanceof SdkError && e.reason === 'stale') { /* read again, then decide */ }
  else if (e instanceof DeliveryUnknownError && e.written) { /* committed but unverified: read back */ }
  else throw e;
}
```

Enum-like strings added after v1 (`reason`, `binding`, `key_state`, `origin`, `propagation` values, effort `state`, session, skill and room `kind`/`tier`/`scope`, room line `kind`, lane `phase`, notice `code`, close `reason`, ...) are open: the client passes unknown values through unchanged, typed as `string`, and callers treat an unknown value as "other". The v1 enums (message `role`, `gate.kind`, `turn.state`, `disposition`, policy) and the five model roles stay closed and still fail validation on an unknown value.

## Maintainers

```sh
npm ci
npm test
npm pack --pack-destination /tmp
# Install that tarball in a clean project, then test the public import.
```

`npm run build` emits ESM JS and `.d.ts` into `dist`. Build before packing; intentionally no install/prepare hooks. Tests use the shared fake adapter, with `PYTHON` override if needed. Real examples intentionally submit provider-backed inference. Windows/Linux runtime verification is separate from source portability.
