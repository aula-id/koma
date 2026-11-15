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
unsupported protocol versions before sending requests.

Exactly one session is bound per adapter process. `create` or `resume` binds it;
repeating either after binding returns `invalid_state`. All other methods require
a bound session. `detach` detaches only this client, does NOT stop the session,
returns `{detached:true}`, flushes stdout, then exits 0. stdin EOF also detaches.
Unexpected EOF/process failure is a transport failure, not a successful detach.

## Correlation and delivery

Request: `{id,method,params}`; id is a nonempty opaque string, unique for the
process lifetime. Response: exactly `{type:"response",id,result}` OR `{type:"response",id,error}`. Results are
method-specific (see schema). Errors are `{code,message,delivery}` where delivery
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
* `state`: `{}` -> `{state}`.
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
