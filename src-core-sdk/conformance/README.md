# Shared subprocess conformance peer

Run `python3 src-core-sdk/conformance/fake_adapter.py --scenario normal`.
Python 3.8+, standard library only; executable can also be launched directly.
This is a deterministic transport test peer, NOT a daemon emulator and NOT a
reference security validator. It trusts well-formed method parameters; production
adapters must implement the full validation and byte budgets in CONTRACT.md.

Scenarios (one per process):

| scenario | behavior / client assertion |
|---|---|
| normal | all methods; running turns stay running until targeted interrupt; events precede mutation replies |
| reject | turn mutations reject with invalid_input; no automatic retries |
| partial | replacement rejects after old turn was interrupted; preserve interrupted_turn_id |
| unknown | valid turn mutation returns unavailable/delivery unknown; do not retry |
| eof | exit 3 on submit/replacement without reply; report delivery unknown |
| malformed | malformed JSON after first request; fail all pending calls |
| wrong_id | extra unknown response before correct response; do not steal correlation |
| out_of_order | hold first response, respond to second then first; send two concurrent requests (including create+state) |
| event_flood | emit 300 state events before reply; bounded stalled event consumers must fail, not hang/unboundedly grow |
| flood_output | first submit overflows; restart reconciles output and subsequent turns emit valid appends |
| flood_finish | submit emits 300 events before its reply; next state read completes the turn; polling and restart recovery remain usable |
| approval | submit parks at tool approval gate; explicitly opted-in approval uses actual pending call ID |
| bad_version | hello protocol 2; client must refuse before requests |
| stderr | >64 KiB diagnostic stream; drain independently and retain bounded tail |

| output | emit full running state followed by a revisioned UTF-8 suffix append |
| hidden | explicit state hides old turn behind a newer running record |
| fragmented | write frames one byte at a time, including split multibyte UTF-8 |
| timeout | delay submit response by two seconds; client cancellation/timeout is local |
| missing_cap | omit detach by name from hello `capabilities` and `methods`; compatibility probe must refuse |
| output_gap / bad_offset | force sequence or append-offset mismatch; clients replace state by readback |
| finish | complete the observed turn on the next state read; runnable-example smoke tests |
| bad_result | return an invalid typed mutation result; report delivery unknown, not success |
| legacy_hello | an older adapter: hello has only `capabilities` (no `methods`, `features`, `events`, `limits`); `config.*` and `sessions.info` are `unsupported_method`; results carry no `session_info`; binding errors carry no `reason`. Clients gate on `capabilities` |
| config | seeded catalogue for every `config.*` branch (below); `create`/`resume` pin the session Main to `model-main` |
| wrong_binding | normal behaviour; add `--binding room_closed` to start in ROOM(closed) (room A below, epoch 1, replayed ring) and reach E4 |
| sessions | the seeded registry (below) for `sessions.*`, `create{name}` and `room.list` |
| usage | the seeded registry and ledger (below) for `usage.query` |
| skills | the seeded skill roots (below) for `skills.*` |
| room | `room.open`/`room.attach` of room A or B, then lines, deltas, status and notices after the attach response, send with echo, history paging, disk reads and team reads and writes |
| room_gap | like `room`, but the first attach is followed by one line and a `room.gap` hole |
| room_takeover | like `room`, but the first attach is followed by `room.closed{taken_over}` |

Every scenario except `legacy_hello` advertises all 49 methods (the ten v1 methods and 39 new ones), the seven features,
the nine events and the full limits block in hello, implements every family
in memory, and answers calls in the wrong binding with E1-E4 (`reason`,
`binding`, `allowed`). `state`, `create` and `resume` results carry `session_info`
beside `state`; state events never do. Unbound `detach` keeps its v1 error
(`invalid_state`, `no session bound`, no added keys) and exits. The families work in every scenario, but the seeded data (registry,
ledger, skills, rooms) exists only in the scenarios named below; elsewhere the
registry holds only the sessions `create` adds, the ledger is `missing`, the
skill roots are empty and `room.open` answers `not_found`.

The fake keeps a fixed clock: `NOW` is 1767225600 (2026-01-01 00:00 UTC). Every
default window, timestamp and age is relative to it, so runs repeat exactly.
Reads, writes and pages are consistent within one process; nothing survives it.

Config catalogue. `config.json` is in memory: it starts `file:"missing"` with
revision `absent`, and `configure` writes into it (provider `prov-<name>`, model
`model-<role>`). The revision is the first 16 hex chars of sha256 over the
catalogue, so it changes on every committed write. Secret values are stored but
never projected; tests assert they never appear on stdout. The `config` scenario
starts with:

| entry | purpose |
|---|---|
| provider `prov-local` | key set; endpoint with userinfo, query and fragment (`endpoint_redacted:true`); owns `model-main` and `model-planner` |
| provider `prov-ext` | extension-managed, no key: delete gives `managed` |
| oauth `oauth-1` | signed in, owns `model-oauth` (`awareness`) |
| `model-main` | global Main |
| `model-planner` | `planner` plus an undisclosed role (`other_roles:true`) |
| `model-orphan` | provider missing (`provider_kind:"missing"`) |
| `model-unverified` | its delete commits and answers `unavailable`/`unverified`, `delivery:"unknown"`, `written:true`; unbound the fake stays up, coding it exits after the reply |

Rules the fake enforces, in the real adapter's order: coding-only methods check
the binding first; `config.get` validates `view`, which decides the binding,
then checks it; `config.roles.unassign` validates `role`, `scope` and
`if_revision`, then for session scope refuses any role but `main`
(`out_of_range`), then `if_revision` (`conflicting_params`), and only then checks
the binding; mutations then check `stale`
(`if_revision` against the current revision), then `not_found`/`managed`;
`dry_run` returns the computed effect with the current view and empty
`propagation`; `config.models.delete` with any `scope` other than `"global"` is
`out_of_range`; session-scope `config.roles.unassign` clears the Main pin (effort
resets to `default`). `config.tool_policy.set` validates `deny` (`missing_params`, then
`out_of_range` per entry), then `if_revision`, then `stale`; an equal list is
`changed:false` with empty `propagation`. An unassigned `compactor`, `awareness` or `planner` reports
`origin:"inherit_main"` with Main's route and `model_uuid:null`; an unassigned
`safeguard` never inherits Main and reports `origin:"default"`,
`reason:"unassigned"`. An adapter-side delete always reports `rebind:"done"`. `config.effort.get{wait_ms:0}`
answers `loading` until options for that Main were fetched once with `wait_ms > 0`
or by `config.effort.set`; options are `default`, `low`, `medium`, `high`, or
`unsupported` with only `default` when no Main exists. `config.limits.set`
updates `session_info` (new `observed_at_ms`) without a state event or a revision
bump. CODING mutation results carry `session_info`.

Sessions (scenarios `sessions`, `usage`, `room`, `room_gap`, `room_takeover`, and
`wrong_binding --binding room_closed`). `sessions.list` orders by `updated_at`
descending, then id; the seeded rows, newest first:

| id | what it is |
|---|---|
| room A (`uuid5(OID, "secure-room:sess-coding-a")` hex) | live room named `Alpha room`, locked, parent `sess-coding-a`, modern features |
| `sess-coding-a` | `Alpha`, live and working, locked, workdir `/fake/workspace` |
| `sess-stuck` | live; `sessions.stop` answers `unavailable`/`still_running`, `unknown` |
| room B (`secure-room:sess-coding-b`) | `Beta room`, not live, an older room: no `sdk_user`/`sdk_reload`, no `template_revision` |
| `sess-coding-b` | `Beta`, not live; deleting it returns `orphaned_room_session_id` = room B |
| `sess-coding-c` | not live, name equals its id; `room.open` answers `no_main_route` |
| `sess-locked` | not live, lock owner alive: `sessions.delete` gives `session_locked` |
| `sess-partial` | first `sessions.delete` gives `unavailable`/`partial_delete` (`unknown`, `written`); the retry succeeds |
| `sess-busy` | `sessions.rename` gives `unavailable`/`sqlite_busy` |
| `sess-nostart` | `room.open` gives `unavailable`/`room_start_failed`, `unknown`, `detail.log_tail` |
| `sess-clash`, `uuid5(...sess-clash)` | the second row is a coding session on the room id of the first: `room.open{parent_session_id:"sess-clash"}` gives `room_id_in_use` |
| `sess-live-nr` | live with no registry row: `name:null, workdir:null, created_at:null, updated_at:null` |

`create` adds (or revives) a row for its session id (`session-1` by default) that
is live, locked and `bound`. `create{name}` trims and validates the name (length
1..200 `out_of_range`, control characters `invalid_name`); the name `fail-rename`
simulates a failed rename: the session is created, `result.warnings` holds
`rename_failed` and `session_info.name` is the session id. `resume` of a known
row keeps its name and workdir. `sessions.rename` of another live coding session
is `invalid_state`/`live`; the bound coding session, rooms and sessions that are
not live are renamed; `sessions.info` follows. `sessions.delete` checks, in order,
`not_found`, `bound_self`, `live`, `session_locked`. `sessions.stop` of an id
that is not live (or unknown) answers `was_live:false`. These calls never end the
process. Bound sessions and rooms report `bound:true`.

Usage (scenarios `usage` and the room ones). The ledger has twelve rows from
`NOW - 40 days` on: eleven inside the default 30 days, for sessions
`sess-coding-a`, `sess-coding-b`, `sess-deleted` (no registry row: `deleted:true`,
`name:null`) and room A (roles `main`, `sub:scout`, `sub:analyst`); one older
`main` row (40 days back) only a wider window sees. Day, hour and week buckets
start on local boundaries shifted by `tz_offset_minutes` (default 0); weeks start
on Monday. Empty buckets are included; more than 1000 buckets or 400 days is
`range_too_large`; `session_id:"sess-slow"` gives `unavailable`/`query_timeout`.
Groups sort by cost descending, then key; `limit` cuts them and sets
`groups_truncated`. Without a ledger (other scenarios) the result is
`ledger:"missing"` with zero totals.

Skills (scenario `skills`). Roots: global holds `code-review` (companions
`notes/checklist.md` and a binary `bin/tool.bin`), `deploy-notes` and `odd-yaml`
(frontmatter that cannot be saved: `skills.update` gives `unsafe_yaml`). The only
project root is `/fake/workspace` (also `/fake/other` and `/fake/empty` exist and
are empty): tier `claude` `claude-helper`, tier `project-agent` `legacy-skill`,
tier `project-agents` `code-review` (shadows the global one) and `proj-helper`,
tier `extra-root:0` `extra-skill`. `claude` and extra roots are read only. The
workdir comes from `workdir`, else the bound coding cwd, else the bound room's
workdir; project tiers without one are `workdir_required`, a directory the fake
does not know is `not_found`. `skills.list` diagnostics (`include_diagnostics`)
hold one entry. `generation` is a hash of the skill, so any change shows. Installs
parse real zips (base64 or an absolute file path); `propagation.skills` is
`next_rebuild` unbound and coding, `room_restart` in a room.

Room (scenarios `room`, `room_gap`, `room_takeover`). `room.open{parent_session_id}`
and `room.attach{session_id}` bind a live room as ROOM(live). Room A replays ten
lines (user, chief, visible, lane_chat, reasoning, tool_call and tool_result with
one `call_id`, shell, visible, chief); room B replays three. The epoch of the n-th
attach in a process is `uuid5(OID, "epoch-n")`, so a test can name it before the
response arrives. Opening a room that is not live "spawns" it (`started:true`,
for example room B or any new parent). The roster is the effective avatars of the
room target (`chief`, `clerk`, `scout`, `analyst`, `recon` from the template
layer, `probe` from room A's session layer). Events follow the attach response, in
this order, each with the room's `epoch` and the shared envelope `seq`:

| scenario | events after each attach |
|---|---|
| `room` | `room.status`, two `room.delta` (scout, chat, text), `room.line` (live line, `line_seq` 11, `replay:false`), `room.notice` `stalled`, `room.notice` `recovered` |
| `room_gap` (first attach only) | `room.status`, `room.line` (`line_seq` 11), then `room.gap{after_line_seq:11, dropped_lines:3, dropped_deltas:2}`; lines 12 to 14 are in the ring and not delivered as events: `room.history{after_line_seq:11}` returns them |
| `room_takeover` (first attach only) | `room.status`, `room.closed{taken_over}`; the binding is ROOM(closed): reads, history and file-only team writes work, socket writes are E4, `room.attach` binds again with a new epoch and no further takeover |

`room.send` emits the adapter echo (`room.line`, `origin:"adapter_echo"`, kind
`user`, or `shell` for `shell:true`; text is the wire text: `!cmd` or `@avatar text`)
before the response. Rooms with `sdk_user` (room A) answer `acked:true` with one
`[Image #n]` or `[Pasted text #n]` insert per attachment; room B answers
`acked:false`, no inserts, and attachments are `peer_unsupported`. An `image_file`
path containing `refused` gives `attachment_refused`. The text `fake:stream_error`
fails with `unavailable`/`stream_error` (`unknown`), moves the binding to
ROOM(closed) and emits `room.closed{stream_error}` after the response; the process
stays. `room.control` `pause` and `resume` change the status (`paused`, lane
phase) and emit a `room.status` event after the response. `room.close` unbinds
(`stop:true` also marks the room not live). Status in ROOM(closed) is
`live:false, fresh:false, counts_basis:"disk_scan"`, no lanes or events.

`room.history` pages the ring by `line_seq` with the `has_more_*` rules of CONTRACT.md
and `epoch_changed` for another epoch. Disk reads serve a fixed shelf (three
findings, two reports, notes, one goal `trajectory/web-recon`) in every binding
that holds a room. Team layers: base (chief, clerk, scout, analyst), template
(`recon`), session (room A: `probe`); template writes work in any binding and
apply `next_start`, room writes apply `live` (room A, `sdk_reload`),
`next_reload` (room B, or ROOM(closed)). Hooks: effort `room-rejects` on a live
room with `sdk_reload` gives `rejected_by_room`; two avatars with the same alias
give `layer_rejected`; removing `probe` from the live room A gives `lane_running`.
Choices: `model-main`, `model-fast`, `model-deep` (provider `prov-local`) and
`model-bare` (no provider, stored as its `model_id` `bare/model`).

Not simulated: ring eviction at 8000 lines, `room.closed{daemon_exit}`, a team
write that fails the restore (`restore_failed`), `frame_limit`, and the 8 MiB page
cut. A client that needs them must fake them itself.

Tests impose their own finite timeout. The timeout scenario intentionally sleeps.
The out_of_order scenario deliberately holds every odd response until a second
request; do not sequentially await the first response. Detach always replies and
exits immediately. Events are full copies; seq is monotonic per process.

Representative `.jsonl` fixtures in ../protocol/fixtures contain only wire frames.
Request/response IDs tie methods to results; schema validation does not replace
cross-frame assertions. Run `python3 src-core-sdk/conformance/test_fake_adapter.py`
for stdlib smoke tests. If jsonschema is installed these also validate every
`*.jsonl` file in that directory, whichever files exist.

End-to-end smokes: `python3 src-core-sdk/conformance/smoke_config.py <path to koma>`
drives the real adapter through the config methods; `smoke_sessions.py`,
`smoke_usage.py`, `smoke_skills.py` and `smoke_room.py` do the same for the sessions,
usage, skills and room families, with the same arguments. They need a debug
build (`target/debug/koma`), because only debug builds honour `KOMA_TEST_DATA_DIR`; they
run with a fresh temporary data directory and `KOMA_TELEMETRY=0`, make no model
call, and fail if a planted secret appears in the adapter's output.
`smoke_config.py` also runs with `KOMA_TEST_OFFLINE=1` (no update check), keeps the
session daemon's catalogue refresh off the network (a fresh, empty catalogue cache,
the OAuth catalogue and HTTP proxies pointed at a closed loopback port, and a check
that no log names a public catalogue host), validates every frame against
`protocol/schema.json` when jsonschema is installed, and stops the session daemon it
started and removes its data directory.
`smoke_fresh_home.py` runs the adapter on a data directory that does not exist yet
and then on an empty one, with `KOMA_TELEMETRY=0` and `KOMA_TEST_OFFLINE=1`: every
`config`, `sessions`, `usage`, `skills` and `room` read answers empty, `missing` or
`not_found` (never `io_error`), and the reads create nothing in the data directory.
