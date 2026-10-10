# Sessions and workspace roots

## GUI
The change session control opens live and past sessions. Click selects; double-click or Enter opens. Ctrl/Cmd-click toggles selections and Shift-click selects a range. Kill stops a live daemon and keeps history. Delete forever removes saved history after confirmation. New session selects a folder; New session + close current first stops the old daemon. Rename changes the display name.
## TUI
/new opens another session; /new kill closes the current one. /resume opens the session hub. /cd changes the working directory and /adddir adds a workspace root. /rename changes the name. Use /quit for the working-aware quit confirmation.
## Recovery
A stopped session can be resumed. Deletion cannot be undone. Switching sessions cancels session-bound guidance. Remote history belongs to the remote host.
