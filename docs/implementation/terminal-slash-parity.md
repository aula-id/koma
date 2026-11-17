# Full-screen terminal command parity

Updated 2026-10-15. This inventory covers the OpenTUI client in `src-term`, the
legacy render/input implementations in `src-agent/src/view` and
`src-agent/src/controller/input`, the shared runtime actions, and IPC projections.
The chat, composer, command palette, and compact overlay placement stay in place.
Scope is a function-for-function legacy port with refreshed UI/UX. Existing
handlers and runtime operations define behavior; do not add product features or
new save policies as part of the migration.

## Protocol and state

`ClientRequest::Menu` is additive and advertises protocol v1 through
`GetMenuCapabilities`. Each operation has a request ID and a matching
`MenuResult`. Plain `Ack`, an unrelated error, and a stale result cannot complete
it. Composer buffer flushes use `ReplaceInputTracked` for the same reason.
Unsupported peers get a host-update/reconnect message.

Selection targets an actual group/index and verifies entity IDs where available.
There is no arrow replay in full-screen pointer controls. Footer controls are
explicit descriptors, independent of explanatory hint text. Keys invoke the same
runtime/controller actions as the legacy client. Menu contracts and domain reply payloads are generated from Rust by
`src-term/scripts/generate-menu-types.py`.

Native input and textarea buffers retain their cursor, selection, paste, and
unsaved content across snapshot updates. Secret inputs begin blank, render masked
text, and retain saved credentials when left unchanged. Settings apply and persist
themes on Enter, toggles/choices on activation, and text edits on confirmation.
Highlight movement alone never applies a theme. Esc cancels the current
unconfirmed edit or returns; no screen-level Save step or prompt is involved.
Existing provider/model/search forms retain their submission controls. Settings
persistence uses the legacy runtime operation while keeping the screen open;
leaving a clean screen does not rewrite its older catalogue. Catalogue broadcasts
preserve drafts and navigation.
Notification and session refreshes preserve selected entity identity.

## Command/state inventory

| Flow | Legacy input / runtime | Projection / new view | State coverage |
| --- | --- | --- | --- |
| `/settings`, `/config` | `input/settings`, settings actions | `SettingsSnapshot`, `modes/settings.tsx` | Compact dashboard, appearance/general groups, immediate theme activation, colored toggles, directories, confirmed native fields and cancelled edits without screen-level Save |
| Providers | settings modal/provider operations | settings provider drafts/modal | Full-width table and separate form; ownership refusal, preserved UUID and unchanged credentials |
| OAuth | settings OAuth flow and OAuth actions | `OAuthFlowSnapshot` | All eight daemon-projected choices, browser/device/token, waiting/progress, copy/open, cancellation, failure/retry |
| Models | settings model navigation/operations | conditional fields/results in settings snapshot | Scope/filter, catalogue search, provider, roles, conditional route, create/edit/delete |
| `/agents`, `/agent` | agent input/actions, model/tool pickers | `AgentsSnapshot`, `modes/manage.tsx` | Full-width roster/source badges; dedicated form, scope controls, model/tool selection, complete multiline fields, save and named delete/cancel |
| `/mcp` | MCP input/actions | `McpSnapshot`, management view | Full-width server table, transport/enabled/status/error, connected **zero** tools, separate conditional form and named delete |
| `/extension`, `/extensions` | extension input/actions | extension and screen snapshots | Installed list → dedicated page; running/activation/grants/contributions/workspace, selectable screens, global/on-demand controls, uninstall confirmation |
| `/store` | store input/actions | `ExtStoreSnapshot` | Catalogue → product page → install confirmation; versions, requirements, installed/authentication/loading/error/installing states |
| Extension screens | extension screen input/invoke | opaque screen snapshot, document renderer | Every v1 node (`text`, `kv`, `divider`, `menu`), ordered actions, selection visibility, pending/error/footer |
| `/security` | security input/actions | `SecuritySnapshot` | Daemon/YOLO controls, domain-grouped tools, separate tier-ordered dependencies, install/repair/domain/restart |
| `/remote`, `/new remote`, `/resume remote` | remote host actions and headless SSH helper | host snapshot + helper events, `remote-screen.tsx` | Host CRUD/import, auth retry, bootstrap stages, directories, live/history discovery, session open/new/kill/delete, reconnect, connection loss, detach/return to hub; same OpenTUI session UI |
| `/resume`, `/sessions` | local discovery/lifecycle commands | `hub.tsx`, session hub snapshot | Real IDs, current/working states, native search, live refresh, visible controls, named kill/delete confirmation, return to session |
| `/usage` | usage navigation/ledger collection | `UsageSnapshot`, analytics composition | Global and runtime session totals, range/metric, theme heat intensities, token bars/model breakdown, role split, scrolling |
| `/notification` | notification actor/input | notification mode snapshot | Native search, Session/App scope, severity filter, unread count, timestamps/source, bounded full details, pending/error, read state and confirmed scope clearing |
| `/help` | help input and command registry | `HelpSnapshot` | Full-width native search/reference; activation uses actual command dispatch |
| `/effort` | model capability-derived effort picker | `EffortSnapshot` | Content-sized choices/current cursor and capability explanation |
| `/quit`, `/q`, `/exit` | quit state/lifecycle | quit snapshot and `quit.tsx` | Content-sized quit/detach/cancel, working counts, actual lifecycle operations |
| Attachment/agent editors | editor input/actions | full editor snapshots/native textarea | Complete content, cursor, selection, paste, scrolling, return navigation and guarded clearing |
| Subagent transcript | live subagent snapshot | `AgentViewer` | Every message/full content/live text, local keyboard/mouse scrolling, return |
| Credentials/onboarding | onboarding/credential input/actions | onboarding/key snapshots | Native editable/masked fields, daemon catalogue results and OAuth states |

Compact Model, Bash, Todo, Skill, attachment list, extension-use, and old settings
menu placement remains governed by `modes/place.ts`. Immediate operations do not
create new pages.

## Verification

- `cd src-term && .tools/bun test`: rendered layouts, resizing, all ten themes at
  50×24, 80×18, 100×40 and 160×45; actual focus/status/shortcut colors; native
  secret editing/selection/paste; stable selection; explicit actions; correlation.
- `cd src-term && .tools/bun node_modules/typescript/bin/tsc --noEmit`.
- `cargo test -p agent ipc:: -- --test-threads=1` and focused `requests_menu::`
  tests cover protocol compatibility, snapshots and direct navigation/draft guards.
- `cargo build -p agent && python3 src-term/scripts/verify-daemon.py` uses a real
  daemon socket and isolated configuration to exercise read/edit/confirm/reopen,
  legacy form validation, cancellation, unchanged secrets/provider identity,
  concurrent updates, MCP/agent CRUD and help command activation.
- `python3 src-term/scripts/verify-remote.py` runs the actual Rust helper against
  an instrumented SSH executable and a real daemon. It covers password retry,
  version mismatch/install/verify/progress, directory/session discovery, real
  daemon frames, bridge loss, detach/reconnect and credential-free argv.
- Set `KOMA_PREVIEW_PATH=/absolute/path/previews.html` while running Bun tests to
  export all forty actual rendered color frames, plus captured span JSON.

The debug-only `KOMA_TEST_DATA_DIR` isolates application data for the
integration scripts; release builds retain the legacy data path. They do not repurpose HOME or write the user's profile.
Socket/process integration tests require permission when the execution sandbox
blocks local sockets or child process control.

## External verification still needed

The automated SSH harness does not establish behavior against an arbitrary real
SSH host. Real provider browser/device logins, authenticated marketplace installs,
security dependency provisioning, and external MCP/extension services need the
corresponding account/host/service. Fixture render coverage and a local transport
harness are not evidence that these external operations passed. Keep these checks
open until exercised with those services; do not mark this inventory as complete
release acceptance solely from the automated suite.

## Approval, composer and history follow-up (2026-10-15)

- Every runtime `awaiting_approval` park uses the paused call at `tool_idx`.
  Producers audited: generic Normal/TAC gates and classifier outage fallback in
  `src-agent/src/app/runtime/stream/tools/approval.rs`, destructive/security intercepts in
  `src-agent/src/app/runtime/stream/tools/intercepts/guard.rs`, plan gates in `intercepts/plan.rs`,
  and mission/human gates in `intercepts/sdlc.rs` (same directory).
  Their automatic allow/block policies are unchanged. The terminal uses the existing
  ApproveTool and PlanDecision operations, preserving the separate compact and
  discussion outcomes. Quit/loading precedence stays with the daemon mode.
- Approval owns keys and paste across chat, palettes, full-screen menus and viewers.
  Decisions are guarded by session/call identity until a resolving snapshot. Menus
  retain their mounted native drafts while their inputs and actions are suspended.
- Composer word wrapping uses OpenTUI's total visual line count after frames and
  resize. Paste/clipboard requests and chip removal flush the native draft, prevent
  duplicate insertion, and rebase newer typing onto returned markers. Staged media
  reconcile when native edits remove markers; pasted bodies collapse in transcripts.
- Runtime follow-ups own text and attachments. Both delivery boundaries expand pasted
  bodies and persist ordinary user messages with media. Text-only snapshot consumers
  retain their string array; the defaulted attachment sidecar participates in diffs
  and shadow reconstruction.
- Running version comes from daemon build metadata; the shared comparator filters
  the update indicator. New and reopened sessions start the existing async checker.
- `chat.test.tsx` exercises native input and captured renders at 50×24, 80×18,
  100×40 and 160×45, including resize, uppercase decisions, Esc, successive gates,
  duplicate input, menu/palette arrival, focus, paste reconciliation and wrapping.
  `KOMA_PREVIEW_PATH` also exports approval HTML and text frames for review.
  The daemon harness additionally verifies small/large paste, path images, editor
  reopening, kind-aware removal, marker deletion, and persisted notification scopes,
  severity, individual/all read and clear confirmation/cancellation.
- Real OS clipboard backends and every live provider/security/human gate remain
  external acceptance checks. Native input fixtures, runtime unit tests and an
  isolated socket harness do not establish those environment-specific integrations.

### Form navigation, code rendering and status follow-up (2026-10-15)

The current terminal distinguishes highlighted fields from active native editors.
Web-search provider selection precedes key editing, and model provider fields open
an anchored chooser using daemon-projected stable identities and canonical indices.
Pointer and keyboard confirmation share the runtime binding transition, including
query/route reset and catalogue refresh. Provider/model/credentials forms expose
defaulted editing state; single-purpose searches retain autofocus. Native drafts,
Unicode caret offsets and selections survive returning to browsing. Explicit form
Cancel actions close the form, while Esc first returns to browsing.

Markdown code tokens now render inside padded square borders with optional language
labels. Python and Rust parsers/queries are vendored with release and content hashes;
JavaScript/TypeScript/TSX reuse OpenTUI's local assets. Worker paths and language
aliases are explicit. Unknown languages remain framed text, including incomplete
streaming fences and literal backticks. Syntax colors use the active terminal palette.

Unselected follow-ups and pending attachment controls are neutral. Transcript images
are collapsed into wrapping attachment piles, with image rendering deferred. Native
image paste, attachment metadata and open/remove operations remain available.
Refreshing/loading/installing status uses a panel's bottom-right border or a reserved
footer row so it cannot shift content when it appears.

Verification uses Bun render/input tests, TypeScript, Rust settings/menu/protocol
checks, `scripts/verify-daemon.py`, and `scripts/verify-form-input.tsx`. The latter
uses a real isolated daemon plus a local catalogue server and actual OpenTUI keyboard,
pointer, native selection and bracketed paste. It verifies provider browsing before
editing, masked drafts and saved-key retention, cancellation/confirmation, catalogue
refresh, Save/Cancel, and resized 50×24, 80×18, 100×40 and 160×45 layouts. Native code
color spans are checked for Python, Rust, JavaScript and TypeScript across all ten
palettes at those dimensions, with local assets under restricted network access.
Captured frames are `/tmp/koma-form-frames.txt` and `/tmp/koma-code-frames.txt`.
Shared-focus tests also cover credentials, provider/model/MCP/agent/remote forms,
help and session searches, selection replacement, exactly-once paste and stable
transient-status positions. Existing approval, composer and attachment regressions
remain part of the Bun suite. Live external provider accounts, OAuth logins, SSH hosts
and future image rendering remain outside this verification.
