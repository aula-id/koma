# Coding IDE implementation

The accepted scope extends the existing Monaco/LSP editor into a daily coding
environment. Workspace selection remains Koma's dropdown, with the existing
two-pane layout, visual tokens, and interaction patterns.

## Delivery ledger

| Milestone | State |
| --- | --- |
| Host/workspace identity and independent coding lifecycle | In progress |
| Document views, recovery, history, external changes | In progress |
| Language packs, toolchains, environment selection | Initial implementation; platform coverage pending |
| Quick Open, commands, outline, navigation, settings | In progress |
| Workspace edits, formatting, rename, code actions, LSP extensions | In progress |
| Search/replace preview and task runner | In progress |
| Debug Adapter Protocol and debugger UI | Initial implementation; native acceptance pending |
| Test Explorer and complete language catalogue | Built-in runners delivered; catalogue parity pending |
| Cross-platform integration and native acceptance | Pending |

## Product decisions

- Every document, diagnostic, installation, and running process belongs to an
  explicit host/workspace. Changing dropdown, chat session, or host must not
  retarget existing work.
- Language support includes runtime/compiler installation. System prerequisites
  use curated OS installers with native elevation and a concrete install plan.
- Project configuration is Koma's own optional `.koma/coding.json`. No VS Code
  workspace/configuration import or extension-host compatibility is planned.
- Cover Rust, JS/TS, Python, Go, C/C++, PHP, Lua, Zig, Bash, Nix, JSON/HTML/CSS,
  and TOML. Capabilities distinguish unsupported platforms and non-applicable
  operations from missing installation. PHP retains Intelephense and offers
  Phpactor where supported; premium capabilities require the user's licence.
- Keep language-pack binaries shared per host/version and project environments
  independent. Tasks/debuggers/tests continue across UI context changes.
- Commit substantial changes independently. Static Rust/TypeScript checks are
  performed during implementation. Regression sources and native acceptance
  instructions are supplied; functional test execution belongs to the user.

## Required acceptance

Identical paths on separate hosts and multiple same-language roots must remain
isolated. Exercise typing during save/format, stale replies, failed writes,
external changes, recovery, cross-file undo, install interruption, environment
selection, debug attach/detach, discovery/run/cancel/rerun of tests, and SSH
reconnect. Native review covers Koma's Windows, Linux, and macOS targets. A
successful compile is not evidence of native workflow correctness.

## Delivered increments

- Workspace-scoped language server process IDs avoid collisions between roots
  using the same language server.
- Native coding RPC carries host/root and request identity, with bounded queues
  and independent SSH channels. Language requests now use this service; legacy
  file/editor operations have not yet fully migrated. SSH requires the updated
  remote binary.
- Quick Open (`Ctrl/Cmd+P`) searches one dropdown root or all configured roots;
  command palette (`Ctrl/Cmd+Shift+P`) exposes editor actions. File traversal
  respects ignore rules, limits depth/file count, and reports truncated results.
- Monaco Go to Symbol uses the existing document-symbol LSP bridge, rejecting
  replies for an obsolete model version.
- Recovery/history storage and project configuration RPC are foundation APIs;
  their editor UI integration is a separate increment.

### Native review: navigation

Open two roots, find a file with Quick Open in each, then choose All workspaces.
Check keyboard navigation, Escape/focus, Unicode paths, ignored directories,
empty results, and a disconnected remote. Open Go to Symbol in a supported
language, edit while results arrive, and verify stale results are ignored.

- Recovery snapshots are written during edits (300 ms throttle, plus visibility
  and unload flush). Save/explicit discard writes a revision tombstone. Remote
  drafts stay on the local machine under their original host/root identity.
- Local History records before/after save and before restore/revert. The editor
  history button opens a read-only diff; restoring changes the unsaved buffer.
  `Recover Unsaved Files` in the command palette lists durable workspace drafts.
  Old-window entries remain available until explicitly discarded.
- Recovery stores at most 24 MiB per draft; history retains 30 days and 500 MiB
  per host, independently of unsaved drafts. Errors are visible through toasts.
  Abrupt process termination can lose edits not yet acknowledged by storage;
  this increment does not claim synchronous durability on every keystroke.

### Native review: recovery/history

Edit a file, wait for the snapshot write, restart, and use Recover Unsaved Files.
Compare before restoring, save, then inspect Local History. Repeat on SSH while
switching chat sessions and disconnecting. Exercise typing during save, discard
with a delayed backup, two windows editing the same path, restore while editing,
and an external disk change before saving recovered content. Regression source
covers monotonic revisions, tombstones and host-scoped history; execution is
left to the user.

- External-change detection polls open documents every five seconds and on
  focus/visibility, including SSH. Unchanged files return fingerprints only.
  Clean buffers reload; edited buffers retain text and expose Compare and
  resolve. Choosing the editor version adopts the compared disk baseline but
  leaves the buffer unsaved; Save still checks for subsequent disk changes.
  This is a polling fallback, not yet an event-driven filesystem watcher/tree
  reconciler. Missing/non-text files keep the buffer and report an error.
- Editor saves now use a synced temporary file and atomic replacement, preserving
  mode and resolving contained symlinks to their targets. Read-only files and
  Unix hardlinks are rejected explicitly. In-process editor saves serialize;
  an unrelated process can still race between hash validation and replacement.

### Native review: disk changes

Change a clean file externally and verify reload without activating a different
file. Repeat with unsaved edits, compare both versions, keep the editor version,
then change disk again before Save to confirm another conflict. Check symlink,
permissions, read-only files, SSH loss, and rename/delete outside Koma. Atomic
save regression source is included; platform execution remains with the user.

- Existing document notifications/completion/hover/navigation now route through
  workspace-owned language managers, with notification acknowledgement before
  dependent requests. Format document/selection, signature help, inlay hints,
  implementation and type-definition providers check server capabilities and
  discard results after cancellation, edits or host changes.
- History/external reload applies model edits with undo boundaries and updates
  the language document. Conflicted buffers remain editable with Save blocked
  until explicit resolution. Language processes are cleaned up on GUI close or
  coding-worker EOF. Persistent SSH job adoption remains a later milestone.

### Native review: language features

Open two projects using the same server, exercise completion in both, format a
selection and document, and undo. Type while formatting runs: the old result must
not replace new text. Check signature help, inlay hints, implementation/type
navigation, unsupported capabilities, server crash/reopen, and remote host
switching. Verify restoring history immediately changes hover/diagnostics.

- Rename Symbol (F2) and text-edit code actions open a multi-file diff preview.
  Apply validates current buffers and disk fingerprints for every file, records
  inverse history, and changes buffers together. Autosave is held for these
  buffers and restored drafts until explicit Save; Save All is in the palette.
  Undo Workspace Edit restores buffers only when every target still matches the
  applied content. File resource operations, command-only code actions and
  unresolved actions are explicitly unavailable in this increment.
- Workspace edits are bounded to 100 files/20 MiB and the initiating root.
  LSP document versions and UTF-16 ranges are checked; overlapping edits fail
  without partial application. File URIs now escape reserved path characters.

### Native review: refactoring

Rename a symbol across multiple files, inspect diffs, apply, undo the workspace
edit, and save explicitly with autosave enabled. Try typing or modifying disk
while preview is open. Exercise UTF-16/supplementary characters, spaces, `#` and
`%` in filenames. Check supported quick fixes and the unavailable explanation
for code actions requiring server commands/resource operations. Added pure edit
regressions cover UTF-16 offsets, insertion order, overlaps and invalid ranges.

- Replace All now builds a read-only native plan using the existing regex/glob
  matcher and opens the shared multi-file preview. Apply/undo uses the same
  revision/fingerprint protections and explicit-save buffers as refactoring.
  Oversized plans fail instead of applying a truncated prefix. Literal search
  treats `$` in replacement text literally; regex mode supports captures.

### Native review: replacement

Preview literal and regex replacements with include/exclude filters; cancel must
leave disk and buffers untouched. Apply, undo, and explicitly Save All. Modify a
candidate after preview and confirm no partial application. Verify capture
replacement, literal dollar text, CRLF/UTF-16 preservation on save, and limits.

- Optional project editor settings, language-specific overrides, snippets and
  editor keybindings now load from `.koma/coding.json`. The palette creates a
  missing config with exclusive creation, then uses the normal editor and
  fingerprint-protected Save. Settings never reshape Koma's workspace UI.
  See `CODING_PROJECT_SETTINGS.md` for supported fields and examples.

### Native review: project settings

Use different indentation/snippets/bindings in two dropdown roots and verify
isolation. Edit/save settings while both editors are open; remove an override
and check defaults return. Try malformed JSON, invalid keys, existing config,
external config updates followed by focus, and SSH configurations. Confirm
project commands do not leak into other workspaces or the chat composer.

- Final cache checks bind cached Monaco models to their original host and clear
  the visible diagnostics/runtime list on host changes. A late Peek read cannot
  overwrite a concurrently opened/edited model. History/refactor diff editors
  load Monaco on demand so these overlays do not add it to the startup bundle.

### Task runner increment

- Added explicit `.koma/coding.json` Run/Build/Test tasks, independent native
  process ownership, workspace-scoped run history, paged bounded output, Stop,
  optional deadlines, and process-group/job cleanup. Starting tasks validates
  the displayed definition fingerprint and prevents concurrent duplicates.
- Added Tasks footer/palette entries and a compact panel with its own workspace
  dropdown, command preview, output selection, and Stop. Closing the panel or
  switching the main workspace/chat/host leaves existing tasks running.
- Configuration and native acceptance cases are documented in
  [Project tasks](CODING_TASKS.md). Noninteractive execution is supported;
  durable SSH adoption, interactive task terminals, task dependencies, automatic
  discovery, and problem matchers remain future work. Regression sources are
  supplied; functional execution remains with the user.

## Work still required for the full accepted plan

The delivered increments are not full VS Code parity. Remaining work includes
managed language packs/toolchains/system prerequisite installation across the
full catalogue, task discovery/dependencies/problem matchers, DAP debugger,
Test Explorer, persistent SSH job adoption, complete document/view identity
across hosts and chat sessions,
event-driven tree reconciliation, navigation history/outline UI, semantic
tokens, format-on-save, and command/resource/resolve code-action support.
Native/platform acceptance and regression execution remain with the user.

## Editor and document lifecycle increment

- Added opt-in `editor.formatOnSave`, including language overrides. Save All and
  autosave share the guarded pipeline; changed buffers/hosts discard stale
  formatting, and failures leave the buffer unsaved with a visible error. The
  configuration document itself always remains saveable.
- Added full-document semantic tokens with server-legend translation, optional
  Outline panel, per-host Back/Forward navigation (Alt+Left/Right), and retained
  cursor/scroll view state. Outline is available from the coding palette.
- Chat switches preserve coding tabs/buffers. Switching hosts banks their coding
  buffers/layout separately and replays pending file replies into the originating
  host when it becomes visible again. Media/editor views remount across hosts.
- File tree, read/save, create/rename/delete, upload/download and content search
  now route through the independent workspace coding service. Save-as dialogs
  stay on the local GUI even when file bytes come from SSH. Transport frame limits
  accommodate the existing 25 MiB binary-file limit.
- Added debounced native filesystem notifications with bounded watcher retention
  and existing fingerprint-based conflict checks. Five-second polling remains as
  fallback for unsupported filesystems, disconnected watches, and evicted roots.

Native review: preserve dirty files and splits across chat/host transitions;
open identical paths on two hosts, including binary previews; switch while a
save/read/rename is pending. Exercise format-on-save with concurrent typing and
failure, Outline after edits, Back/Forward, semantic colors, and external
create/rename/delete. Functional/platform execution remains with the user.

### Language packs, task dependencies and DAP

Language Packs now reviews host-bound installation commands, runs them in Tasks,
and selects a project runtime/interpreter. LuaLS/ZLS/nil managed provisioning is
connected to the existing LSP resolver. Task discovery and dependency execution
are implemented. See [language packs](CODING_LANGUAGE_PACKS.md) and
[tasks](CODING_TASKS.md) for supported platforms and remaining limitations.

The debugger provides a workspace/session selector, launch/attach, persistent
breakpoints, conditions, stepping, call stacks, variables and watch expressions.
See [debugger configuration and acceptance](CODING_DEBUGGER.md). Adapter reverse
terminal requests and native cross-platform acceptance remain outstanding.

### Test Explorer

Workspace-bound discovery, run selection, cancellation, failed-test reruns and
structured results now use the shared task supervisor. Python/Go/Node tests can
launch a DAP session; other runners can select a project debug profile. See
[Test Explorer](CODING_TEST_EXPLORER.md) for the supported runner contracts,
current Node/Cargo identity limits and native acceptance.

### SSH adoption

Unix SSH workers now proxy to a persistent per-user coding service; reconnecting
adopts its existing tasks, debuggers, test results and language processes. See
[remote service lifecycle](CODING_REMOTE_SERVICE.md). Windows remote persistence,
crash durability and daemon-version migration are still separate acceptance/scope
items; they must not be implied by the Unix implementation.

### Advanced language actions and resource edits

Lazy code-action resolution, advertised server commands and preview-mediated
`workspace/applyEdit` are connected. File create/rename/delete uses native
preflight, inverse journals, rollback and disk-guarded Undo; see
[resource edits](CODING_RESOURCE_EDITS.md) for limits and acceptance.

### Interactive Tasks

Tasks now offers configured PTY execution, queued input, resize, process-tree
cancellation and source-location links for common compiler/traceback output.
The terminal reuses Koma's existing palette and font. Windows uses a startup gate
before releasing the real command into its job object. Platform testing remains
with the user; see [Tasks](CODING_TASKS.md).
