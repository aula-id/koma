# Secure room

A coding agent with a security lane bolted on the side is the wrong shape: the
capability leaks, the boundary is wherever the prompt last said it was, and a
"confirmed" finding turns out to be a tool exit code. The secure room is its own
daemon and its own module, and this document is the contract the code enforces.

The room replaces the earlier Cyber mode (`src-security`, the `sec_*` tool
catalog, the Docker egress boundary and `app/sec`), which no longer exists.

## Separation from the coding chat

`src-agent/src/room/` is its own module and reads its registry from
`src-misc/room`. The coding chat does not drive it. The only code outside
`room/` that touches it is the `koma room` CLI (`room/cli.rs`), daemon startup in
`main.rs`, the SDK room methods (`app/runtime/sdk/room`), and the read-only
`secure_shelf` tool. The room runs as
its own daemon kind, started with `koma --daemon --kind secure`. `/security` in
the terminal hands the session to it, and `/security` again returns to the coding
chat.

The room's session id is derived from the coding session that opened it:
`room_session_id` in `room/mod.rs` (UUID v5 of `"secure-room:" + parent`). The
terminal derives the same id in `roomSessionId` (`src-term/src/paths.ts`), and a
test pins the two to the same values. The room's database is
`<session dir>/room.sqlite`.

## Avatars, roles and lanes

`src-misc/room/avatars.toml` lists who is in the room; each avatar points at a
role under `src-misc/room/roles/<role>/` (`role.toml`, `prompt.md`,
`tools.toml`). Roles: chief, clerk, red, re, reviewer, reporter, researcher,
builder, blue. The chief and the clerk are single; specialist roles may have
several avatars (`red-web`, `red-api`).

What a role may do on the shelf is `shelf_allow` in its `role.toml`, enforced by
`Shelf::call_as`. A sentence in a prompt is advice; the allow list is checked
before a write. The reviewer cannot rewrite a claim, because that would move the
revision the review is judging.

Which role may take which assignment is `AssignmentKind::roles` in
`room/types.rs`: probes go to red or re, verification to the reviewer, reports to
the reporter, execution to the builder or blue.

Every avatar has two loops (`room/lane`): a chat lane that answers the user and
the Chief in short turns, and a work lane that runs one assignment. The actor
(`room/actor`) owns both, and everything reaches it as one message on one
channel.

The registry (`room/registry`) has three layers, lowest first: files embedded
from `src-misc/room` (always valid), a template directory
`<koma data dir>/room/`, and a per-session overlay `<session dir>/room/`. A
broken overlay file is skipped with a warning and the embedded version stays in
force. A turn keeps the snapshot it started with, so a reload never changes a
running turn.

## The shelf

The only channel between agents (`room/shelf`). One SQLite table of
`key -> (content, revision)` in WAL mode, with typed writers per kind and a key
prefix each: `scope/`, `claims/`, `evidence/`, `verdicts/`, `findings/`,
`reports/`, `assets/`, `trajectory/`, `results/`. A record is at most 128 KiB.

Writes are compare-and-swap on `expected_revision`; assets and trajectories merge
into the current revision instead. Results cite `{key, revision}` references and
`verify_references` refuses one that changed or vanished, so a verdict always
names the exact revision it judged.

Verdicts are `confirmed`, `returned`, `dismissed` or `discarded`. A confirmed
claim needs a published finding before its review can complete.

## Proof and the oracle

A claim may carry up to 16 `proof` entries, `{path, needle}`: this exact text is in
this file. `room/oracle.rs` opens the file and looks, in the background and off
every lane. The result is `grounded`, `refuted` or `unproven`, written under
`notes/oracle/`. Only a program can establish that cited code is not there, so
`refuted` goes back to the red lane that made the claim. `unproven` (no proof, no
workspace, a path outside it, a file over 4 MiB, a needle under 8 characters) is
held for review and never a rejection. `grounded` says the code exists, not that
it is a bug; that stays the reviewer's call.

## Scope

Scope records on the shelf hold `allow` and `deny` lists (`room/clerk/scope.rs`).
Matching is strict, because a false "in scope" hides a violation while a false
"out of scope" costs the Chief a glance: an exact host, `*.example.com` for any
depth of subdomain but not the apex, an IPv4 CIDR, and a deny entry always wins.
Allowing `example.com` does not allow `app.example.com`.

## Finishing

`assert_finishable` (`room/work/finish.rs`) is the rule for closing an
engagement: all live assignments completed and passed, and every production
result (a probe) covered by a passed verification of its claims, evidence and
results. Otherwise the Chief gets the reason and cannot finish.

State changes for assignments and batches are pure functions in
`room/transition.rs`: callers pass in what they observed and apply the result.
Work that cannot be proven stopped is uncertain, never success.

## The clerk

An automatic sanity check over the work lanes (`room/clerk`). It speaks only to
the Chief, cannot pause or stop anything, and never edits anyone's work; it
raises a flag and the Chief decides.

Deterministic checks (`audit.rs`) run on every 4 s tick at no cost: out of scope,
no scope record, stalled, unpublished work, repeated work, claims without
evidence, contradictory severities, a review loop, a report due, duplicate
claims. A model pass runs only while work is active, never twice at once, and
only when something changed, at least 30 s apart for shelf-only changes.

## Reading a room from outside

- `koma room trajectory [--goal KEY] [--format ascii|json|dot|mermaid]
  [--view outline|layered] [--width COLS] [--session ID]` draws the map from the
  shelf. It opens the database read-only, so it is safe against a live session.
- `koma room attach --session ID` is a line client for a running secure daemon.
- The `secure_shelf` tool (`tool/secure_shelf.rs`) lets the coding agent list the
  project's room sessions, browse a shelf, read an item or a report. It never
  writes.

## Verifying

    cd src-agent && cargo test room

CI also runs `koma --daemon --kind secure --daemon-selftest` (bind, accept, one
client round trip, clean exit over the native IPC the chat uses).

## Known limits

- The room does not isolate a target in a container or restrict the network. Scope
  is checked by the clerk's audit and the lanes' tool rules, not by an egress
  boundary.
- The CWE "speculation" follow-up on confirmed verdicts is left out of the shelf
  port.
