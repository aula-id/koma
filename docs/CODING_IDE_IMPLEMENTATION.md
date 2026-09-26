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
| Workspace edits, formatting, rename, code actions, LSP extensions | Pending |
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
  and independent SSH channels. Existing file/editor and LSP frontend routing
  has not yet migrated to this service. SSH requires the updated remote binary.
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
