# Coding save reliability review

This update addresses save acknowledgements and file-format preservation. It
keeps the existing editor layout and controls. Native functional testing is
left to the user; regression cases are included but were not executed here.

## Changes

- Save acknowledgements match a pending request ID and the exact text snapshot
  sent with it. Newer edits remain dirty, including undo back to the previous
  disk text while a different snapshot is being saved.
- A second explicit Save queues the latest buffer after the first acknowledgement.
  Autosave follows up when edits remain. LSP sees the saved snapshot followed by
  the current dirty buffer, so save notifications do not replace newer edits.
- Failed saves keep the buffer and baseline. Stale/duplicate replies are ignored.
  Rereads cannot replace dirty text or silently adopt another version's disk
  fingerprint. Closed files and previous sessions ignore unmatched read replies.
- Closing an editor waits for a pending save. Any remaining edits are saved with
  autosave, or require discard confirmation. Path changes/deletion are blocked
  while the affected file is saving. Dirty-tab lookup supports colons and pipes
  in filenames and Windows paths.
- Missing IPC and remote transport failures complete the matching save request
  with an error. A remote transport failure can have an unknown save outcome;
  the buffer is retained in memory. Reload discards it; this update does not add
  a conflict comparison or recovery interface.
- Disk reads decode UTF-8 strictly, with an optional BOM, and BOM-marked UTF-16
  in either byte order. Saving restores the verified file's encoding, BOM and
  uniform LF/CRLF/CR line endings. The editor/LSP buffer still uses LF internally.
- Unsupported encodings, malformed Unicode and mixed line endings are not
  silently converted. Mixed line endings require explicit normalization outside
  this editor. Unsupported encoding files remain downloadable in original form.
- Fingerprints cover all on-disk bytes and are computed from the same read that
  produced the editor content. The format is re-read and verified before save.

## Native review

Use local and updated SSH sessions. Do not treat a successful compile as native
workflow validation.

1. Save a file, keep typing while the write is pending, and inspect the disk
   content. The extra edit must remain dirty. Save again and verify it lands.
2. Repeat with autosave enabled, and with an explicit second Save during the
   first write. Close while saving; the tab must wait and handle any newer edit.
3. Save, undo back to the old disk content before acknowledgement, then close.
   The undo is a new unsaved change relative to the completed save.
4. Check dirty-close confirmation for ordinary paths, nested paths, Windows
   drive paths, and (where supported) filenames containing `:` or `|`.
5. Change the file externally, then save from the editor. Verify the external
   file is intact and the local buffer retained. Also try a disk permission error.
6. Disconnect SSH during save. The editor must surface an error rather than
   remain at Saving forever. Verify the actual remote content before reloading.
7. Edit UTF-8 files with LF, CRLF, CR, a UTF-8 BOM, and no final newline. Verify
   the raw saved bytes retain the original format, without an extra BOM/newline.
8. Edit BOM-marked UTF-16LE and UTF-16BE files containing non-ASCII characters
   and emoji. Verify encoding, byte order, BOM, line endings, and text survive.
9. Open invalid UTF-8, malformed UTF-16, and mixed-LF/CRLF files. Editing must
   be unavailable; their bytes must stay unchanged. Check image/PDF previews too.
10. Start a read/save then close or switch sessions. Late replies must not mark
    a different or reopened buffer as saved or repopulate a closed file.

Regression sources: `src-webgui/src/store/coding.test.ts` and
`src-agent/src/app/runtime/client/file_ops_test.rs`.

## Scope

This does not add crash recovery, a merge UI for external-file conflicts, editor
formatting/refactoring, a new encoding picker, or workspace-wide undo. Save still
uses the existing filesystem write operation; it is not a cross-process lock or
an atomic save transaction. Editing and search have separate format support:
workspace search/replace still skips unsupported/binary input, including UTF-16.
