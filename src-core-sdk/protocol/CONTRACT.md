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

* `create`: `{cwd, model_id?, policy?, external_approvals?, name?}`. cwd is an absolute path on the daemon
  host. Optional model_id selects the main model; absent uses configured default.
  Optional `name: string(1..200)` names the session (feature `create.name`; an
  older adapter ignores unknown params, so check the feature first). A bad
  `name` is rejected before anything is started. Without `name`, `create`
  behaves exactly as in v1. If the rename fails after the session is created and
  bound, the session stays and the result carries
  `warnings:[{code:"rename_failed", message}]` (at most 8), and
  `session_info.name` is the session id.
  policy defaults to `{plan:"forbid",approval:"reject"}`. Return `{state}` only
  after session binding and requested policy have succeeded.
* `resume`: `{session_id, policy?, external_approvals?}`. Preserve persisted policy if omitted;
  explicit policy is installed before success. Return `{state}`.
  `resume` on a secure-room session id is unsupported and unsafe: room sessions
  are not coding sessions. When the room is not running, `resume` starts a
  coding session on the room's directory. Use `sessions.list` and its `kind`
  to tell the two apart, and never pass a room's id to `resume`.
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
            "config.effort.get","config.effort.set","config.limits.get","config.limits.set","sessions.info",
            "sessions.list","sessions.rename","sessions.delete","sessions.stop","usage.query",
            "skills.list","skills.get","skills.read_file","skills.install","skills.remove","skills.create","skills.update"],
 "features":["errors.reason","state.session_info","create.name","config.lock"],
 "events":["state","output_append","turn"],
 "limits":{"frame_bytes":41943040,"request_ids":65536,"text_bytes":2097152,
           "attachments":{"count":16,"pasted_bytes":2097152,"total_bytes":33554432},
           "list":{"sessions":500,"skills":1000,"usage_groups":500,"usage_buckets":1000},
           "skills":{"inline_zip_bytes":29360128,"zip_bytes":67108864,"entry_bytes":2097152,"companion_bytes":8388608,
                     "files":1000,"depth":16,"name_bytes":64},
           "config":{"lock_wait_ms":3000},"time":{"ipc_ms":15000,"usage_query_ms":2000,"session_stop_ms":8000}}}
```

`capabilities` is always exactly the ten v1 names, in every binding, so a v1
client keeps working. New clients gate calls on `methods` and fall back to
`capabilities` when `methods` is absent (an older adapter, which implements
only v1). Names are added over time; ignore names you do not know.

| feature | meaning |
|---|---|
| `errors.reason` | errors may carry `reason`, `binding`, `allowed`, `written` and `detail` |
| `state.session_info` | `create`, `resume` and `state` results carry a sibling `session_info`, and `sessions.info` exists |
| `create.name` | `create` accepts `name` |
| `config.lock` | config writes take the SDK config lock (wait bound in `limits.config.lock_wait_ms`) |
| `bindings.room`, `events.room` | the room bindings, the `room.*` methods and the `room.*` events exist (see Room methods and Room events) |
| `room.template` | team methods accept `target:"template"`, and the computer-wide room template layer exists (see Team setup) |

hello is written before the adapter reads any request, so it carries adapter
facts only. Facts about the daemon a session runs in are binding-time data:
see `session_info.daemon` for a coding session and `AttachResult.daemon` for a
room.

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

With feature `bindings.room` the families grow: `room.open` and `room.attach`
are binders; `sessions.*` (except `sessions.info`), `usage.query`, `skills.*`,
`room.list` and team methods with `target:"template"` are host methods; the
remaining `room.*` methods and team methods with `target:"room"` are
room-scoped. Team methods are `room.team.get`, `room.models.set`,
`room.avatars.upsert` and `room.avatars.remove`.

| | UNBOUND | CODING | ROOM(live) | ROOM(closed) |
|---|---|---|---|---|
| legacy session | E1 | yes | E2 | E2 |
| `create`, `resume` | yes | E3 | E3 | E3 |
| `room.open`, `room.attach` | yes | E3 | E3 | yes (rebinds) |
| host | yes | yes | yes | yes |
| coding-scoped | E2 | yes | E2 | E2 |
| room reads (`room.status`, `room.history`, `room.shelf.list`, `room.item`, `room.trajectory`, `room.reports`, `room.spend`, `room.team.get{target:"room"}`) | E2 | E2 | yes | yes |
| room socket writes (`room.send`, `room.interrupt`, `room.control`) | E2 | E2 | yes | E4 |
| team writes with `target:"room"` | E2 | E2 | yes | yes (file only) |
| `room.close` | E2 | E2 | yes | yes |
| `configure` | v1 unbound path | v1 bound path | v1 unbound path | v1 unbound path |
| `detach` | v1 error `no session bound` (no added keys), then exit | v1 | leaves the room, `{detached:true}`, exit 0 | `{detached:true}`, exit 0 |

The room states:

* ROOM(live) is entered by a successful `room.open` or `room.attach`. The
  adapter holds the room's single live subscription (see Room methods,
  Takeover).
* ROOM(closed) is entered when the room connection ends: another client took
  the room over, the room daemon exited, or the stream failed (a `room.closed`
  event says which), or a socket write ended with an unknown outcome. The
  line history, the epoch, the last status and the room identity are kept.
  Reads and file-only team writes keep working; socket writes fail with E4.
* `room.open` or `room.attach` in ROOM(closed) drops the dead binding and binds
  anew in one call. This is the explicit take-back.
* `room.close` returns the adapter to UNBOUND. A room binding is the only one
  that can return to UNBOUND.
* `detach` in ROOM(live) tells the room this client is leaving and then exits.
  It never stops the room; the room keeps running for the next client. Stop a
  room with `room.close{stop:true}` or `sessions.stop`.

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
the process. Losing the bound coding connection still ends the process. In a
room binding no outcome ends the process: an unknown outcome of a room socket
write moves the adapter to ROOM(closed) and emits
`room.closed{reason:"stream_error"}`.

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

Reasons used by the config, session, usage and skills methods (more are added with later
method families; treat an unknown reason by its `code` and `delivery`):

| reason | code | delivery | where | meaning |
|---|---|---|---|---|
| `wrong_binding` | `invalid_state` | not_sent | any (E1, E2) | the call is not allowed in this binding |
| `already_bound` | `invalid_state` | not_sent | binders (E3) | a binding is held |
| `peer_unsupported` | `invalid_state` | not_sent | calls that need the daemon | the daemon lacks the needed feature |
| `out_of_range` | `invalid_request` | not_sent | any | a value is outside its bounds; a session-scope `config.roles.unassign` of a role other than `main`; a `config.models.delete` `scope` other than `"global"` |
| `missing_params` | `invalid_request` | not_sent | `config.limits.set`, `skills.update` | none of the required alternatives was given |
| `conflicting_params` | `invalid_request` | not_sent | `config.roles.unassign`, `skills.install` | params that exclude each other were combined (`if_revision` with session scope; both or neither of `data_b64` and `path`) |
| `bad_cursor` | `invalid_request` | not_sent | `sessions.list`, `skills.list` | the cursor does not decode |
| `invalid_name` | `invalid_request` | not_sent | `create`, `sessions.rename`, `skills.*` | a name breaks its rule or holds control characters |
| `range_too_large` | `invalid_request` | not_sent | `usage.query` | more than 400 days, or more than 1000 buckets |
| `not_offered` | `invalid_request` | not_sent | `config.effort.set` | the effort is not among the options |
| `workdir_required` | `invalid_request` | not_sent | `skills.*` | a project, `claude` or `extra-root` tier without `workdir` while unbound |
| `invalid_package` | `invalid_request` | not_sent | `skills.install` | the zip is malformed |
| `too_large` | `invalid_request` | not_sent | `skills.install` | a size cap was exceeded |
| `binary` | `invalid_request` | not_sent | `skills.read_file` | the file is not UTF-8 text |
| `not_found` | `not_found` | not_sent | addressed objects | the addressed provider, model, session or skill does not exist |
| `stale` | `invalid_state` | not_sent | mutations with `if_revision` or `generation` | the document changed |
| `managed` | `invalid_state` | not_sent | `config.providers.delete` | the provider is managed by an extension |
| `main_changed` | `invalid_state` | not_sent | `config.effort.set` | the session's Main model changed |
| `options_not_ready` | `invalid_state` | not_sent | `config.effort.set` | effort options are still loading |
| `daemon_refused` | `invalid_state` | not_sent | daemon-mediated writes | the daemon refused and the readback shows no change |
| `bound_self` | `invalid_state` | not_sent | `sessions.delete`, `sessions.stop` | the target is the session this adapter is bound to |
| `live` | `invalid_state` | not_sent | `sessions.delete`, `sessions.rename` | the session's daemon is running (for `sessions.rename`: another live coding session) |
| `session_locked` | `invalid_state` | not_sent | `sessions.delete` | the process that holds the session lock is alive |
| `exists` | `invalid_state` | not_sent | `skills.create`, `skills.install` | a skill of that name exists |
| `read_only` | `invalid_state` | not_sent | skill writes | the tier is not editable |
| `unsafe_yaml` | `invalid_state` | not_sent | `skills.update` | the skill's frontmatter cannot be rewritten safely |
| `lock_timeout` | `unavailable` | not_sent | config writes | the SDK config lock was not acquired in time |
| `config_unreadable` | `unavailable` | not_sent | `config.*` | the config file is corrupt or unreadable; it is left untouched |
| `save_failed` | `unavailable` | not_sent | config writes in the adapter | the save failed and the file is verified unchanged |
| `sqlite_busy` | `unavailable` | not_sent | `sessions.rename`, `sessions.list` | the session registry was busy |
| `query_timeout` | `unavailable` | not_sent | `usage.query` | the 2 second deadline passed |
| `frame_limit` | `unavailable` | not_sent | reads | the result would exceed the frame |
| `io_error` | `unavailable` | not_sent | reads and pre-write steps | a file or database read failed, the bound session connection was already lost, or the daemon answered a read with an unexpected reply |
| `unverified` | `unavailable` | unknown, `written:true` | config writes | the readback does not match the intended document |
| `partial_apply` | `unavailable` | unknown (`written:true` when on disk) | daemon-mediated writes | the daemon refused, but a change is in effect |
| `partial_delete` | `unavailable` | unknown, `written:true` | `sessions.delete` | removal failed midway; a retry is valid |
| `still_running` | `unavailable` | unknown | `sessions.stop` | the daemon is still alive after the stop sequence |

Reasons used by the room methods:

| reason | code | delivery | where | meaning |
|---|---|---|---|---|
| `room_closed` | `invalid_state` | not_sent | room socket writes (E4) | the room connection is closed |
| `peer_unsupported` | `invalid_state` | not_sent | `room.send` | the room is too old for attachments |
| `out_of_range` | `invalid_request` | not_sent | any | a value or `limit` is outside its bounds; the 64-avatar cap |
| `bad_cursor` | `invalid_request` | not_sent | `room.list`, `room.shelf.list`, `room.reports` | the cursor does not decode |
| `missing_params` | `invalid_request` | not_sent | `room.models.set` | neither `model` nor `effort` was given |
| `conflicting_params` | `invalid_request` | not_sent | `room.send`, `room.control`, `room.history` | params that exclude each other were combined |
| `target_required` | `invalid_request` | not_sent | team writes | `target` is missing |
| `note_required` | `invalid_request` | not_sent | `room.control` | `redirect` without `note` |
| `parent_is_room` | `invalid_request` | not_sent | `room.open` | the parent is itself a room |
| `clerk_not_addressable` | `invalid_request` | not_sent | `room.send` | `to:"clerk"` |
| `no_work_lane` | `invalid_request` | not_sent | `room.control` | `avatar:"chief"` |
| `no_such_lane` | `invalid_request` | not_sent | `room.models.set` | `(chief, work)` or `(clerk, chat)` |
| `attachment_refused` | `invalid_request` | not_sent | `room.send` | the room refused an attachment |
| `layer_rejected` | `invalid_request` | not_sent | team writes | the room's own loader rejects the candidate layer, or the layer file is already broken |
| `role_immutable` | `invalid_request` | not_sent | `room.avatars.upsert` | an existing avatar's role cannot change |
| `reserved_id` | `invalid_request` | not_sent | `room.avatars.upsert` | the `chief` or `clerk` role or id |
| `not_found` | `not_found` | not_sent | addressed objects | the parent session, room or shelf item does not exist |
| `unknown_model` | `not_found` | not_sent | `room.models.set` | the model choice id is not among the choices |
| `unknown_avatar` | `not_found` | not_sent | `room.send`, `room.control`, team writes | not in the roster or in the layer |
| `unknown_role` | `not_found` | not_sent | `room.avatars.upsert` | no such role |
| `stale` | `invalid_state` | not_sent | team writes with `if_revision` | the layer changed |
| `room_id_in_use` | `invalid_state` | not_sent | `room.open` | the room's id holds a coding session |
| `no_main_route` | `invalid_state` | not_sent | `room.open` | the parent has no usable Main model |
| `not_a_room` | `invalid_state` | not_sent | `room.open`, `room.attach` | the socket does not answer as a room |
| `not_running` | `invalid_state` | not_sent | `room.attach` | the room is not running; use `room.open` |
| `epoch_changed` | `invalid_state` | not_sent | `room.history` | the epoch is not the binding's |
| `rejected_by_room` | `invalid_state` | not_sent | team writes | the running room refused the change; the rollback is verified |
| `lane_running` | `invalid_state` | not_sent | `room.avatars.remove` | the avatar's lane is running |
| `lock_timeout` | `unavailable` | not_sent | team writes | the layer lock was not acquired in time |
| `query_timeout` | `unavailable` | not_sent | `room.spend` | the 2 second deadline passed |
| `frame_limit` | `unavailable` | not_sent | room reads | the result would exceed the frame |
| `io_error` | `unavailable` | not_sent | room reads and pre-write steps | a file or database read failed |
| `room_unreadable` | `unavailable` | not_sent | room reads | the room's shelf or team files cannot be read |
| `restore_failed` | `unavailable` | unknown, `written:true` | team writes | the rollback after a refusal could not be verified |
| `still_running` | `unavailable` | unknown | `room.close` | the room is still alive after `stop:true` |
| `room_start_failed` | `unavailable` | unknown | `room.open` | the room did not come up in time |
| `stream_error` | `unavailable` | unknown | `room.send`, `room.interrupt`, `room.control` | the room socket failed during or after the write |

`unverified` carries `detail.diff`, naming the uuids that differ (never
values). `room_start_failed` carries `detail.log_tail` (at most 2 KiB of the
room's start log, scrubbed). `layer_rejected` and `rejected_by_room` carry
`detail.warnings`. `partial_apply` carries `detail:{in_memory, on_disk, daemon_error}`.

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
| `skills` | `next_rebuild`, `room_restart` | skill writes only; see Skills methods |
| `room` | `reloaded`, `not_supported`, `not_live`, `failed` | team writes only; see Team setup |

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
  in. `usage` is `null` while the daemon has reported no usage.
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
Propagation  = {bound?: string, broadcast?: string, rebind?: string, rooms?: string, skills?: string, room?: string}
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

```
SessionRow = {session_id: string, name: string|null, workdir: string|null, kind: "coding"|"room",
              created_at: int|null, updated_at: int|null,
              live: bool, working: bool|null, locked: bool, bound: bool,
              parent_session_id: string|null,
              room_session_id: string|null, room_live: bool}
```

* `kind` is `room` for a secure-room session. `working` is `null` for rooms,
  because a room has no single working state.
* `live` says that a daemon for the session is running; `locked` says that the
  process holding the session's lock is alive; `bound` says that this adapter
  holds the session.
* `parent_session_id` is the coding session a room was started from.
  `room_session_id` and `room_live` are set on a coding session that has a room.
* `created_at` and `updated_at` are seconds, or `null` for a live session that
  has no registry entry.

Shapes used here are defined under Session info and Config methods.

### `sessions.info`

* Binding: CODING.
* Params: `{}`.
* Result: `{session_info: SessionInfo}`. It costs no daemon request: it
  returns the adapter's latest values.
* Idempotency: read.

### `sessions.list`

* Binding: any.
* Params: `{kind?: "coding"|"room"|"all" = "all", live?: bool, limit?: 1..500 = 100, cursor?: string}`.
  `live:true` keeps live sessions, `live:false` the others.
* Result: `{sessions: [SessionRow], next_cursor: string|null}`, ordered by
  `updated_at` descending, then `session_id`. Pass `next_cursor` back as
  `cursor` for the next page; a cursor that does not decode is `bad_cursor`.
* A live session that has no registry entry is listed with `name:null` and
  `workdir:null`. `name` always comes from the registry, so a running room
  shows the name it has now.
* Listing never deletes anything: it removes no session, socket or pid file,
  so it is safe to poll, including while a daemon is starting.
* A koma home with no sessions yet lists none; it is not an error.
* Errors: `out_of_range`, `bad_cursor`, `sqlite_busy`, `io_error`.
* Idempotency: read.

### `sessions.rename`

* Binding: any.
* Params: `{session_id: string(1..64), name: string(1..200)}`.
* Result: `{session: SessionRow}`.
* The bound coding session is renamed through its daemon and the result is
  confirmed by readback. A room (running or not) and a coding session that is
  not live are renamed in the session registry. Another live coding session is
  refused with `live`, and nothing is sent to its daemon: rename it from an
  adapter bound to it, or stop it first. A running room's own probe keeps the
  old name until it restarts; listings show the new one.
* Errors: `not_found`, `invalid_name`, `live`, `sqlite_busy`.
* Idempotency: idempotent.

### `sessions.delete`

* Binding: any.
* Params: `{session_id: string(1..64)}`.
* Result: `{deleted: true, kind: "coding"|"room", orphaned_room_session_id: string|null}`.
* Checks run in this order, and the daemon state is probed again right before
  the removal: `bound_self` (this adapter's own binding), `live` (its daemon
  is running), `session_locked` (the lock owner is alive).
* Removes the session directory only. It does not remove usage rows, the
  scratch directory, or a room that was started from the session;
  `orphaned_room_session_id` names such a room.
* Errors: `not_found`, `bound_self`, `live`, `session_locked`. A failure after
  removal started is `unavailable`, `delivery:"unknown"`, `written:true`,
  `reason:"partial_delete"`; a retry is valid.
* Idempotency: a retry after success is `not_found`.

### `sessions.stop`

* Binding: any.
* Params: `{session_id: string(1..64)}`.
* Result: `{stopped: true, was_live: bool}`. A session that is not live gives
  `{stopped:true, was_live:false}`.
* Asks the daemon to quit, then signals it if it does not. The whole call is
  bounded by `limits.time.session_stop_ms`.
* Hazard: stopping a session disconnects every client of that daemon, the
  terminal client included, and an approval that was waiting for an answer is
  lost.
* Errors: `bound_self` (stop the session you are bound to by closing it, or
  stop it from another adapter), `still_running` (`unknown`: the daemon is
  still alive afterwards).
* Idempotency: idempotent.

## Usage methods

```
UsageTotals = {calls: int, tokens_in: int, tokens_cached: int, tokens_out: int, cost: num}
UsageGroup  = UsageTotals + {key: string, label: string|null,
               role_kind?: "main"|"sub", agent?: string|null,      // group_by "role" only
               kind?: "coding"|"room"|null, deleted?: bool}        // group_by "session" only
UsageBucket = UsageTotals + {start_at: int}
```

* `key` is the model id for `group_by:"model"`, the raw role for `"role"` and
  the session id for `"session"`. `label` is the display name, or `null`.
* `agent` is null for `main`. For a session group, `kind` is `null` and
  `label` is `null` when the session is unknown, and `deleted:true` marks a
  session that has been deleted.

### `usage.query`

* Binding: any.
* Params, all optional:

  | param | type and bounds | default |
  |---|---|---|
  | `from_at` | int seconds | now minus 30 days |
  | `to_at` | int seconds, greater than `from_at` | now |
  | `session_id` | string | all sessions |
  | `workdir` | absolute path | all workdirs |
  | `group_by` | `"none"`, `"model"`, `"role"`, `"session"` | `"none"` |
  | `bucket` | `"none"`, `"hour"`, `"day"`, `"week"` | `"none"` |
  | `tz_offset_minutes` | -720..840 | the host's offset |
  | `limit` | 1..500, the number of groups | 100 |

  A range over 400 days, or one that needs more than 1000 buckets, is
  `invalid_request`/`range_too_large`.
* Result: `{from_at, to_at, ledger: "ok"|"missing", totals: UsageTotals}`, plus
  `groups: [UsageGroup]` with `groups_truncated: bool` when `group_by` is not
  `none` (by cost descending, then key), and `buckets: [UsageBucket]` when
  `bucket` is not `none` (ascending, empty buckets included). A missing ledger
  gives `ledger:"missing"` and zeros; an error is never an empty result.
* What is counted: rows exist for the role `main` and for `sub:<agent>` only.
  Awareness, safeguard and compaction calls are not recorded. A room's lanes
  are recorded under the room's session id, as `main` (the Chief) or
  `sub:<avatar>`. `tokens_in` is the prompt size of each call, so summing it
  counts a repeated prompt every time.
* The totals, groups and buckets of one result are separate reads, so a row
  written while the call runs can show in one part and not in another.
* Errors: `invalid_request` (with `out_of_range` or `range_too_large`),
  `query_timeout`, `io_error`.
* Hazard, dropped usage rows: the ledger is a SQLite file in the default
  journal mode, and the daemon's insert has no busy wait. While a reader holds
  the file, a daemon that records a call fails that insert at once, and the row
  is logged and lost. The adapter reads with short statements and a 2 second
  deadline, which keeps the window small but does not remove it. Poll
  `usage.query` no more than you need: a dashboard should refresh every 30
  seconds or slower, and should not poll in a tight loop.
* Idempotency: read.

## Skills methods

```
SkillRow    = {name: string, scope: "computer"|"project"|"external",
               tier: "global"|"claude"|"project-agent"|"project-agents"|"extra-root:<n>",
               description: string, triggers: [string] (≤ 64), triggers_truncated: bool,
               tags: [string] (≤ 64), tags_truncated: bool, pinned: bool, editable: bool,
               generation: string, path: string, effective: bool, shadowed_by: string|null}
SkillDetail = SkillRow + {instruction: string (≤ 2 MiB), declared_tools: [string] (≤ 64),
               declared_tools_truncated: bool, companion_files: [string] (≤ 1000),
               companion_files_truncated: bool, structured_save_supported: bool,
               raw: string|null (≤ 2 MiB; null unless `include_raw`)}
```

Skills are found in five tiers. A later tier wins when two skills share a
name; the others are listed with `effective:false` and `shadowed_by` naming
the winner's tier.

| `tier` | root | `scope` | editable |
|---|---|---|---|
| `global` | the computer-wide skills directory | `computer` | yes |
| `claude` | `<workdir>/.claude/skills` | `external` | no |
| `project-agent` | `<workdir>/.agent/skills` | `project` | yes, in place; new skills are never created here |
| `project-agents` | `<workdir>/.agents/skills` | `project` | yes; new project skills are created here |
| `extra-root:<n>` | configured extra skill root number `n` | `external` | no |

* A skill is addressed by `(tier, name)`, so a shadowed skill can still be read
  and two project tiers never collide. `skills.get` and `skills.read_file` may
  omit `tier` and then address the effective skill of that name. Writes to an
  existing skill (`update`, `remove`) require `tier`. `generation` is the
  optimistic token: pass the one you read, and a changed skill gives `stale`.
* `path` is for display. `scope` follows from `tier`. `skills.create` and
  `skills.install` take `scope` `computer` or `project`, which means tier
  `global` or `project-agents`.
* `workdir` is an absolute, existing directory, returned canonicalized. In
  CODING it defaults to the session's directory and in ROOM to the room's. In
  UNBOUND any project, `claude` or `extra-root` tier needs it, else
  `invalid_request`/`workdir_required`.
* New names match `^[a-z0-9]+(-[a-z0-9]+)*$`, at most 64 bytes, and are not
  `skill` or `readme`. Skills that exist on disk under other names are still
  listed and addressed exactly.
* Skill source is returned verbatim (see Secrets). Every `skills.*` call runs in
  the adapter, in every binding; none of them sends a request to a daemon.
* Mutation results carry `raw:null`.

`propagation.skills` is one of:

* `"next_rebuild"` (UNBOUND and CODING). A running session keeps a snapshot of
  the skill catalogue, rebuilt with its system prompt: when the session is
  loaded, when its agent mode changes, after compaction, when a memory file
  changes, and after some tool calls. It is not rebuilt before every prompt.
  So an added, removed or renamed skill reaches a running session at its next
  rebuild or reload, and an edit to the body of a skill already in the
  catalogue is read from its file the next time that skill is loaded.
* `"room_restart"` (ROOM). The room's catalogue is fixed when it starts. Edits
  to a body reach its lanes on their next run; additions, removals and renames
  need a room restart, and a skill that was already loaded and then removed
  keeps its cached body.

No event follows a skills mutation: the result is the only signal. Limits:
updating a skill drops YAML comments and formatting in its frontmatter;
companion files cannot be written; binary companion files cannot be read;
changes made by other processes are not observed until you list again.

### `skills.list`

* Binding: any.
* Params: `{scope?: "computer"|"project"|"external"|"all" = "all", tier?: string, workdir?: string,
  include_diagnostics?: bool, limit?: 1..1000 = 500, cursor?: string}`.
* Result: `{project_workdir: string|null, skills: [SkillRow], diagnostics: [{path, error}] (≤ 200),
  diagnostics_truncated: bool, next_cursor: string|null}`, ordered by tier in
  the order of the table, then name. `diagnostics` names skills that were
  skipped because they could not be read, and is empty unless
  `include_diagnostics` is true.
* Errors: `out_of_range`, `bad_cursor`, `workdir_required`, `invalid_request`
  for a bad `workdir`.
* Idempotency: read.

### `skills.get`

* Binding: any.
* Params: `{name: string, tier?: string, workdir?: string, include_raw?: bool = false}`.
* Result: `SkillDetail`. `raw` is the whole skill file, verbatim, when
  `include_raw` is true; otherwise `null`.
* Errors: `not_found`, `workdir_required`, `io_error`.
* Idempotency: read.

### `skills.read_file`

* Binding: any.
* Params: `{name: string, path: string, tier?: string, workdir?: string, generation?: string}`.
  `path` is relative to the skill directory and must be one of its
  `companion_files`.
* Result: `{path, text, bytes}`, `text` verbatim. Files over 2 MiB are not
  returned, and a file that is not UTF-8 is `invalid_request`/`binary`.
* Errors: `not_found`, `binary`, `invalid_request`, `stale`, `workdir_required`, `io_error`.
* Idempotency: read.

### `skills.create`

* Binding: any.
* Params: `{scope: "computer"|"project", name, description: string(1..2000), instruction: string(1..2 MiB),
  workdir?: string, triggers?: [string(1..200)] (≤ 32), allowed_tools?: [string(1..100)] (≤ 64)}`.
* Result: `{skill: SkillDetail, propagation}`.
* Errors: `invalid_name`, `exists`, `workdir_required`, `io_error`.
* Idempotency: a retry after success is `exists`.

### `skills.update`

* Binding: any.
* Params: `{tier, name, patch: {name?, description?, triggers?, allowed_tools?, instruction?},
  workdir?: string, generation?: string}`. `patch` must hold at least one key;
  omitted fields keep their current value. A `patch.name` renames the skill.
* Result: `{skill: SkillDetail, renamed: bool, propagation}`.
* Errors: `missing_params` (empty patch), `invalid_name`, `not_found`,
  `read_only`, `stale`, `exists` (the new name is taken), `unsafe_yaml`,
  `workdir_required`, `io_error`.
* Idempotency: a retry with the same `generation` is `stale`.

### `skills.install`

* Binding: any.
* Params: `{scope: "computer"|"project", data_b64?: string, path?: string, name?: string, workdir?: string}`.
  Give exactly one of `data_b64` (a zip, base64) and `path` (an absolute path
  to a zip on the adapter's host); both or neither is `conflicting_params`.
  `name` defaults to the zip's single top-level directory, else `invalid_name`.
* `data_b64` may be at most 39,146,840 characters, checked before decoding, so
  at most 28 MiB (29,360,128 bytes) of zip. A larger zip goes through `path`,
  up to 64 MiB. `path` must be a regular file; a symbolic link is refused.
* A zip with a lowercase `skill.md` at its root, or with a `__MACOSX/` entry, is
  `invalid_package` with a message that says so.
* Result: `{skill: SkillDetail, propagation}`.
* Errors: `conflicting_params`, `invalid_name`, `invalid_package`, `too_large`,
  `exists`, `workdir_required`, `io_error`.
* Idempotency: a retry after success is `exists`.

### `skills.remove`

* Binding: any.
* Params: `{tier, name, workdir?: string, generation?: string}`.
* Result: `{removed: true, now_effective: SkillRow|null, propagation}`.
  `now_effective` is the skill of that name that wins after the removal, when
  the removed one was shadowing another.
* Errors: `not_found`, `read_only`, `stale`, `workdir_required`, `io_error`.
* Idempotency: a retry after success is `not_found`.

## Room methods

A secure room is a session of its own, started from a coding session (its
parent). Its id is derived from the parent's id, and `sessions.list` lists it
with `kind:"room"`. All room methods need feature `bindings.room`.

Reads come from disk. Status, history, shelf, trajectory, reports, spend and
team reads are served by the adapter from the room's files and from its own
line history, never over the room connection, so they keep working after the
connection ends.

```
RoomInfo      = {session_id: string, parent_session_id: string|null, parent_name: string|null, name: string|null,
                 workdir: string|null, live: bool, locked: bool, bound: bool,
                 created_at: int|null, updated_at: int|null}
AttachResult  = {room: RoomInfo, started: bool,
                 daemon: {version: string|null, features: [string] (≤ 64), template_revision: string|null},
                 epoch: string,
                 history: {first_line_seq: int|null, last_line_seq: int|null, count: int, replayed: int},
                 tail: [RoomLine] (≤ 200), status: RoomStatus|null,
                 roster: [string] (≤ 64), roster_truncated: bool}
RoomStatus    = {session_id: string, live: bool, fresh: bool, age_ms: int|null, now_at_ms: int,
                 counts: {open: int, verified: int, returned: int, reports: int},
                 counts_basis: "first_400_items"|"disk_scan",
                 paused: [string] (≤ 64), paused_truncated: bool,
                 lanes: [LaneStatus] (≤ 128), lanes_truncated: bool,
                 events: [TimelineEvent] (≤ 60),
                 digest: string, timeline: string, trajectory_outline: string,
                 shelf_fingerprint: string, truncated: bool}
LaneStatus    = {avatar: string, display: string, alias: string, role: string, lane: "chat"|"work"|null,
                 phase: "running"|"idle"|"paused"|"submitted"|"done"|"failed"|string,
                 task: string, started_at_ms: int, last_event_at_ms: int, steps: int,
                 last_tool: string, error: string}
TimelineEvent = {at_ms: int, avatar: string, marker: string, label: string}
RoomLine      = {line_seq: int, kind: LineKind, author: string, display: string|null, alias: string|null,
                 lane: "chat"|"work"|null, audience: "user"|"team"|null,
                 text: string|null,
                 call_id: string|null, tool: string|null, args: string|null,
                 ok: bool|null, result: string|null,
                 replay: bool, origin: "room"|"adapter_echo", received_at_ms: int, truncated: bool}
LineKind      = "chief"|"lane_chat"|"visible"|"user"|"shell"|"reasoning"|"tool_call"|"tool_result"|"other"
```

* `daemon` describes the room daemon: its build `version`, the optional
  frames it understands (`features`, such as `sdk_user` and `sdk_reload`), and
  the revision of the template layer it read at start. An older room reports
  `version:null`, `features:[]` and `template_revision:null`.
* `roster` is the room's avatar ids, as `room.send{to}` and `room.control`
  accept them. It is read at attach and again after each team write.
* `history` describes the whole line history held for this epoch (`count`
  lines), not the tail. `replayed` is the number of lines the room replayed at
  attach.
* `fresh` is false until the room sends its first status after the attach;
  the replayed status can be minutes old. `age_ms` is the age of the status
  the adapter holds. `counts` cover the first 400 shelf items while live
  (`counts_basis:"first_400_items"`) and a full scan in ROOM(closed)
  (`"disk_scan"`). `digest`, `timeline` and `trajectory_outline` are text
  rendered for a terminal, each at most 256 KiB; `truncated` says one was cut.
* `shelf_fingerprint` changes whenever the shelf changes. A change tells the
  shelf, trajectory and reports screens to fetch again.
* Line kinds: `chief` is the Chief's reply; `lane_chat` an avatar's chat
  reply; `user` a line the user sent (`author:"you"`); `shell` a shell command
  and its output (`author:"shell"`); `visible` any other line shown to the user
  (author an avatar id or `room`); `reasoning`, `tool_call` and `tool_result`
  are parts of a lane's run. `text` is set for the text kinds; `call_id`,
  `tool` and `args` for `tool_call`; `ok`, `result` and `call_id` for
  `tool_result`, where `call_id` pairs it with its call. A part of an unknown
  kind has `kind:"other"` and its raw JSON in `text`. `display` and `alias` are
  the author's display name and short alias when the author is an avatar.
* Text fields over 1 MiB are cut, with `truncated:true`.
* `origin:"adapter_echo"` marks the line the adapter adds for a `room.send`
  (see `room.send`); `replay:true` marks lines the room replayed at attach.

### Takeover

A room has exactly one live subscriber. `room.open` and `room.attach` make
this adapter that subscriber and replace whoever held it before: a terminal
room view or another adapter loses its stream at once, with no frame. The
terminal shows that the room closed the connection and does not reconnect;
another adapter sees `room.closed{reason:"taken_over"}`. The room cannot tell
the new subscriber whether someone else was attached, so attaching is always a
takeover. Showing one room in two clients at once is not supported.

### Trust

A room runs shell commands for its user. `room.send` with `shell:true`, or a
raw text that starts with `!`, runs that command with the rights of the koma
user who runs the adapter, with no model and no role check. Whatever forwards
frames to the adapter (a dashboard gateway, a web server) must authenticate
its callers before it forwards anything.

### `room.list`

* Binding: any.
* Params: `{parent_session_id?: string, live?: bool, limit?: 1..500 = 100, cursor?: string}`.
* Result: `{rooms: [RoomInfo], next_cursor: string|null}`, ordered as
  `sessions.list`. `parent_session_id` keeps the room started from that
  coding session.
* Errors: `out_of_range`, `bad_cursor`, `sqlite_busy`, `io_error`.
* Idempotency: read.

### `room.open`

* Binding: UNBOUND or ROOM(closed).
* Params: `{parent_session_id: string, tail?: 0..200 = 50, wait_ms?: 0..30000 = 12000}`.
* Starts the parent's room when it is not running (in the parent's working
  directory, with the parent's settings), then attaches as `room.attach`. When
  it is running, it only attaches.
* Result: `AttachResult`, with `started:true` when this call started the room.
* Takeover: opening a room that is running takes over its live view (see
  Takeover).
* Errors, all `not_sent` unless stated: `not_found` (no such parent),
  `parent_is_room`, `room_id_in_use` (someone ran `resume` on the room's id),
  `no_main_route` (the parent has no usable Main model, checked before
  starting), `not_a_room`, `room_start_failed` (`unknown`; the room may still
  come up, so list again before retrying).
* Idempotency: idempotent in effect: a repeat attaches again. Each call starts
  a new epoch.

### `room.attach`

* Binding: UNBOUND or ROOM(closed).
* Params: `{session_id: string, tail?: 0..200 = 50, wait_ms?: 0..15000 = 2000}`.
  `wait_ms` bounds the wait for the room's first fresh status; when it passes,
  the result carries the replayed status with `fresh:false`.
* Never starts a room.
* Result: `AttachResult`, with `started:false`.
* Takeover: attaching replaces the room's live subscriber (see Takeover).
* Errors: `not_running` (the room is not running: use
  `room.open{parent_session_id}`), `not_a_room`.
* Idempotency: idempotent in effect. Each call starts a new epoch.

### `room.close`

* Binding: ROOM(live) or ROOM(closed).
* Params: `{stop?: bool = false}`.
* Leaves the room (it keeps running) and returns the adapter to UNBOUND.
  `stop:true` also stops the room daemon.
* Result: `{closed: true, stopped: bool}`. No `room.closed` event is emitted;
  the response is the signal.
* Errors: `still_running` (`unknown`) when `stop:true` and the room is still
  alive afterwards. The adapter is UNBOUND anyway.
* Idempotency: a repeat is E2, because the adapter is already UNBOUND.

### `room.send`

* Binding: ROOM(live).
* Params: `{text: string, to?: string, shell?: bool = false, attachments?: [Attachment]}`.
  `text` is nonblank and at most 2 MiB. Attachments are the `submit` shapes
  with the same bounds (at most 16, at most 32 MiB together).
* What is sent:
  * Neither `shell` nor `to`: `text` is sent unchanged, so a text that starts
    with `!` (a shell command) or `@name` (addressed to an avatar) behaves
    exactly as it does when typed in the terminal.
  * `shell:true`: the room runs `text` as a shell command. `to` and
    `attachments` must be absent (`conflicting_params`).
  * `to`: the text is addressed to that avatar. It must be in the roster
    (`unknown_avatar`); `to:"clerk"` is `clerk_not_addressable`.
* Attachments need a room whose `daemon.features` lists `sdk_user`. A room
  without it accepts text only (`acked:false`); a call with attachments is
  `peer_unsupported` before anything is sent.
* Result: `{sent: true, acked: bool, inserts: [string] (≤ 16), echo_line_seq: int}`.
  `acked` says the room confirmed the message; `inserts` is the text each
  attachment became in the message. The room does not send the sender's own
  line back, so the adapter adds it to the history as a `user` line with
  `origin:"adapter_echo"` and emits its `room.line` before the response;
  `echo_line_seq` is its number.
* Errors: `unknown_avatar`, `clerk_not_addressable`, `conflicting_params`,
  `peer_unsupported`, `attachment_refused` (the room's own message),
  `stream_error` (`unknown`: the connection failed during or after the write;
  the adapter is now ROOM(closed)).
* Idempotency: not idempotent. Never retry automatically.

### `room.interrupt`

* Binding: ROOM(live).
* Params: `{}`.
* Stops every chat lane; work lanes continue.
* Result: `{sent: true}`.
* Errors: `stream_error` (`unknown`).
* Idempotency: harmless to repeat.

### `room.control`

* Binding: ROOM(live).
* Params: `{action: "pause"|"resume"|"stop"|"redirect", avatar: string, note?: string(1..8192)}`.
  `note` is required for `redirect` and allowed only there.
* Acts on the avatar's work lane.
* Result: `{sent: true}`. A control that changes nothing comes back as a
  `room.line` with `kind:"visible"` and `author:"room"`.
* Errors: `note_required`, `conflicting_params`, `unknown_avatar`,
  `no_work_lane` (`chief`), `stream_error` (`unknown`).
* Idempotency: not idempotent.

### `room.status`

* Binding: ROOM(live) or ROOM(closed).
* Params: `{}`.
* Result: `RoomStatus`. In ROOM(live) it is the latest status the room sent.
  In ROOM(closed) it is `live:false`, with counts from a full disk scan and
  empty `lanes` and `events`.
* Do not poll: `room.status` events carry the same object.
* Idempotency: read.

### `room.history`

* Binding: ROOM(live) or ROOM(closed).
* Params: `{epoch: string, after_line_seq?: int, before_line_seq?: int, limit?: 1..200 = 100, kinds?: [LineKind], author?: string}`.
  The bounds are exclusive `line_seq` values, and at most one may be given
  (`conflicting_params`). With neither, the newest `limit` lines are returned.
  `kinds` and `author` filter the page.
* Result: `{epoch: string, first_line_seq: int|null, last_line_seq: int|null, lines: [RoomLine], has_more_before: bool, has_more_after: bool}`.
  `lines` ascend by `line_seq`. `first_line_seq` and `last_line_seq` describe
  the whole history, not the page. A page also stops at 8 MiB.
* The history holds what the room replayed at attach (at most 2000 lines)
  plus every line that arrived since. Older lines are not available. The
  history keeps the newest 8000 lines or 64 MiB, and `first_line_seq`
  advances as lines are dropped.
* Errors: `epoch_changed` (the epoch is not this binding's; discard your
  buffer and start from the current attach), `conflicting_params`,
  `out_of_range`.
* Idempotency: read.

### `room.shelf.list`

```
ShelfRow  = {key: string, revision: int, updated_at: int, kind: string, title: string (≤ 101 chars), status: string}
ShelfItem = ShelfRow + {content: string (≤ 16 MiB), json: object|null, truncated: bool}
```

* Binding: ROOM(live) or ROOM(closed).
* Params: `{prefix?: string(0..200), limit?: 1..500 = 200, cursor?: string}`.
* Result: `{rows: [ShelfRow], total: int, next_cursor: string|null, fingerprint: string}`,
  ordered by key. `total` counts every row under `prefix`. `fingerprint` is
  the shelf fingerprint the page was read at.
* `kind` comes from the item's record; `title` and `status` are the first
  non-empty listed fields, cut at 100 characters plus `…`.
* Errors: `out_of_range`, `bad_cursor`, `room_unreadable`, `io_error`.
* Idempotency: read.

### `room.item`

* Binding: ROOM(live) or ROOM(closed).
* Params: `{key: string(1..512)}`. A key that breaks the shelf's key rules is
  `invalid_request`.
* Result: `ShelfItem`. `json` is the content parsed as a JSON object, or
  `null` when it is not one.
* Errors: `not_found`, `room_unreadable`, `io_error`.
* Idempotency: read.

### `room.trajectory`

```
Goal     = {goal_key: string, counts: {open: int, verified: int, returned: int, reports: int},
            columns: int, rows: int,
            nodes: [GoalNode] (≤ 2000), nodes_truncated: bool,
            edges: [{from: string, to: string, rel: string|null}] (≤ 8000), edges_truncated: bool}
GoalNode = {id: string, label: string, kind: string, status: string|null, severity: string|null,
            note: string|null, resource_key: string|null, revision: int|null, rank: int, row: int}
```

* Binding: ROOM(live) or ROOM(closed).
* Params: `{goal?: string}`, with or without the `trajectory/` prefix.
* Result: `{goals: [Goal] (≤ 200), goals_truncated: bool}`. An empty shelf
  gives `goals:[]`, and so does a `goal` the shelf does not hold.
* `rank` and `row` place a node in the goal's grid of `columns` by `rows`.
* Errors: `room_unreadable`, `io_error`.
* Idempotency: read.

### `room.reports`

```
Report = {key: string, revision: int, updated_at: int, title: string, audience: string,
          severity_summary: string, go_no_go: string,
          finding_keys: [string] (≤ 1000), finding_keys_truncated: bool, markdown: string|null}
```

* Binding: ROOM(live) or ROOM(closed).
* Params: `{include_markdown?: bool = false, limit?: 1..100 = 50, cursor?: string}`.
* Result: `{reports: [Report], next_cursor: string|null}`, newest
  `updated_at` first. `markdown` is `null` unless `include_markdown`.
  `severity_summary` and `go_no_go` are free text from the report, not enums.
* Errors: `out_of_range`, `bad_cursor`, `room_unreadable`, `io_error`.
* Idempotency: read.

### `room.spend`

```
GoalSpend  = {goal_key: string, total: SpendEntry,
              by_avatar: [{avatar: string} + SpendEntry] (≤ 64), by_avatar_truncated: bool}
SpendEntry = {calls: int, prompt_tokens: int, completion_tokens: int, cost: num}
```

* Binding: ROOM(live) or ROOM(closed).
* Params: `{goal?: string}`, as `room.trajectory`.
* Result: `{goals: [GoalSpend] (≤ 200), goals_truncated: bool,
  ledger: {ledger: "ok"|"missing", totals: UsageTotals, by_avatar: [{avatar: string, totals: UsageTotals}] (≤ 64), by_avatar_truncated: bool}}`.
* `goals` is the spend the room records per goal on its shelf. `ledger` is
  the room's rows in the usage ledger (see `usage.query`), with the Chief
  under `chief` and each lane under its avatar id.
* Errors: `query_timeout`, `room_unreadable`, `io_error`.
* Idempotency: read.

## Team setup

A room's team (its avatars, their roles and the models their lanes run) is
built from three layers, lowest first:

* base: the team that ships with koma;
* template: a computer-wide layer in the koma data directory (`room/`), absent
  by default, read by every room when it starts (feature `room.template`);
* session: the room's own layer, inside the room's session directory.

Template and session layers use the same files: `avatars.toml`, plus role
files a person may write by hand. A higher layer may only narrow a role (fewer
shelf areas, skills or tools), never widen it; a role file that widens is
dropped whole. Avatars merge field by field: an avatar in a higher layer
changes only the fields it sets.

Roles are read-only through the SDK. `Team.roles` shows each role's effective
values and the layer that defines it; no method writes or removes a role
file. A hand-written role file keeps working as it does today.

The template is frozen per room: a room reads it once, at start, and reports
its revision in `AttachResult.daemon.template_revision`. A template write
never reaches a running room.

```
Team         = {target: "template"|"room", revision: string, layer_present: bool,
                template_revision: string, template_pending_restart: bool,
                avatars: [AvatarView] (≤ 128), avatars_truncated: bool,
                roles: [RoleView] (≤ 64), roles_truncated: bool,
                choices: [{id: string, name: string, model_id: string,
                           provider_name: string|null, provider_uuid: string|null}] (≤ 2000),
                choices_truncated: bool, choices_source: "room_daemon"|"config",
                warnings: [{layer: "template"|"session", path: string, message: string}] (≤ 100),
                warnings_truncated: bool}
Origin       = "base"|"template"|"session"|"role"|"default"
Field<T>     = {value: T|null, origin: Origin}
AvatarView   = {id: string, role: string, defined_in: "base"|"template"|"session", removable: bool,
                display: Field<string>, display_effective: string,
                alias: Field<string>, alias_effective: string,
                lanes: [LaneView] (≤ 2)}
LaneView     = {lane: "chat"|"work", provider_uuid: Field<string>, model: Field<string>,
                effort: Field<string>, model_label: string|null, resolves: bool}
RoleView     = {id: string, display: string, shelf_allow: [string], skills_allow: [string]|null,
                chat: RoleLaneView, work: RoleLaneView, defined_in: "base"|"template"|"session"}
RoleLaneView = {provider_uuid: string|null, model: string|null, effort: string|null}
```

* `revision` is the revision of the target layer: the first 16 lowercase hex
  characters of SHA-256 over its files, or `"absent"` when the layer has no
  files (`layer_present:false`). Pass it as `if_revision`.
* `template_revision` is the revision of the template layer as it is now. For
  `target:"room"`, `template_pending_restart:true` says it differs from the
  one the running room read at start.
* `choices` are the model choices a lane can run. For a running room they are
  the room's own list (`choices_source:"room_daemon"`), fixed when the room
  started, so a model registered later needs a room restart. Otherwise they
  come from the config (`"config"`).
* `origin` names the layer a value comes from. `"role"` means the avatar
  inherits its role's lane default; `"default"` means it is set nowhere, so
  the lane uses the session's default model.
* `model` is a choice id, or a bare model id for a choice without a provider.
  `provider_uuid` is the stored provider; a hand-written file may hold a
  provider name there, shown as it is. `resolves:false` means the lane's model
  is not among the choices, so the lane falls back to the Main model.
* A lane's provider, model and effort are one unit: a layer that sets `model`
  also decides `provider_uuid` and `effort` (absent means none), so a lane
  never pairs one layer's provider with another layer's model. A layer that
  sets only `effort` changes only the effort.
* The Chief has a chat lane only and the Clerk a work lane only.
* `removable` is true when the target layer holds an entry for the avatar.
* `RoleView` holds plain effective values; `skills_allow:null` means
  unrestricted. `defined_in` is `base` unless a template or session layer holds
  an accepted role file for it.
* `warnings` are the loader's warnings for the layers as they are. They never
  block a write.

### Targets

`target` is `"template"` or `"room"`.

* Writes require it: a missing `target` is `target_required`.
* `room.team.get` defaults to `"room"` in a room binding, else `"template"`.
* `target:"room"` outside a room binding is E2. `target:"template"` works in
  any binding.

### Reset idiom

* An omitted field keeps its value.
* `null` clears the field in the target layer, so it inherits from the layer
  below.
* `room.avatars.remove` removes the target layer's whole entry for the avatar.

### Writes and `applies`

A team write locks the target layer, compares `if_revision`, checks the
candidate layer with the room's own loader (a new warning on a touched entry
is `layer_rejected`, and nothing is written), writes the file through a
temporary file and rename, and reads it back. Then it applies the change:

| target and state | `applies` | `propagation.room` |
|---|---|---|
| `template`, any binding | `next_start`: rooms read the template when they start | `not_live` |
| `room`, ROOM(live), room lists `sdk_reload` | `live`: the room reloads its team and lanes use it from their next turn; a running turn keeps the team it started with | `reloaded` |
| `room`, ROOM(live), room without `sdk_reload` | `next_reload`: the file is in place and the room takes it at its next team reload or restart | `not_supported` |
| `room`, ROOM(closed), room still running | `next_reload` | `not_live` |
| `room`, room not running | `next_start` | `not_live` |

When a running room refuses the reloaded team (a new warning on a touched
entry, or a wrong effective value), the adapter restores the previous file,
reloads again and verifies it: the result is `rejected_by_room`, `not_sent`.
If the restore cannot be verified, the result is `restore_failed`,
`unknown`, `written:true`. If the room connection is lost after the file is
committed, the call still returns its result, with `applies:"next_reload"`
and `propagation.room:"failed"`, and `room.closed` follows.

Result of every team write: `{team: Team, applies: "live"|"next_reload"|"next_start", propagation: {room: string}}`.

### `room.team.get`

* Binding: any for `target:"template"`; a room binding for `target:"room"`.
* Params: `{target?: "template"|"room"}`.
* Result: `Team`.
* Errors: `room_unreadable`, `io_error`.
* Idempotency: read.

### `room.models.set`

* Params: `{target, avatar: string, lane: "chat"|"work", model?: string|null, effort?: string(1..64)|null, if_revision?: string}`.
  At least one of `model` and `effort` (`missing_params`).
* `model` is a choice id from `Team.choices` (`unknown_model`); `null` clears
  the lane unit in the target layer. `effort` has no whitespace or control
  characters and is not checked against a model's offered options.
* Errors: `target_required`, `missing_params`, `unknown_avatar`,
  `unknown_model`, `no_such_lane` (`(chief, work)` or `(clerk, chat)`),
  `stale`, `layer_rejected`, `rejected_by_room`, `lock_timeout`,
  `restore_failed`.
* Idempotency: idempotent.

### `room.avatars.upsert`

* Params: `{target, id: string, role?: string, display?: string(1..64)|null, alias?: string(1..3)|null, if_revision?: string}`.
* `id` matches `^[a-z0-9][a-z0-9_-]{0,47}$`. A new avatar needs `role`, an
  existing role other than `chief` and `clerk`. An existing avatar's role
  cannot change. `alias` is ASCII letters and digits, stored uppercase. The
  team holds at most 64 avatars (`out_of_range`).
* Errors: `target_required`, `unknown_role`, `reserved_id`,
  `role_immutable`, `out_of_range`, `stale`, `layer_rejected`,
  `rejected_by_room`, `lock_timeout`, `restore_failed`.
* Idempotency: idempotent.

### `room.avatars.remove`

* Params: `{target, id: string, if_revision?: string}`.
* Removes the target layer's entry. The result adds
  `reverted_to: "base"|"template"|null`: the layer the avatar now comes from,
  or `null` when it is gone. Base avatars cannot be removed, only reverted.
* Errors: `target_required`, `unknown_avatar` (no entry in this layer),
  `lane_running` (in a live room, the avatar's lane is running), `stale`,
  `layer_rejected`, `rejected_by_room`, `lock_timeout`, `restore_failed`.
* Idempotency: a retry after success is `unknown_avatar`.

## Room events

Room events are emitted only in a room binding, and only when hello `events`
lists them (feature `events.room`). A client that never binds a room sees
none of them.

| event | body | when |
|---|---|---|
| `room.line` | `{epoch, line: RoomLine}` | every history line and every `room.send` echo |
| `room.delta` | `{epoch, author, lane: "chat"\|"work", kind: "text"\|"reasoning", text}` | streaming text of a lane. Deltas are never history; the complete text follows as a `room.line` |
| `room.status` | `{epoch, status: RoomStatus}` | when the attach completes, and whenever the status content changes (`age_ms` and `now_at_ms` alone do not count). Latest wins |
| `room.notice` | `{epoch, code: "room"\|"stalled"\|"recovered"\|"layer_warnings", text}` | `room`: a message from the room; `stalled`: no status for 10 s on an open connection; `recovered`: status resumed; `layer_warnings`: a team reload reported warnings |
| `room.closed` | `{epoch, session_id, reason: "taken_over"\|"daemon_exit"\|"stream_error"}` | the room connection ended; the adapter is now ROOM(closed) |
| `room.gap` | `{epoch, after_line_seq: int, dropped_lines: int, dropped_deltas: int}` | the adapter dropped queued room events (see Limits). The lines are still in the history |

No room event carries `state` or `revision`. `code`, `reason` and `kind`
values are open strings.

### Epochs and line numbers

* Every successful `room.open` or `room.attach` starts a new epoch, an opaque
  string in `AttachResult.epoch` and in every room event. A new epoch has a
  fresh history: the room replays its recent lines again, so a client replaces
  its line buffer and never merges two epochs.
* `line_seq` numbers the lines of one epoch, starting at 1. A line is
  identified by `(epoch, line_seq)`. Every room field that numbers lines uses
  `line_seq`: `RoomLine.line_seq`, `history.first_line_seq` and
  `last_line_seq`, the `room.history` bounds, `room.send` `echo_line_seq` and
  `room.gap.after_line_seq`.
* The envelope `seq` keeps its meaning: one counter for every event this
  adapter writes. `seq` and `line_seq` are separate counters; never compare
  them.
* `room.gap.after_line_seq` is the `line_seq` of the last line event
  delivered before the hole, or 0 when none was.

### Ordering

* `room.line` events of one epoch arrive in `line_seq` order, without
  duplicates; `room.gap` marks the holes.
* The `room.delta` events of a lane come before the `room.line` that
  completes them. Consecutive deltas of one `(author, lane, kind)` may be
  merged while queued, or dropped (counted in `room.gap.dropped_deltas`).
* No event of a new epoch comes before the `room.open` or `room.attach`
  response, and no event of an older epoch comes after it.
* The `room.send` echo line comes before the `room.send` response.
* A stall never closes the binding: it gives `room.notice{code:"stalled"}`.

### What a client must do

* After an attach: render `result.tail`, and page older lines with
  `room.history{epoch, before_line_seq}`. Apply `room.line` events whose
  `line.line_seq` is greater than `result.history.last_line_seq`.
* Ignore any room event whose `epoch` is not the current binding's.
* On `room.gap`, or on a client-side event overflow: call
  `room.history{epoch, after_line_seq: <last line_seq seen>}` until
  `has_more_after` is false, then call `room.status`. Never call `state`.
* On `room.closed{reason:"taken_over"}`: show that the room is open in
  another client and offer an explicit take-back, which is `room.attach`.
  Never re-attach automatically: two clients that both do would take the room
  from each other forever.
* On `room.closed{reason:"daemon_exit"}`: offer a restart, which is
  `room.open{parent_session_id}`.
* In ROOM(closed): reads, history and file-only team writes keep working.
* On `room.notice{code:"stalled"}`: mark the live view stale. Do not
  reconnect.

## Limits

`hello.limits` advertises bounds by key. Keys are added over time; treat an
absent key as unknown, and an absent `limits` as the v1 bounds.

| key | meaning |
|---|---|
| `frame_bytes` | maximum frame size in bytes (40 MiB) |
| `request_ids` | requests accepted per adapter process (see Request ids) |
| `text_bytes` | maximum prompt text in UTF-8 bytes |
| `attachments.count`, `attachments.pasted_bytes`, `attachments.total_bytes` | attachment bounds (see DTOs and normalization) |
| `list.sessions`, `list.skills`, `list.usage_groups`, `list.usage_buckets` | page size or cap of that list |
| `skills.inline_zip_bytes` | decoded bytes accepted in `skills.install` `data_b64` (28 MiB) |
| `skills.zip_bytes` | size of a zip read with `skills.install` `path` (64 MiB) |
| `skills.entry_bytes`, `skills.companion_bytes` | size of one zip entry, and of a companion file read by `skills.read_file` |
| `skills.files`, `skills.depth`, `skills.name_bytes` | entries per skill, directory depth, bytes in a skill name |
| `config.lock_wait_ms` | how long a config write waits for the SDK config lock |
| `time.ipc_ms` | how long the adapter waits for one daemon reply |
| `time.usage_query_ms` | the deadline of one `usage.query` |
| `time.session_stop_ms` | the bound of one whole `sessions.stop` call |
| `list.rooms`, `list.shelf_rows`, `list.reports`, `list.history_lines` | page size of `room.list`, `room.shelf.list`, `room.reports` and `room.history` |
| `list.trajectory_goals`, `list.trajectory_nodes`, `list.trajectory_edges` | caps of `room.trajectory` |
| `room.ring_lines`, `room.ring_bytes` | the line history kept per epoch (8000 lines, 64 MiB) |
| `room.page_bytes` | the most serialized lines in one `room.history` page or attach `tail` (8 MiB) |
| `room.event_text_bytes`, `room.status_text_bytes` | where a line text and a status text are cut (1 MiB, 256 KiB) |
| `room.item_bytes` | where `room.item` content is cut (16 MiB) |
| `room.tail_lines` | the largest attach `tail` (200) |
| `time.room_start_ms` | the longest `room.open` waits for a room to start |

Room event queue. The adapter queues room events while stdout is slow, up to
1024 events or 16 MiB. Deltas of one `(author, lane, kind)` are merged while
queued. When the queue is full, queued `room.line` and `room.delta` events are
dropped and one `room.gap` is queued in their place; the lines stay in the
history, so `room.history` recovers them. `room.status` is latest-wins and is
never dropped. Every room event is at most 4 MiB, because its texts are cut.
A client library's own event queue can overflow as well; a room client
recovers from that with `room.history`, exactly as from `room.gap`, and never
with `state`.

v1 responses keep the v1 rule: a state too large for a frame becomes
`unavailable`/`not_sent`. A new read whose result would exceed the frame fails
with `reason:"frame_limit"`. New mutation results are bounded by construction
(`ConfigView` by its caps, a skill result by the 2 MiB instruction and the
1000-path companion list), so a committed mutation never loses its result.

Every new list is paged or capped. `sessions.list`, `skills.list`,
`room.list`, `room.shelf.list` and `room.reports` are paged by `cursor`, and
`room.history` by `line_seq`; `RoomStatus.events` is the newest 60 and
`AttachResult.tail` the newest `tail` lines; `usage.query` groups are capped by `limit` (`groups_truncated`) and
its buckets are capped by refusal (`range_too_large`); the other lists state
their cap and a `<name>_truncated` flag.

## Request ids

An adapter accepts `limits.request_ids` (65,536) requests per process. The
next request is fatal: the adapter exits, as in v1. Clients must recycle the
adapter before the limit: an UNBOUND adapter holds no state, so a client
respawns it once 60,000 requests have been sent; a client driving a coding
session exposes the number of requests remaining, and the caller reconnects
with `resume` from a fresh adapter. A client holding a room recycles with
`room.attach` from a fresh adapter; the old adapter then sees
`room.closed{reason:"taken_over"}` and is closed by the client. For scale, polling one method every 5 s
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
* The SDK lock files and the room start log are created readable by their
  owner only.
  A symlink or any other non-file at a lock path is refused (`io_error`), so
  a lock never creates or locks a file elsewhere.

Room text is scrubbed. Every string of every room result and room event
(`room.status` with its texts, lanes and timeline, shelf rows and items
including every string inside `json`, reports including `markdown`,
trajectory nodes, spend, history and attach results, `room.line`,
`room.delta` and `room.notice`), every `inserts` text, and every room error
text and `detail` has two things replaced by `[redacted]`:

* every secret value koma itself has configured, of 8 or more characters:
  each provider API key, each OAuth token and each sealed environment value.
  The set is refreshed whenever the config file changes, so a key added by
  another process after the attach is covered;
* URL userinfo (`scheme://user:pass@`).

Nothing else is scrubbed. There is no pattern matching on text that looks
like a secret: a room's findings, evidence and reports may quote a credential
found in the system under test, and rewriting it would corrupt that evidence.
Treat room text as sensitive. Scrubbed room results are for display; never
write them back. Coding `state` is not touched.

Skill source is returned verbatim. `skills.get` (`instruction`, `raw`) and
`skills.read_file` (`text`) return the user's own files exactly, with no
redaction, because a client edits them and sends them back through
`skills.update`. Do not log or forward them as if they were scrubbed.

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

A room's session layer can also be written by the room's current subscriber
when it changes a lane model from the terminal. Team writes narrow this with
`if_revision` and the read-back after each write. The template layer is a
plain directory people may edit by hand; an edit made between the adapter's
read and its write is lost unless the caller passes `if_revision`.

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
