# Code actions and file resource edits

Code actions can resolve lazily, preview text/resource edits, and run a command
advertised by the language server. Command name and arguments are shown before
execution. `workspace/applyEdit` requests open a separate preview and receive an
explicit applied/rejected result. Previews expire after three minutes; inactive
hosts and conflicting previews reject the request. Commands run asynchronously,
so waiting for a preview does not occupy the SSH request channel.

Create, rename and delete operations support regular text files inside the
initiating workspace. Ordered LSP document changes are simulated for preview.
Directories, binary files and edits spanning roots are rejected before apply.
Affected buffers must be clean. The preview explicitly distinguishes these disk
operations from ordinary text-only edits that stay in unsaved buffers.

Native apply preflights every path, disk fingerprint, size and text format under
one in-process mutation lock. It preserves encodings and line endings, writes
synced inverse images before mutation, and rolls back completed writes if a later
step fails. External processes can still race filesystem operations; an external
change during rollback preserves the recovery journal instead of overwriting it.
Multi-file operations are not crash-atomic. Incomplete journals are retained at
`~/.koma/coding/transactions/<id>` and named in errors for manual recovery.

Undo verifies all post-apply fingerprints and restores original bytes and Unix
modes from the journal. Ten completed transactions are retained per host, in
addition to ordinary Local History. Pending/incomplete journals are not pruned.
Typing or changing hosts during apply never replaces a newly edited buffer with
the result; such a buffer remains available for conflict resolution.

Native acceptance (not executed): ordered create/edit/rename/delete, overwrite
and ignore options, UTF-16/BOM/CRLF round trips, stale disk/buffer versions,
mid-apply failure and rollback, interruption/recovery, Undo after later changes,
command-only actions, lazy code actions, and accepted/canceled/expired server
workspace edits. Concurrent GUI windows requesting the same server edit require
further arbitration before claiming multi-window ownership.
