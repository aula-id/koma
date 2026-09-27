# Workspace debugger

Open **Debug: Launch, Attach and Inspect** in the command palette. F9 or a click in
the editor glyph gutter toggles a persistent host/workspace breakpoint. The panel
provides conditional breakpoints, exception filters, launch profiles, multiple sessions, pause,
continue, stepping, threads, stack, variable trees, watch expressions and output.
Breakpoint lines follow editor changes, and updates are serialized per source.
Watch expressions persist per host/workspace. The selected paused frame has an
editor line marker; the console accepts expressions while paused.
Click a stack frame to open its source on the currently selected host. Stop for
an attach profile requests detachment without terminating the attached program.
Closing the panel or switching chats does not stop a session.

Python and Node current-file profiles are provided; Go projects also get a package
profile. Save your source before starting. More profiles live in `debug` in
`.koma/coding.json`, for example:

```json
{
  "version": 1,
  "debug": [
    {
      "id": "native",
      "label": "Debug application",
      "adapter": "lldb-dap",
      "request": "launch",
      "preLaunchTask": "cargo:build",
      "configuration": {"program": "${workspaceFolder}/target/debug/app"}
    },
    {
      "id": "python-attach",
      "label": "Attach Python",
      "adapter": "debugpy",
      "request": "attach",
      "configuration": {"connect": {"host": "127.0.0.1", "port": 5678}}
    }
  ]
}
```

Managed adapter names: `debugpy`, `js-debug`, `delve`, `lldb-dap`, `php-debug`,
`bash-debug`, `lua-debug`. Adapter-specific launch fields belong in
`configuration`. `${workspaceFolder}` and `${file}` expand on the selected host.
`preLaunchTask` names a configured or discovered task. Launch waits for it to
succeed; Stop cancels preparation, and its output is available from the terminal
button. Optional `exceptionFilters` lists the adapter filter IDs to enable at
startup; absent configuration uses the advertised defaults.
An external adapter uses `command`, `args`, and optional `tcp: true`; `{port}` in
its arguments is replaced with a loopback port. No adapter is started by reading
settings. Installation is provided by Language Packs where supported.

DAP uses bounded Content-Length frames, bounded retained events and timed requests.
Initialization waits for `initialized`, sends breakpoints, and sends
`configurationDone` when advertised before awaiting launch completion. References
from scopes/variables/stack/watch are invalidated after resume. Up to eight live
sessions and 32 retained session summaries are held per native host process.
Integrated `runInTerminal` requests use workspace-owned PTY tasks. Open their
input/output with the debugger terminal button. External terminal and shell-
interpreted argument requests are explicitly rejected. Terminal cwd must stay
inside the workspace, and launch-session teardown stops its terminal tasks. Unix remote sessions use the persistent coding service and can be adopted after
SSH reconnect; Windows remote workers retain their original connection lifecycle.

Native acceptance (not executed by the implementation agent): each managed adapter
on each supported OS, breakpoints set before/after launch, condition rejection,
step/continue with slow replies, two simultaneous roots/hosts, attach/detach,
adapter crash, child cleanup, and source paths containing non-ASCII characters.
