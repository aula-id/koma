# Chat and composer

## GUI
Attach a session to chat. Enter sends; Shift+Enter adds a newline. Use the composer Stop button during a running turn. Mode, model and effort pickers affect the active session. A draft starting with ! runs a local shell command only while idle with no staged attachments; otherwise it is ordinary message text. File search inserts @ references. Use the attachment controls for supported files; large pasted content can be stored as a paste attachment.
## TUI
Use /mode, /model, /effort, /attach and /internet. The command and keyboard reference comes from the command registry. /clear clears the live conversation but keeps the archive; /compact summarizes context.
## Recovery
If sending fails, inspect the current provider/model and Notifications. Do not resend a destructive request without checking its result. Help has an independent koma-free conversation and cannot execute work.
