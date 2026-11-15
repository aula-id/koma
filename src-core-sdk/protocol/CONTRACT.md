# Public SDK adapter protocol v1

Normative wire contract for independently implemented clients. `schema.json` is
JSON Schema draft 2020-12. This is a public projection, NOT a daemon IPC tunnel.
The schema validates individual frames; the rules below also constrain sequences.

## Transport and lifecycle

Launch the configured adapter binary with its configured arguments. stdin/stdout
are UTF-8 JSON Lines, one JSON object followed by LF; CRLF is accepted. No BOM,
blank lines, pretty-printing, logs, or private IPC frames on stdout. Diagnostics
belong on stderr (clients must drain it and retain at most a 64 KiB tail).
Every outbound frame has an explicit `type` discriminator: `hello`, `response`,
or `event`. Requests have no type discriminator.
First stdout frame MUST be `type:"hello"`, before reading any request. No client hello.
`protocol` is the integer 1, `binary` identifies the running Koma executable
(path and version), `diagnostics` is an array of non-secret human-readable strings,
and `capabilities` lists supported method names. Unsupported methods fail with
`unsupported_method`; never silently emulate atomic methods. All ten methods
are defined by v1; a particular adapter may advertise a subset. Clients reject
unsupported protocol versions before sending requests. hello may also carry
`methods`, `features`, `events` and `limits` (see Discovery).

Exactly one session is bound per adapter process. `create` or `resume` binds it;
repeating either after binding returns `invalid_state`. All other v1 methods
require a bound session. Methods added after v1 state their own bindings (see
Bindings). `detach` detaches only this client, does NOT stop the session,
returns `{detached:true}`, flushes stdout, then exits 0. stdin EOF also detaches.
Unexpected EOF/process failure is a transport failure, not a successful detach.

## Correlation and delivery

Request: `{id,method,params}`; id is a nonempty opaque string, unique for the
process lifetime. An adapter accepts a bounded number of requests per process
(see Request ids). Response: exactly `{type:"response",id,result}` OR `{type:"response",id,error}`. Results are
method-specific (see schema). Errors are `{code,message,delivery}`, plus the
optional keys listed under Errors, where delivery
is `not_sent` (adapter guarantees no mutation was dispatched) or `unknown`
(dispatch may have occurred). Responses may arrive out of order; events may
interleave anywhere after hello. Events have no request id. Never associate a
reply with the oldest outstanding request. Never automatically retry a mutation
on timeout, EOF, cancellation, malformed response, or `delivery:unknown`.
Client-generated transport errors MUST also expose delivery unknown for requests
whose bytes may have been written; pre-write validation errors are not_sent.
There is NO deduplication, exactly-once delivery, or replay promise in v1.
A late response after local timeout is ignored, not reassigned to a new request.

All events carry `type:"event"` and `seq`. seq starts at 1 and increases by
one per emitted event in this process; it is NOT a daemon revision or persisted
cursor. Three event shapes are defined:

* `state`: `{type,event:"state",seq,state,resynced}`. A complete authoritative
  replacement; `resynced` is required boolean, true on recovery from a private
  sequence gap/shadow reset, false for ordinary observations. A `state` response
  is also a complete replacement, without seq or resynced.
* `output_append`: `{type,event:"output_append",seq,revision,offset,text}`.
  Only changes `state.streaming`, treating null as empty text. offset is the
  **UTF-8 byte length** of the prior streaming value; text is a nonempty proven
  suffix. revision is the resulting state revision, exactly previous revision + 1.
  The adapter may emit this only if all other projected fields are unchanged and
  old streaming is a prefix of new streaming. A rewrite, reset, transcript commit,
  or uncertain prefix MUST produce full state instead. Never append guessed deltas.
* `turn`: `{type,event:"turn",seq,revision,turn}`. Informational notification of
  an observed change of the current/latest turn (including a new ID or terminal
  transition), emitted immediately AFTER its authoritative state event. revision
  equals that state's revision and turn equals its nullable turn. It does not
  mutate state or advance revision; it is not a lifecycle journal.

`state.revision` is a positive safe integer local to this connection, starting at
1 on binding and increasing by one for each changed projected state. Unchanged
explicit state queries may repeat it. State responses and events use the same
revision domain. Full replacements may skip revisions. Clients ignore older or
identical revisions when applying state (but still consume seq); a full state at
the same revision is identical. On append revision/offset mismatch or event seq
gap, do not apply a partial update: obtain `state` and resume from that replacement,
ignoring older in-flight observations. Adapters recover private IPC gaps by full
state with resynced true, never by fabricating missing appends/turn transitions.
An explicit state response can supersede an earlier event still buffered locally.

Arrival order is observation order; snapshots may skip intermediate daemon states.
No private event passthrough. Turn completion is observed in state.turn, NOT the
immediate submit result. On resume the current/latest turn can already be final.

## Methods (all params objects, no positional parameters)

* `create`: `{cwd, model_id?, policy?, external_approvals?}`. cwd is an absolute path on the daemon
  host. Optional model_id selects the main model; absent uses configured default.
  policy defaults to `{plan:"forbid",approval:"reject"}`. Return `{state}` only
  after session binding and requested policy have succeeded.
* `resume`: `{session_id, policy?, external_approvals?}`. Preserve persisted policy if omitted;
  explicit policy is installed before success. Return `{state}`.
  `resume` on a secure-room session id is unsupported and unsafe: room sessions
  are not coding sessions.
* `state`: `{}` -> `{state}`. With feature `state.session_info`, the `create`,
  `resume` and `state` results also carry a sibling `session_info` (see Session
  info); the `state` object itself is unchanged.
* `submit`: `{text,attachments?}`. Start only if idle; busy is rejected, never
  silently queued. Return TurnResult with disposition started or rejected.
* `enqueue`: same input. Only a running waiting turn can accept a follow-up;
  bounded queue of 5. Return queued or rejected. Queued input is a steer in the
  current logical turn, NOT a reserved future turn; turn_id is null. It may be
  consumed before the immediate response is observed. No per-item durable IDs.
* `interrupt`: `{turn_id}`. Target must match active turn; stale target is
  rejected and MUST NOT stop a newer turn. Return interrupted or rejected;
  turn_id echoes the target even on rejection.
* `interrupt_and_submit`: `{turn_id,text,attachments?}`. Targeted replacement:
  validate input before interrupt, then stop target and start replacement without
  an intervening external mutation. Return started or rejected. Always include
  `interrupted_turn_id`: null if target was not interrupted; target ID if it was,
  INCLUDING when replacement failed. This partial rejected result must not be
  collapsed into a generic error or a claim that nothing happened.
* `approve`: `{call_id,approve}`. Tool approval only, requiring explicit
  external_approvals opt-in. call_id MUST equal the actual pending tool call ID,
  never a turn ID. Adapter checks pending_approval and forwards that exact ID as
  `ApproveTool {approve,call_id:Some(id)}`. The daemon's superseded-call check is
  mandatory: a preflight snapshot check alone is NOT race-safe. Return
  `{approved:boolean}` echoing the decision after acknowledgment. Missing/stale
  IDs fail `invalid_state`, never retry against the newer pending call. Plan,
  mission, and unknown gates are unsupported, even if they share a pending tool
  representation: fail rather than synthesize keys or generic approvals.
* `detach`: `{}` -> `{detached:true}`.
* `configure`: `{providers?, models?}`. At least one is required. `providers` lists
  `{name, endpoint, api_type, api_key?}`. `api_type` is `openai` or `anthropic`.
  `endpoint` is an http(s) URL. Names in one call are unique. A new provider
  requires `api_key`. The same user-owned name updates that provider and keeps
  its uuid; omitting `api_key` (or JSON null) keeps the stored key, and an empty
  key is `invalid_request`. Extension-owned names are rejected. `models` maps
  `main`, `awareness`, `safeguard`, `compactor`, and `planner` to
  `{provider, model_id, route?}`. These five are the only role keys; any other
  key (including `cyber-*`) fails `invalid_request` with `not_sent`. The first
  call on an empty catalogue must include `models.main`. A later call, including
  one on a live session, may name any subset. Unmentioned roles stay. The same
  provider and model id share one catalogue entry.
  Return `{providers:[{name,uuid}], models:{role:{uuid,model_id,provider}}}`
  for what this call changed. The result, events, and errors never include an API
  key. Before a session, the call writes `config.json` and does not start a
  daemon. On a live session the running catalogue updates and the next turn uses
  it; a turn already streaming keeps the model it started with. When `models.main`
  is included on a live session, that session's Main is re-pinned. An already-open
  Koma window keeps its in-memory catalogue until restart.

TurnResult is `{disposition,turn_id,reason}`; reason is null on success or
`{code,message}` on rejection. Rejected is a known negative domain outcome, not
an RPC error. started requires a new nonnull turn_id; queued requires null;
interrupted requires the target ID. Replacement adds interrupted_turn_id and
has only started/rejected dispositions. Suggested reason codes: busy, stale_turn,
queue_full, invalid_input, no_session, unavailable, internal. Codes are open
strings; do not parse human messages. RPC codes include invalid_request,
unsupported_method, invalid_state, not_found, unavailable, internal. Ordinary
schema/preflight failures use not_sent; an ambiguous daemon disconnect uses
unknown. Definitive daemon rejections remain known negative results.

## DTOs and normalization

State fields are always present: session_id, revision, cwd, model_id, policy,
external_approvals, turn, working, awaiting_approval, approval_reason,
pending_approval, gate, pending_count, messages, streaming.
turn is null before any tracked turn, otherwise `{id,state,error}`; states are
running/finished/interrupted/failed, error is nullable text. Preserve the last
final turn until the next tracked turn starts; never infer success from idle.
working is daemon activity (not a synonym for turn.running). awaiting_approval
and approval_reason retain the daemon's gate indication, including unsupported
gates, and MUST NOT be treated as sufficient authority to approve.
`pending_approval` is null or `{call_id,reason}` (reason nullable text): populated
only for an awaiting ordinary tool at pending_tool_calls[tool_idx], using its
actual id. It remains visible with external_approvals false; visibility is not
authorization. `gate` is null when no gate is observed, otherwise
`{kind,reason,supported}`; kind is tool/plan/mission/unknown, reason nullable text.
Classify plan_ready as plan and mission_ready as mission BEFORE considering tool
approval. supported is true ONLY for an ordinary tool with a known call ID and
external_approvals enabled; plan/mission/unknown are always false. If awaiting
approval has no identifiable pending call, expose unknown, not a safe tool gate.
pending_count is 0..5. messages are normalized `{role,content}` with roles
system/user/assistant/tool; content is text only (empty if no text). Private
tool-call JSON, reasoning, credentials, composer drafts and attachment storage
paths are excluded. streaming is nullable in-progress assistant text, not a
committed transcript entry. These DTOs are independent public projections, never
serialized aliases of private daemon structs.

Policy is credential-free: plan allow/forbid and approval default/reject.
`reject` means unattended denial, NOT auto-approval or a sandbox. Unsupported
policy must fail explicitly before accepting user input. Policy is session scoped.
`external_approvals` is a connection-scoped boolean, default false on BOTH create
and resume, never inherited from a previous controller. An explicit policy with
approval default requires external_approvals true or fails invalid_request before
policy mutation. Resume without policy preserves persisted policy (even default),
but does not enable the external approval channel: pending gates stay visible and
approve fails invalid_state until a new explicitly opted-in connection is made.
Opt-in alone does not rewrite policy; create still defaults to forbid/reject unless
policy is explicitly supplied. A user of external approvals normally supplies
both external_approvals true and policy approval default.

Attachments are request-owned, ordered, and never use a shared composer:
`{kind:"image_file",path}` (absolute regular file on daemon host, recognized
image, model must support images) or `{kind:"pasted_text",text,name?}` (name is
nullable display label, sanitized by daemon). No base64, URL fetching, or path
interpretation in prompt text. Entire batch is preflighted before interrupt or
submission. Accepted bytes are ingested into session-owned storage; later source
file changes do not mutate queued content. Max 16 attachments, each pasted text
<= 2 MiB UTF-8, all attachments together <= 32 MiB raw bytes including images.
Prompt must be nonblank and <= 2 MiB UTF-8. Counts are bytes, NOT characters.

## Bounds and forward compatibility

A frame is at most **40 MiB (41943040 bytes)** excluding LF (including any CR).
This is a serialized-byte cap, independent of decoded payload limits; escaping
can cause otherwise valid 32 MiB inputs to exceed it. Reject locally rather than
truncate; caller may split pasted text. Readers must bound allocation while
reading, not read an unlimited line then check. Overlong or invalid UTF-8/JSON
stdout is fatal; outstanding dispatched requests become delivery unknown.
Adapters reject malformed stdin frames by stderr diagnostic and nonzero exit
(no trustworthy correlation). Schema-invalid but correlated requests may receive
invalid_request/not_sent. Unknown fields are ignored by readers for additive v1
compatibility, but writers emit only specified fields. Unknown event names may
be ignored after validating sequence; unknown response IDs are ignored.

Client pending requests: at most 64; fail excess before writing. Client event
queue: at most 256 frames AND 64 MiB serialized bytes. Adapters may coalesce
unsent full-state events BEFORE assigning seq. A client may not silently drop
received events: on queue overflow, clear buffered events and their byte accounting,
pause event buffering, and report event_overflow once through event consumption.
Overflow alone MUST NOT fail pending requests, mark delivery unknown, or detach.
Continue validating frames and correlating responses while buffering is paused.
Polling-only submit-and-wait usage is supported without consuming events.
After the error is observed, restarting the single event iterator enables buffering
and obtains authoritative state by readback. Yield that state with resynced true,
ignore queued observations superseded by its revision, then accept later appends.
A second overflow during recovery is another explicit event-stream error.
Polling waits never consume an active iterator's events. Malformed frames, EOF,
process failure, and transport errors remain connection-fatal.
Drain responses independently of event consumers; bound stderr independently.
An adapter must not accumulate unbounded snapshots while stdout is blocked.
State too large for a frame produces unavailable/not_sent on explicit state
queries; an unsolicited oversized state is a fatal diagnostic, never truncated.

## Additive extensions: rules

Methods, keys and errors added after the ten v1 methods follow these rules.
A client that never calls a new method or sends a new param sees the same
results, events and errors as before, apart from the optional `session_info`
key beside `state` (see Session info) and the optional error keys on the two
binding errors (see Bindings).

* Additive. The ten v1 methods keep their names, params, results, error codes,
  message texts and behaviour.
* One outcome per mutation. Every new mutation ends in exactly one of three
  outcomes. A result means the change is committed and verified by reading it
  back. An error with `delivery:"not_sent"` means no effective change: the
  adapter refused before writing, or it verified that nothing changed. An
  error with `delivery:"unknown"` means the adapter cannot tell; it carries
  `written:true` when bytes are known to be committed. Side effects after a
  commit (rebinding sessions, reloading daemons) are reported in
  `result.propagation` and never turn a committed write into an error.
* Validate first. Parsing, bounds, existence, binding and compare-and-swap
  checks all run before the first write.
* Concurrent writers are assumed: other adapters, running sessions, the
  terminal and GUI, and the model's own tools. See Concurrent writers.
* Secrets are write-only. See Secrets.
* Old peers degrade predictably. An adapter never sends a daemon a request the
  daemon did not advertise; a missing peer feature gives a documented fallback
  or `reason:"peer_unsupported"`.
* New enum-like strings are open. Clients map an unknown value of a string
  introduced after v1 (reasons, bindings, `key_state`, `origin`, `propagation`
  values, and so on) to "other". The closed v1 enums (message `role`,
  `gate.kind`, `turn.state`, `disposition`, policy) and `PublicRole` never gain
  values.
* No new event kind is emitted in UNBOUND or CODING. A client that only drives
  coding sessions sees exactly the v1 events `state`, `output_append` and
  `turn`.
* Params are objects. Unknown params are ignored, as in v1, so a client checks
  `methods` and `features` before relying on a new param.
* Bounds. A value outside its stated bounds is `invalid_request` with
  `reason:"out_of_range"`. Values are never clamped silently.
* Times. `*_at` fields are unix seconds, `at_ms` and `*_at_ms` fields are unix
  milliseconds, and every other `*_ms` field is a duration in milliseconds.
* Lists. Every list is either cursor-paged or has a stated maximum and a
  sibling `<name>_truncated` boolean. A client shows that a truncated list is
  incomplete.
* Revisions. A file-backed document has a `revision`: the first 16 lowercase
  hex characters of SHA-256 over its bytes, or `"absent"` when the file does
  not exist. A mutation that takes `if_revision` fails with `invalid_state` and
  `reason:"stale"` when the document no longer has that revision.
* Mutation results return the fresh document plus `propagation`, so a screen
  re-renders without another read.
* Idempotency below says what a blind retry does. Errors are `not_sent` unless
  stated otherwise.

## Notation

The sections below describe DTOs and params in this notation. `int` is a JSON
integer, `num` a JSON number, `bool` a JSON boolean. `string(a..b)` bounds a
length in characters on input. Id fields
(`*_uuid`) are `string(1..64)`. `T|null` is always present and may be null.
`k?: T` is an optional key that may be absent; an optional key is never sent
as null. `[T] (≤ n)` is a list with a stated maximum.

## Discovery

hello may carry four optional keys after the v1 keys:

* `methods`: every method this adapter implements, including the v1 methods.
* `features`: optional behaviours, static per binary, never dependent on the
  binding.
* `events`: every event kind this adapter can emit.
* `limits`: numeric bounds (see Limits).

```json
{"type":"hello","protocol":1,"binary":{"path":"/usr/local/bin/koma","version":"1.0.0"},"diagnostics":[],
 "capabilities":["create","resume","state","submit","enqueue","interrupt","interrupt_and_submit","approve","configure","detach"],
 "methods":["create","resume","state","submit","enqueue","interrupt","interrupt_and_submit","approve","configure","detach",
            "config.get","config.providers.delete","config.models.delete","config.roles.unassign",
            "config.effort.get","config.effort.set","config.limits.get","config.limits.set","sessions.info"],
 "features":["errors.reason","state.session_info","config.lock"],
 "events":["state","output_append","turn"],
 "limits":{"frame_bytes":41943040,"request_ids":65536,"text_bytes":2097152,
           "attachments":{"count":16,"pasted_bytes":2097152,"total_bytes":33554432},
           "config":{"lock_wait_ms":3000},"time":{"ipc_ms":15000}}}
```

`capabilities` is always exactly the ten v1 names, in every binding, so a v1
client keeps working. New clients gate calls on `methods` and fall back to
`capabilities` when `methods` is absent (an older adapter, which implements
only v1). Names are added over time; ignore names you do not know.

| feature | meaning |
|---|---|
| `errors.reason` | errors may carry `reason`, `binding`, `allowed`, `written` and `detail` |
| `state.session_info` | `create`, `resume` and `state` results carry a sibling `session_info`, and `sessions.info` exists |
| `config.lock` | config writes take the SDK config lock (wait bound in `limits.config.lock_wait_ms`) |

hello is written before the adapter reads any request, so it carries adapter
facts only. Facts about the daemon a session runs in are binding-time data:
see `session_info.daemon`.

## Bindings

One adapter process holds at most one binding. The binding is one of
`unbound`, `coding`, `room_live` or `room_closed`; the two room states exist
only when hello `features` lists `bindings.room`. Without it an adapter is
UNBOUND until `create` or `resume` succeeds, then CODING until it exits. CODING
never returns to UNBOUND, as in v1. A dashboard runs one adapter per coding
session it drives, plus one long-lived UNBOUND adapter for host screens.

Methods fall into families:

| family | methods |
|---|---|
| legacy session | `state`, `submit`, `enqueue`, `interrupt`, `interrupt_and_submit`, `approve` |
| binders | `create`, `resume` |
| host (any binding) | `config.get{view:"global"}`, `config.providers.delete`, `config.models.delete`, `config.roles.unassign{scope:"global"}` |
| coding-scoped | `sessions.info`, `config.get{view:"session"}`, `config.roles.unassign{scope:"session"}`, `config.effort.get`, `config.effort.set`, `config.limits.get`, `config.limits.set` |
| other | `configure`, `detach` |

| | UNBOUND | CODING |
|---|---|---|
| legacy session | E1 | yes |
| binders | yes | E3 |
| host | yes | yes |
| coding-scoped | E2 | yes |
| `configure` | v1 unbound path | v1 bound path |
| `detach` | v1 error `no session bound` (no added keys), then exit | v1 |

Effort, output and context limits and the session view of config exist only
in a running session's settings; there is no global default to edit, so those
methods need CODING.

A call in the wrong binding fails with `delivery:"not_sent"` and the additive
keys `reason`, `binding` (the current state) and `allowed` (the states where
the call is allowed):

| id | code | message | reason |
|---|---|---|---|
| E1 | `invalid_state` | `no session bound` (v1 text) | `wrong_binding` |
| E2 | `invalid_state` | `<method> requires binding <allowed>; this adapter is <binding>` | `wrong_binding` |
| E3 | `invalid_state` | `session already bound` (v1 text) | `already_bound` |
| E4 | `invalid_state` | `the room connection is closed` | `room_closed` (room bindings only) |

Example:
`{"code":"invalid_state","delivery":"not_sent","message":"config.effort.get requires binding coding; this adapter is unbound","reason":"wrong_binding","binding":"unbound","allowed":["coding"]}`.
E1 and E3 keep their v1 code and message and only gain keys. Unknown method
names stay `unsupported_method`. Branch on `reason`, never on the message.

Process lifetime. An `unknown` outcome on the bound coding connection ends the
process after the response, exactly as in v1. This covers every v1 method and
every new call that talks to the bound session's daemon: the coding-scoped
methods and CODING-path config deletes. Host work done inside the adapter
(reading and writing files, one-shot connections to other daemons) never ends
the process. Losing the bound coding connection still ends the process.

## Errors

Every error introduced after v1 carries `reason`, with two exceptions:
`internal` errors (a defect in the adapter), and errors that come from the
bound coding connection's v1 request path (an `unknown` outcome, or the
daemon's own error reply to a read), which keep their v1 shape. With feature
`errors.reason`, errors may carry these optional keys; old clients ignore them:

* `reason`: an open string from the registry below. The reason fixes the code
  and the delivery.
* `binding`, `allowed`: on binding errors (see Bindings).
* `written`: `true` when an `unknown` outcome is known to have committed bytes.
* `detail`: an object of at most 16 KiB, never secret. Lists inside it
  (`warnings`, `diff`) hold at most 100 items, with `truncated:true` when cut.

v1 errors, and v1 `unknown` outcomes on the bound coding connection, keep their
v1 shape and carry no `reason`, except E1 and E3 as above.

Reasons used by the config and session methods (more are added with later
method families; treat an unknown reason by its `code` and `delivery`):

| reason | code | delivery | where | meaning |
|---|---|---|---|---|
| `wrong_binding` | `invalid_state` | not_sent | any (E1, E2) | the call is not allowed in this binding |
| `already_bound` | `invalid_state` | not_sent | binders (E3) | a binding is held |
| `peer_unsupported` | `invalid_state` | not_sent | calls that need the daemon | the daemon lacks the needed feature |
| `out_of_range` | `invalid_request` | not_sent | any | a value is outside its bounds; a session-scope `config.roles.unassign` of a role other than `main`; a `config.models.delete` `scope` other than `"global"` |
| `missing_params` | `invalid_request` | not_sent | `config.limits.set` | none of the required alternatives was given |
| `conflicting_params` | `invalid_request` | not_sent | `config.roles.unassign` | params that exclude each other were combined (`if_revision` with session scope) |
| `not_offered` | `invalid_request` | not_sent | `config.effort.set` | the effort is not among the options |
| `not_found` | `not_found` | not_sent | addressed objects | the addressed provider or model does not exist |
| `stale` | `invalid_state` | not_sent | mutations with `if_revision` | the document changed |
| `managed` | `invalid_state` | not_sent | `config.providers.delete` | the provider is managed by an extension |
| `main_changed` | `invalid_state` | not_sent | `config.effort.set` | the session's Main model changed |
| `options_not_ready` | `invalid_state` | not_sent | `config.effort.set` | effort options are still loading |
| `daemon_refused` | `invalid_state` | not_sent | daemon-mediated writes | the daemon refused and the readback shows no change |
| `lock_timeout` | `unavailable` | not_sent | config writes | the SDK config lock was not acquired in time |
| `config_unreadable` | `unavailable` | not_sent | `config.*` | the config file is corrupt or unreadable; it is left untouched |
| `save_failed` | `unavailable` | not_sent | config writes in the adapter | the save failed and the file is verified unchanged |
| `frame_limit` | `unavailable` | not_sent | reads | the result would exceed the frame |
| `io_error` | `unavailable` | not_sent | reads and pre-write steps | a file read failed, the bound session connection was already lost, or the daemon answered a read with an unexpected reply |
| `unverified` | `unavailable` | unknown, `written:true` | config writes | the readback does not match the intended document |
| `partial_apply` | `unavailable` | unknown (`written:true` when on disk) | daemon-mediated writes | the daemon refused, but a change is in effect |

`unverified` carries `detail.diff`, naming the uuids that differ (never
values). `partial_apply` carries `detail:{in_memory, on_disk, daemon_error}`.

## Writes and propagation

Config writes are made in one of two ways, and a client sees the difference
only in `propagation` and in the possible errors.

Adapter writes (UNBOUND, and global `config.roles.unassign` in any binding):
the adapter takes the SDK config lock, loads and validates the config file,
compares `if_revision`, applies the change, saves through a temporary file and
rename, and reads the file back. It then rebinds sessions and agents on disk
that referenced a removed model or provider, and asks every live coding
daemon to reload the catalogue.

Daemon-mediated writes (CODING): config deletes and every session setting go
through the bound session's daemon, which saves, broadcasts and rebinds
itself. Before a global delete the adapter makes the daemon reload the config
file, so a change it had not loaded is not reverted, then compares the whole
document after the delete. A daemon refusal is not proof of no change: the
adapter reads back and reports `daemon_refused` (nothing changed) or
`partial_apply` (a change is in effect).

The SDK config lock is an advisory lock on `config.json.sdk-lock` next to the
config file, waited for up to `limits.config.lock_wait_ms`. Only SDK adapters
take it; see Concurrent writers.

`dry_run:true` on a delete runs every check, returns the computed effect with
the current view, and writes nothing (no daemon request is sent either).

`propagation` reports what happened after the commit. Every key is optional
and every value is an open string:

| key | values | meaning |
|---|---|---|
| `bound` | `reloaded`, `failed`, `not_bound`, `applied_by_daemon` | the bound session's daemon picked up the change |
| `broadcast` | `sent`, `failed`, `sent_by_daemon` | other live coding daemons were asked to reload; `failed` means at least one could not be reached |
| `rebind` | `done`, `failed`, `not_needed`, `done_by_daemon` | sessions and agents that used a removed model or provider were moved |
| `rooms` | `next_start` | running rooms pick the change up when they next start |

A `failed` propagation value is not an error: the write is committed. If the
bound coding connection fails while it is being told to reload, the response
is still the committed result, and then the process ends (see Bindings).

## Session info

With feature `state.session_info`, per-session facts that change without a
state revision travel in a sibling `session_info`, never inside `state`:

* `create`, `resume` and `state` results become `{state, session_info}`. The
  `state` object is the v1 object, byte for byte.
* `state` events are unchanged and carry no `session_info`.
* `sessions.info` returns `{session_info}`.
* coding-path config mutation results add `session_info`.

```
SessionInfo  = {name: string, effort: string, routes: [RoleRoute] (5), limits: Limits,
                usage: SessionUsage|null,
                daemon: {version: string, ipc_protocol: int, capabilities: [string] (≤ 64)},
                observed_at_ms: int}
SessionUsage = {last_prompt_tokens: int, last_cached_tokens: int, output_tokens_total: int, cost_total: num,
                context: null|{prompt_tokens: int, effective_window: int, estimated: bool,
                               cached_tokens: int|null, drss_active: bool}}
```

* `name` is the session's display name; `effort` spells the model default
  `"default"`; `routes` has one entry per public role; `limits` is the last
  read of the session's limits; `daemon` describes the daemon the session runs
  in. `usage` is `null` in this release.
* `observed_at_ms` is the adapter's clock when it received the newest frame
  that contributed to these values.
* Client rule: `session_info` is not subject to `revision`. Apply the one with
  the highest `observed_at_ms`, regardless of the state revision it came with.
  A change to `session_info` emits no event and does not bump the revision;
  call `sessions.info` or `state` to see fresh values.

## Config methods

```
ConfigView   = {revision: string, file: "ok"|"missing", view: "global"|"session",
                providers: [Provider] (≤ 256), providers_truncated: bool,
                oauth: [OAuthConn] (≤ 64), oauth_truncated: bool,
                models: [Model] (≤ 2000), models_truncated: bool,
                roles: [RoleRoute] (5),
                main_fallback: null|"unconfigured"|"provider_removed"|"no_key"|"not_signed_in"}
Provider     = {provider_uuid: string, name: string, api_type: string, endpoint: string, endpoint_redacted: bool,
                managed: bool, has_key: bool, key_state: "set"|"missing"|"unreadable", model_count: int}
OAuthConn    = {oauth_uuid: string, name: string, provider: string, has_token: bool, expires_at: int (0 = never)}
Model        = {model_uuid: string, name: string, model_id: string, provider_uuid: string,
                provider_kind: "provider"|"oauth"|"missing", provider_name: string|null, route: string|null,
                roles: [PublicRole] (≤ 5), other_roles: bool, source_uuid: string|null}
PublicRole   = "main"|"awareness"|"safeguard"|"compactor"|"planner"                      (closed)
RoleRoute    = {role: PublicRole, origin: "session"|"global"|"inherit_main"|"legacy"|"default",
                model_uuid: string|null, source_uuid: string|null, configured_model: string|null,
                provider_uuid: string|null, provider_name: string|null, effective_model: string|null,
                reason: null|"unsupported_route"|"credentials_unavailable"|"provider_missing"|"reference_deleted"|"unassigned"}
Limits       = {max_output_tokens: int, context_window_limit: int, context_model_alias: string,
                effective_window: int|null}
Propagation  = {bound?: string, broadcast?: string, rebind?: string, rooms?: string}
```

* `revision` is the revision of `config.json` in both views (`"absent"` when
  the file is missing; then `file` is `"missing"` and the lists are empty).
* `endpoint` has URL userinfo, query and fragment removed, and
  `endpoint_redacted` says whether anything was removed. Never feed a redacted
  endpoint back into `configure`.
* `managed` providers belong to an extension and cannot be deleted.
* OAuth connections are read-only here.
* `other_roles` is true when a model entry holds a role outside `PublicRole`;
  such roles are never named.
* `roles` and `routes` hold one `RoleRoute` per public role: who serves the
  role and why. `origin:"inherit_main"` means the role is unassigned and uses
  Main's route: `model_uuid` and `source_uuid` are null and `configured_model`
  is Main's model id. Only `compactor`, `awareness` and `planner` inherit Main;
  an unassigned `safeguard` fails closed and reports `origin:"legacy"` (a legacy
  classifier) or `"default"` with `reason:"unassigned"`. A non-null `reason` says why the configured route is not usable.
* `main_fallback` is non-null when Main cannot use a configured model.
* `models` is capped, not paged. `ConfigView` is one consistent snapshot of
  the config file, returned whole by every config mutation, so it cannot be
  paged without losing that. 2000 entries is far above any catalogue koma's
  own screens are built to show. When `models_truncated` is true, the view
  lists every model that holds a role first, then the rest by name, and a
  client must show that the list is incomplete.

### `config.get`

* Binding: `view:"global"` in any binding; `view:"session"` in CODING only (E2
  otherwise).
* Params: `{view?: "global"|"session"}`. The default is `"session"` in CODING,
  else `"global"`.
* Result: `ConfigView`. The session view shows the session's own role routes.
* Errors: `config_unreadable`.
* Idempotency: read. Reading may seal plain-text secrets in the config file,
  exactly as every koma reader does.

### `config.providers.delete`

* Binding: any.
* Params: `{provider_uuid: string(1..64), if_revision?: string, dry_run?: bool = false}`.
  Providers are addressed by uuid, because names are not unique.
* Result: `{dry_run: bool, removed: {provider_uuid, models_removed: [model_uuid], main_reset: bool},
  config: ConfigView, propagation, session_info?}`. The provider's models are
  removed with it; `main_reset` says Main lost its model. On a dry run,
  `config` is the current view.
* Errors: `not_found`; `managed`; `stale`; `lock_timeout`,
  `config_unreadable`, `save_failed`, `unverified`; in CODING also
  `daemon_refused` and `partial_apply`.
* Idempotency: a retry after success is `not_found` ("already gone").

### `config.models.delete`

* Binding: any. Global scope only: session-local model entries are not
  deleted through the SDK.
* Params: `{model_uuid: string(1..64), if_revision?: string, dry_run?: bool = false}`.
  There is no `scope` param, but a `scope` key with any value other than
  `"global"` is `out_of_range`, so a caller that expects a session delete never
  deletes globally.
* Result: `{dry_run: bool, removed: {model_uuid, roles_released: [PublicRole], main_reset: bool},
  config: ConfigView (view "global"), propagation, session_info?}`.
* Deleting the only Main holder makes Main fall back to the keyless default
  route, and `config.main_fallback` then reads `"unconfigured"`.
* Errors: `not_found`; `stale`; `out_of_range`; `lock_timeout`,
  `config_unreadable`, `save_failed`, `unverified`; in CODING also
  `daemon_refused` and `partial_apply`.
* Idempotency: a retry after success is `not_found`.

### `config.roles.unassign`

* Binding: `scope:"global"` in any binding; `scope:"session"` in CODING only,
  and only with `role:"main"`.
* Params: `{role: PublicRole, scope?: "global"|"session" = "global", if_revision?: string}`.
  Session scope with a role other than `main` is `out_of_range`; session scope
  with `if_revision` is `conflicting_params`.
* Result: `{changed: bool, previous_model_uuid: string|null, config: ConfigView, propagation, session_info?}`,
  with `config` in the view that matches the scope.
* Global: removes the role from the model that holds it. If nobody holds it,
  `changed` is false and nothing is written. An unassigned `compactor`,
  `awareness` or `planner` inherits Main; an unassigned `safeguard` never
  does, and falls back to its legacy or default route instead.
* Session `main`: clears the session's Main pin, so the session follows the
  global Main again; this also resets the session's effort to `"default"`.
  With no pin, `changed` is false and nothing is sent.
* Errors: `out_of_range`, `conflicting_params`, `stale`, `lock_timeout`,
  `config_unreadable`, `save_failed`, `unverified`; for session scope also
  `daemon_refused` and `partial_apply`.
* Idempotency: idempotent (`changed:false` on repeat).

### `config.effort.get`

* Binding: CODING.
* Params: `{wait_ms?: 0..10000 = 3000}`.
* Result: `{state: "ready"|"loading"|"unsupported", options: [string] (≤ 64), selected: string,
  note: string|null, main_model_uuid: string|null}`. `options` starts with
  `"default"`. `selected` is `"default"` when the session uses the model
  default. `main_model_uuid` is the session's Main model at the time of the
  read. While the options are `loading` the adapter waits up to `wait_ms` for
  them.
* Idempotency: read. It may make the daemon fetch the model catalogue from the
  configured provider endpoint.

### `config.effort.set`

* Binding: CODING.
* Params: `{effort: string(1..64), if_main_model?: string|null}`. `effort` has
  no whitespace or control characters; `"default"` selects the model default.
* Result: `{effort: string, main_model_uuid: string|null, superseded: bool, session_info?}`.
  Every output spells the model default `"default"`.
* `if_main_model` guards against a Main change since the client read the
  options: if it differs from the current Main, the call fails `main_changed`.
* The effort must be one of the options offered for the current Main (when
  options are `unsupported`, only `"default"` is accepted).
* After the write the adapter reads the effort back. If it differs because
  Main changed meanwhile (a Main change resets effort), the call fails
  `main_changed` with `not_sent`: none of this call's change is in effect. If
  it differs while Main is the same, another writer set effort after this
  call, and the result has `superseded:true`.
* Errors: `main_changed`, `options_not_ready`, `not_offered`, `out_of_range`,
  `daemon_refused`, `partial_apply`.
* Idempotency: idempotent.

### `config.limits.get`

* Binding: CODING.
* Params: `{}`.
* Result: `Limits`. `0` means automatic or the catalogue default;
  `context_model_alias` is `""` when unset; `effective_window` is the context
  window in effect, or `null` when not yet known.

### `config.limits.set`

* Binding: CODING.
* Params: at least one of `max_output_tokens?: 0 | 256..1000000`,
  `context_window_limit?: 0 | 1024..100000000`,
  `context_model_alias?: string(0..200)` (`""` clears). `0` means automatic or
  the catalogue default.
* Result: `Limits` after readback, plus `session_info?`.
* Errors: `missing_params`, `out_of_range`, `daemon_refused`, `partial_apply`.
* Applies from the session's next model call. Idempotent.

## Session methods

Shapes used here are defined under Session info and Config methods.

### `sessions.info`

* Binding: CODING.
* Params: `{}`.
* Result: `{session_info: SessionInfo}`. It costs no daemon request: it
  returns the adapter's latest values.
* Idempotency: read.

## Limits

`hello.limits` advertises bounds by key. Keys are added over time; treat an
absent key as unknown, and an absent `limits` as the v1 bounds.

| key | meaning |
|---|---|
| `frame_bytes` | maximum frame size in bytes (40 MiB) |
| `request_ids` | requests accepted per adapter process (see Request ids) |
| `text_bytes` | maximum prompt text in UTF-8 bytes |
| `attachments.count`, `attachments.pasted_bytes`, `attachments.total_bytes` | attachment bounds (see DTOs and normalization) |
| `config.lock_wait_ms` | how long a config write waits for the SDK config lock |
| `time.ipc_ms` | how long the adapter waits for one daemon reply |

v1 responses keep the v1 rule: a state too large for a frame becomes
`unavailable`/`not_sent`. A new read whose result would exceed the frame fails
with `reason:"frame_limit"`. New mutation results are bounded by construction
(`ConfigView` by its caps), so a committed mutation never loses its result.

## Request ids

An adapter accepts `limits.request_ids` (65,536) requests per process. The
next request is fatal: the adapter exits, as in v1. Clients must recycle the
adapter before the limit: an UNBOUND adapter holds no state, so a client
respawns it once 60,000 requests have been sent; a client driving a coding
session exposes the number of requests remaining, and the caller reconnects
with `resume` from a fresh adapter. For scale, polling one method every 5 s
uses about 17,000 ids per day.

## Secrets

Never returned, in any result, event, error or `detail`: API keys; OAuth
access, refresh and id tokens; sealed or unreadable stored secret values;
MCP server environment values; account and organization ids; credentials,
query strings and fragments in endpoints. The only key facts exposed are
`has_key`, `key_state`, `has_token` and `expires_at`.

* Keys travel in one direction only: in `configure` requests.
* Endpoints are returned with userinfo, query and fragment removed
  (`endpoint_redacted:true` when something was removed).
* Errors name fields, never the values of secret-bearing fields. New methods
  take no secrets. A `config_unreadable` error names only the kind and position
  of the fault (for example `config.json is not valid (data error at line 12,
  column 40)`), never text from the file.
* Daemon error text and `detail` returned by new methods have every configured
  secret value of 8 or more characters, and URL userinfo, replaced by
  `[redacted]`.
* The SDK lock file is created readable by its owner only. A symlink or any
  other non-file at the lock path is refused (`io_error`), so the lock never
  creates or locks a file elsewhere.

Trust boundary: the adapter's stdin is as powerful as the koma user who runs
it. A gateway that forwards frames from elsewhere must authenticate the sender
first.

## Concurrent writers

Some writers take no SDK lock: running coding daemons, the GUI, and v1
`configure`. For `config.json` the last writer wins, and a daemon with a stale
catalogue can revert an SDK change on its next whole-file save. The adapter
narrows this with `if_revision`, the reload before a daemon delete, the
whole-document compare after it, the broadcast after every SDK write, and the
fresh view in every result. A client that must not overwrite a change it has
not seen passes `if_revision` from its last `ConfigView`.

In CODING the adapter changes the bound session's settings only through its
daemon. The rebind after an UNBOUND delete can rewrite the settings file of
another running session on disk; that session converges when its daemon next
reloads.

## Known defects of `configure`

`configure` is frozen v1 behaviour and is not changed. Clients should know
three defects; the `config.*` methods avoid all of them:

* In UNBOUND, `configure` loads the config file leniently: a corrupt
  `config.json` is treated as empty and can be overwritten with a catalogue
  that holds only the providers and models of this call.
* It never asks live daemons to reload, so a running session can later revert
  the change with its own whole-file save.
* In CODING, once the daemon has acknowledged and applied the change, a
  failure to read the result back is reported as `unavailable` with
  `delivery:"not_sent"`, although the change is in effect. Read back with
  `config.get` before retrying.

## Mapping rationale (non-normative)

Checked daemon sources: ipc/proto/turn.rs (policy, turn states, dispositions,
owned attachment variants); hub/requests_turn.rs (queued has no turn ID,
replacement can reject after interruption, target guard, tool vs plan approval);
  ipc/proto/mod.rs (ApproveTool optional call_id) and hub/requests.rs (mandatory
  supplied-ID superseded-call guard);
actions/turn_input.rs (16 attachments, 2 MiB paste, 32 MiB aggregate);
ipc/proto/snapshot/session.rs (working, approval, messages, streaming, pending).
These public DTOs deliberately do not serialize those private structs directly.
Fixtures are representative wire lines, not executable scripts; JSON Schema
alone cannot express byte budgets, method/result correlation or lifecycle rules.
