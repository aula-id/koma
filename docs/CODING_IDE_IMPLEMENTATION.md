# Coding IDE implementation

The accepted scope extends the existing Monaco/LSP editor into a daily coding
environment. Workspace selection remains Koma's dropdown, with the existing
two-pane layout, visual tokens, and interaction patterns.

## Delivery ledger

| Milestone | State |
| --- | --- |
| Host/workspace identity and independent coding lifecycle | In progress |
| Document views, recovery, history, external changes | In progress |
| Language packs, toolchains, environment selection | Pending |
| Quick Open, commands, outline, navigation, settings | In progress |
| Workspace edits, formatting, rename, code actions, LSP extensions | In progress |
| Search/replace preview and task runner | Pending |
| Debug Adapter Protocol and debugger UI | Pending |
| Test Explorer and complete language catalogue | Pending |
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
