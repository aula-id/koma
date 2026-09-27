# Workspace debugger

Open **Debug: Launch, Attach and Inspect** in the command palette. F9 or a click in
the editor glyph gutter toggles a persistent host/workspace breakpoint. The panel
provides conditional breakpoints, launch profiles, multiple sessions, pause,
continue, stepping, threads, stack, variable trees, watch expressions and output.
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
An external adapter uses `command`, `args`, and optional `tcp: true`; `{port}` in
its arguments is replaced with a loopback port. No adapter is started by reading
settings. Installation is provided by Language Packs where supported.

DAP uses bounded Content-Length frames, bounded retained events and timed requests.
Initialization waits for `initialized`, sends breakpoints, and sends
`configurationDone` when advertised before awaiting launch completion. References
from scopes/variables/stack/watch are invalidated after resume. Up to eight live
sessions and 32 retained session summaries are held per native host process.
Reverse `runInTerminal` requests are explicitly unsupported in this increment;
use an adapter's internal console. Debug processes currently end with the coding
worker, so network-loss adoption is a separate milestone.

Native acceptance (not executed by the implementation agent): each managed adapter
on each supported OS, breakpoints set before/after launch, condition rejection,
step/continue with slow replies, two simultaneous roots/hosts, attach/detach,
adapter crash, child cleanup, and source paths containing non-ASCII characters.
