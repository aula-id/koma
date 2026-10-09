# Coding, files and saves

## GUI
Coding has the workspace tree and Monaco editor tabs. Open a workspace file from the tree, edit it, and use Save or Ctrl+S. A dirty marker means edits remain unsaved. Saves accepted by the host cannot be cancelled by closing a tab; edits made during a save remain dirty until saved. Resolve unsaved-change prompts yourself. Autosave, if enabled, writes automatically according to its settings.
LSP uses the configured server or a server available on PATH. The footer Language Servers drawer and Problems show status and diagnostics. Diff tabs are syntax-only. Explorer lists file changes, Plan, agent jobs and bash jobs; change rows open diffs.
## TUI
/cd and /adddir control workspace roots. Agent file tools operate in allowed roots; TUI does not have Monaco editor tabs.
## Recovery
For save errors inspect the notification, host connection, permissions and current root. A conflict or external change needs review before overwriting.
