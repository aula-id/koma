# Native Git workbench review

Functional testing is intentionally delegated to the user. The implementation
has been checked with TypeScript and Rust compilation; the scenarios below have
not been exercised in the native application. Browser review cannot verify the
native IPC, filesystem, Git subprocesses, or SSH transport.

Use a disposable repository for destructive actions. Repeat the main workflows
with a local session and an SSH session whose remote Koma includes this update.
The existing Koma colors, button styles, tab groups, and Monaco setup are reused.

Static checks: `cargo check -p agent --offline --features gui` and the WebGUI
`tsc --noEmit` pass. The documentation project's typecheck reports two routing
type errors in unchanged `src/components/Sidebar.tsx:214` and
`src/routes/_docs/docs/$slug.tsx:39`. No functional test suites or browser/native
review were run for this change.

## Entry points and expected behavior

| Feature | Where to open it | Native review |
| --- | --- | --- |
| Partial staging | Source Control → changed file → Hunks | Select a hunk or individual added/deleted lines, stage, then inspect staged and unstaged changes. Unstage a subset from Staged Changes. Working-file content must stay unchanged. |
| Conflict editor | Source Control → Conflicts → file | Current and incoming are read-only above an editable result. Try next/previous conflict, accept current/incoming/both, manual edits, Save result, and Mark resolved. Save alone must not stage; Mark resolved saves and stages. |
| Interactive rebase | Graph → right-click base commit → Interactive rebase after here | Reorder, pick, reword, squash, and drop. The base is excluded; HEAD is the last input commit. Review resulting order/messages and the recovery ref shown on completion or error. |
| Amend | Source Control → Amend last commit | HEAD's message is preloaded. Amend with staged changes, and amend only the message. Unstaged work must stay unstaged. Cancel the confirmation or turn off the toggle to return to the normal draft. |
| Branch management | Source Control toolbar / graph ref Manage action | Rename local branches; delete a merged local branch; refuse current or occupied branches; confirm force deletion of an unmerged branch. Remote branch deletion names the remote explicitly. |
| Tags | Source Control toolbar / graph commit → Create tag here | Create lightweight and annotated tags at the selected commit. Delete locally; separately confirm pushing a tag or deleting it from the selected remote. |
| Stashes | Source Control toolbar / graph → Manage stashes | Name a stash with and without untracked files. Inspect its patch; apply, pop, and drop selected entries. Pop conflicts must retain the stash. |
| Reflog and recovery | Source Control toolbar → Reflog | Load another page, open an entry's commit, create a recovery branch, and right-click for existing graph/reset actions. This is ref history, not a universal undo stack. |
| Blame | Coding file context menu / working diff → Blame HEAD | Verify author, commit, and line numbers against HEAD; click a line's attribution to open the commit. A modified working file must show the committed-version notice. |
| Remotes | Source Control toolbar → Manage remotes | Add, rename, edit fetch/push URLs, and confirm removal. Saving configuration must not fetch. Try SSH push/tag deletion using the repository's selected key. |
| Image diff | Click an image in working/staged changes or commit detail | Check both revisions, dimensions, byte sizes, Fit/Actual size, added/deleted images, renamed images, and a decode error. SVG is displayed as an image. |

## Cases worth checking

- Partial staging: existing and new files, a repository without HEAD, deletions,
  multiple hunks, CRLF, and a missing final newline. Selecting a replacement
  means selecting both its deleted and added lines; each row is independently
  selectable. Hunks in the staged view describe **Index → HEAD** (the unstage
  operation); side-by-side and image views use **HEAD → Index**.
- Change a file/index externally after opening its diff. The stale selection
  must fail with a refresh request. Binary files, renames, mode changes, custom
  clean filters, and working-tree encodings use whole-file actions.
- Conflicts: merge, cherry-pick, rebase, diff3 markers, binary and modify/delete.
  Rebase labels must explain that current is the rebased destination and incoming
  is the replayed commit. Remaining markers block Mark resolved. Closing a dirty
  result prompts for discard. External changes require reload before saving.
- Rebase: a clean linear range of up to 500 commits; reject dirty worktrees,
  detached HEAD, merge commits, a non-ancestor base, and squash as the first
  retained step. Resolve a conflict and Continue; separately exercise Abort.
  Helper files persist while paused and are cleaned after Continue/Abort.
- Switch repository while a view is open: its title/root stay bound to the
  original repository and actions disable until that repository is active.
  Switch session during a request: a late reply must not populate the new session.
- Remove/reorder a stash externally after selecting it: operations resolve its
  object ID again rather than acting on a stale numeric slot.
- Close and reopen tabs, move them between split groups, and inspect narrow
  layouts and both light/dark themes in the native app.

## Current scope

- Text preview is limited to 2 MiB per side. Image comparisons support PNG,
  JPEG, GIF, WebP, BMP, ICO, AVIF, and SVG, with a 16 MiB combined limit.
- Symlink/submodule conflicts require external whole-file Git operations.
- Rebase uses Git's native sequencer and local recovery refs under
  `refs/koma/recovery/`. There is no automatic deletion of recovery refs.
- Repository remotes may have multiple URLs configured externally; the editor
  exposes Git's primary fetch/push URL. Manage complex URL lists with Git config.
- Network operations still depend on installed Git, authentication, and the
  remote server. A timeout is not proof that an operation was cancelled: refresh
  repository status before retrying.
