# Coding IDE implementation

Koma extends its existing Monaco/LSP coding surface. The workspace dropdown,
two-pane maximum, visual tokens and compact panels are retained. There is no VS
Code workspace import, extension host, or claim of complete VS Code parity.

## Delivered surface

| Area | Implemented behavior | Detail |
| --- | --- | --- |
| Documents | Host-scoped buffers and models, retained chat/host views, recovery, local history, guarded atomic saves, external-change comparisons | Below |
| Navigation/editor | Quick Open, command palette, Outline, Back/Forward, view state, snippets, keybindings, project/language settings, format-on-save, semantic tokens | [Settings](CODING_PROJECT_SETTINGS.md) |
| Language support | Existing LSP bridge extended with formatting, inlay hints, signature help, implementation/type navigation, lazy code actions and server commands | [Resource edits](CODING_RESOURCE_EDITS.md) |
| Refactoring/search | Multi-file diff preview, fingerprint/version checks, inverse Undo, replace preview, resource journals, recovery browser, clean renamed-tab remapping | [Resource edits](CODING_RESOURCE_EDITS.md) |
| Tooling | Reviewable managed packs, SDK/debugger prerequisite recipes, workspace interpreter/server selection, compatible stable ZLS selection, automatic LSP restart | [Language packs](CODING_LANGUAGE_PACKS.md) |
| Tasks | Manifest discovery, explicit recipes, dependencies, bounded output, cancellation, PTY input/resize, built-in and custom problem matchers | [Tasks](CODING_TASKS.md) |
| Debugger | DAP launch/attach, pre-launch tasks, breakpoints/conditions/exceptions, steps, stack, variables, persistent watches, evaluation console, integrated terminals, read-only adapter sources | [Debugger](CODING_DEBUGGER.md) |
| Tests | Python/Go/Cargo/Node/Jest/Vitest/PHPUnit/CTest adapters, Cargo doctests, custom JSON runner, discovery, selection, run/cancel/rerun failed, structured results, Rust rebuild before debug | [Tests](CODING_TEST_EXPLORER.md) |
| Remote/lifecycle | Independent workspace RPC, Unix/Windows SSH daemon adoption, safe daemon replacement at reconnect, window-isolated LSP state, native filesystem notifications plus polling fallback | [Remote service](CODING_REMOTE_SERVICE.md) |

Every coding request carries host/root ownership. Switching UI context does not
retarget existing task, test or debug processes. Local processes end with their
GUI service; remote Unix and Windows jobs belong to the daemon. Lost transport replies have
an unknown outcome and mutations are never replayed automatically.

## Document safeguards

- Chat switches retain coding tabs. Host switches bank buffers and layout and
  defer late file acknowledgements until the originating host becomes active.
  Cursor/scroll state and navigation history stay host-scoped.
- Recovery writes are throttled by 300 ms, with visibility/unload flushes. Explicit
  save/discard writes revision tombstones. Remote drafts remain on the local
  machine. History keeps 30 days / 500 MiB per host; each draft is at most 24 MiB.
  A crash can lose edits not yet acknowledged by recovery storage.
- Native watch events refresh trees and trigger fingerprint checks, with a
  five-second/focus fallback. File hints are also forwarded to existing workspace
  language servers. Notifications are bounded hints, not a durable event log.
- Clean externally changed files reload; dirty buffers retain text for comparison.
  Save validates the disk baseline, preserves encoding/BOM/line endings and mode,
  and performs synced atomic replacement. Read-only files and Unix hardlinks are
  rejected explicitly. Other processes can still race an in-process preflight.
- Format-on-save, language replies and edit previews reject stale buffer versions
  or host changes. Resource edits preflight the whole bounded plan, journal inverse
  bytes, and compensate on errors. They are not crash-atomic transactions.

## Supported boundaries

These are concrete limitations, not features implied by successful compilation:

- Native acceptance is outstanding on Linux, macOS and Windows. Windows native
  service code passed a non-GUI GNU-target cross-check; it has not been executed.
- Runtime recipes depend on host repositories/installers. Some tools require a
  preinstalled runtime, SDK, system package or project dependency. Package and
  adapter execution still needs native review. ZLS selection matches stable Zig
  major/minor versions; development Zig releases require a custom installation.
- Built-in structured adapters require the documented runner CLI/report formats.
  Custom Cargo harnesses and other frameworks use configured tasks or the JSON
  runner. Duplicate full names in a file, including JS multi-project variants,
  can share a runner filter/result identity. Framework project dependencies are
  not installed just by opening Test Explorer.
- Windows SSH persistence requires permission to break away from the SSH job.
  Daemon crash/reboot recovery and migration of running jobs between versions are
  not implemented. Version replacement waits for active jobs and clients to leave.
  Existing v2 jobs must finish before updating to v3; they are not adopted by v3.
  Window-isolated language managers do not provide collaborative document merging;
  concurrent disk saves retain fingerprint conflict checks.
- Resource edits cover regular text files within one root, not directory/binary
  transactions. Recovery refuses files independently changed since interruption.
  Dirty/overwritten/occupied rename destinations can keep conflict buffers instead
  of remapping tabs. Permissions or external changes can require manual recovery.
- DAP sourceReference content is read-only, bounded to 5 MiB in the viewer and
  tied to the current paused session. Automatic launch mapping for every framework
  is not implemented.
  Very old terminal screen state cannot be reconstructed after output truncation.
- Custom task matchers use Koma's bounded single-line Rust-regex schema; arbitrary
  VS Code matcher imports/multiline/background matching are not supported.
  Retained process histories/output are bounded and in memory.

## Validation and native review

The implementation agent performed Rust GUI/test-source compilation, non-GUI
compilation, Windows GNU-target non-GUI cross-checking, TypeScript type checking,
JavaScript syntax checking and whitespace
checks. It did not run functional tests, installers, GUI/browser reviews, project
tests, or debugger sessions, as requested. Regression sources are supplied for
the user to execute. Compilation is not evidence of native workflow correctness.

Review in the native app:

1. Open two roots and identical paths on different hosts; switch chats/hosts with
   dirty files and pending saves. Check split views, focus, history and recovery.
2. Exercise format-on-save while typing, server failure/restart, Outline/navigation,
   semantic colors, snippets/settings, and external file create/rename/delete.
3. Preview/apply/undo multi-file rename, code actions and replacement; change a
   buffer or disk file during preview. Exercise encodings, permissions and limits.
4. Review pack recipes, install/update/cancel, change interpreter/PHP server, and
   confirm the new environment in LSP, Tasks, Tests and Debug.
5. Run dependent and interactive tasks; enter input, resize, stop process trees,
   inspect truncated output and follow compiler source links.
6. Launch/attach each adapter, build before launch, move breakpoints by editing,
   inspect stack/variables/watches, evaluate expressions and detach cleanly.
7. Discover/run/cancel/rerun/debug tests, including duplicate Cargo names and nested
   Node/Go cases. Inspect collection failures, teardown failures and missing tools.
8. Disconnect/reconnect Unix and Windows SSH with jobs running; adopt output and
   stop the same job. Replace the remote binary, verify deferred service draining,
   and reopen buffers after reconnection. Check visible unknown outcomes.
9. Edit the same remote document differently in two GUI windows; verify separate
   language diagnostics, completion and edit previews, plus disk-save conflicts.
10. Review interrupted transactions before restoring; independently modify one
    affected file and verify that recovery blocks without overwriting it.
