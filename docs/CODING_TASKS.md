# Project tasks

Open **Tasks** in the footer, or use **Tasks: Run Task…**, **Tasks: Build…**,
**Tasks: Test…**, or **Tasks: Show Output and Running Tasks** in the command
palette. Choose the workspace and task, inspect the command, and press Run.
The panel uses Koma's existing visual tokens and leaves the main workspace
dropdown and editor panes intact. Escape or Close hides the panel; processes
continue. The output selector retains previous runs, with status and exit code.

The panel keeps its own explicit host/workspace selection. Switching the main
workspace, chat session, or connected host does not retarget or stop existing
tasks. Its dropdown remembers visited workspaces so you can inspect and stop
runs from a previously selected host. Remote tasks require an updated Koma
binary and a previously established connection to that host.

## Configuration

The configuration button opens `.koma/coding.json` in the existing editor.
Add `tasks` to its top-level object and save. Refresh the panel after editing.
The native service rejects a Run request if task definitions have changed
since they were displayed. It never executes a project task on opening a
workspace or reading its settings.

```json
{
  "version": 1,
  "tasks": [
    {
      "id": "dev",
      "label": "Development server",
      "group": "run",
      "command": "npm",
      "args": ["run", "dev"],
      "cwd": "web",
      "env": { "PORT": "3000" }
    },
    {
      "id": "build",
      "label": "Build agent",
      "group": "build",
      "command": "cargo",
      "args": ["build", "-p", "agent"],
      "timeoutMs": 600000
    },
    {
      "id": "test",
      "label": "Python tests",
      "group": "test",
      "command": ".venv/bin/python",
      "args": ["-m", "pytest"],
      "env": { "PYTHONUNBUFFERED": "1" }
    }
  ]
}
```

`id`, `label`, and `command` are required. IDs must be unique within a project.
`group` defaults to `run`; `build` and `test` provide the other panel filters.
`cwd` defaults to `.` and must resolve to a directory inside the workspace,
including when symlinks are involved. Arguments default to an empty array.
Unknown task fields are rejected, making misspelled execution options visible.

Commands use a program and an argument array. Koma does not interpolate shell
expressions, environment variables, or `${workspaceFolder}`. A relative program
path containing a path separator resolves against the task's working directory;
a bare program name uses the host's executable lookup. Tasks inherit Koma's
process environment plus project/toolchain environment and the task's `env`
overrides. `envRemove` lists inherited keys to remove. Examples assume their tools and working
directories already exist.

For a shell expression, explicitly select a shell and pass its arguments, e.g.
`"command": "/bin/sh", "args": ["-c", "make && ./app"]`. For Windows npm
installations that provide `npm.cmd`, use an explicit Windows command such as
`"command": "cmd.exe", "args": ["/d", "/c", "npm run dev"]`. The displayed
command is run on the selected host. Paths and tools must match that host.

Tasks use **saved files**. Run does not save editor buffers automatically.
Use Save or Save All first when needed. Task configuration values, including
`env`, live in the project file; avoid committing credentials there.

## Process and output behavior

- Task processes belong to the native coding service, independently of chat.
  Starting a process returns promptly; polling its output does not wait for it
  to finish. One task ID can run once per workspace at a time.
- Stop force-terminates the Unix process group or Windows Job Object. Windows
  processes start suspended until assigned to their Job Object. When the parent
  exits, remaining descendants in the group/job are terminated as well. Programs
  that deliberately detach themselves from the process group are unsupported.
- Optional `timeoutMs` is between 100 milliseconds and 24 hours, measured with a
  monotonic clock. A timeout stops the task and records a failed result.
- Standard input is closed by default. `interactive: true` uses an owned PTY
  with input and resize (see below). Pipes capture stdout/stderr independently,
  so ordering between streams is approximate. Some programs buffer piped output.
- Up to eight tasks/captures are active per coding service. Up to 64 run records
  are retained across its workspaces; completed records are evicted first. Each
  run retains up to 1 MiB/2048 output chunks, fetched in pages of at most 64 chunks.
  The UI also bounds retained output and reports discarded earlier output.
- There are at most 100 configured tasks, each at most 64 KiB. The whole project
  configuration still has the existing 1 MiB limit.
- Run history/output is in memory. Closing Koma stops local tasks. Unix SSH
  tasks belong to a persistent remote service and can be adopted after reconnect;
  Windows SSH workers stop on EOF. No host-reboot or daemon-crash durability is
  claimed. Check the outcome after a transport error; commands are not retried.
- Task Runs, Output, and Stop do not require the workspace directory or config to
  remain readable. A deleted root or malformed config must not block Stop.

Language packs and Test Explorer share the same supervisor. Debugger integrated
terminals and pre-launch tasks appear here too. Custom problem-matcher definitions
are not supported; common compiler/source locations are recognized automatically.

## Native review

Functional execution is left to the user. Suggested acceptance cases:

1. Configure success, nonzero-exit, missing-executable, and long-running tasks.
   Check stdout/stderr, exit codes, timeout, Run, Stop, and repeated Stop.
2. Start tasks with the same ID in two roots. Switch the main dropdown, chat
   session, and host while they run. Select their original workspace in Tasks;
   output and Stop must still address the original run. Close/reopen the panel.
3. Try Run twice rapidly. Change the saved command while the panel is open:
   Run must reject the stale definition until Refresh. Invalid JSON, duplicate
   IDs, unknown fields, and a cwd outside the root must report errors.
4. Run a command that spawns a child. Stop it and verify descendants stop too.
   Also verify cleanup when its parent exits first, Koma closes, and a remote
   Windows worker reaches EOF; Unix remote jobs should remain adoptable. Repeat on Linux, macOS, and Windows.
5. Exercise Unicode split across writes, mixed stdout/stderr, very noisy output,
   Follow on/off, selection/copy, and reopening an old run after truncation.
   The panel must remain responsive and clearly report discarded output.
6. Delete/rename a running workspace or corrupt its config. Output/Stop must
   remain usable. Disconnect SSH and verify errors are shown without rerunning
   the command. On Unix reconnect, verify adoption without duplicate execution.
7. Keep an unsaved editor change, run a task, and confirm it uses saved disk
   content. Check the panel's appearance and keyboard focus in the native app.

Regression sources in `coding::tasks::tests` cover stale configuration,
ambiguous definitions, split Unicode, workspace identity, literal arguments,
exit status, duplicate execution, Stop, deadlines, descendant pipes, and output
pagination/truncation. They are supplied for execution by the user.

## Discovery and dependencies

The task selector discovers package.json scripts, Cargo, Go, pytest and Make entry
points when their project files are present. Explicit tasks with the same ID take
precedence. Discovery does not execute project commands. `dependsOn: ["task-id"]`
adds sequential prerequisites; cycles and missing IDs are rejected. Shared
prerequisites run once per invocation. A failed or canceled prerequisite prevents
later steps. The task output identifies each step. Timeouts apply per step.

`continueOnError: true` allows a task step to finish unsuccessfully while later
steps run; the final task still reports failure. Test Explorer uses this for
selected cases that need separate commands. Ordinary dependency steps default
to stopping on the first failure.

## Interactive tasks and source locations

Set `interactive: true` on a task to give it a real PTY, keyboard input and terminal
resize. Tasks uses the same terminal palette/font as Koma's existing terminal.
Closing the panel does not stop the task. Opening its run replays retained terminal
output; very old screen state cannot be reconstructed after buffer truncation.
Input is bounded and queued separately so a program that stops reading does not
block Stop. Large pastes may report backpressure and should be retried in smaller
pieces, not automatically duplicated.

```json
{"id":"shell","label":"Project shell","command":"bash","interactive":true}
```

Use a suitable executable such as `pwsh` on Windows. Unix tasks own a session /
process group; Stop terminates their foreground and original process groups before
reaping. Windows ConPTY launches a gated helper, assigns it to a kill-on-close job
object, and only then releases the real command. This avoids a child-spawn race
before job assignment. Native platform execution remains an acceptance requirement.

Common GCC/Clang, Rust, TypeScript and Python output locations are listed above
output as clickable source links. Paths are resolved against the task's workspace
and working directory; links require that host to be active. The text is never
interpreted as HTML. Up to 200 distinct retained locations are shown.
