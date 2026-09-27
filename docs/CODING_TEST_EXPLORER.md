# Test Explorer

Open **Tests: Discover, Run and Debug** in the command palette. Choose a workspace
and runner, then explicitly Discover or Run. Discovery may import or compile
project code. Results are grouped by source file or suite and expose selection,
run all, run file/suite, cancel, rerun failed, source navigation, failure details
and retained output in Tasks. Native supervision survives panel/chat changes.

Built-ins are pytest (project-selected Python), Go, Cargo and Node's own test
runner. Jest/Vitest and other frameworks need a project runner profile. Pytest
uses its collection/report hooks; Go uses `go test -json` and slash-aware exact
subtest filters. Cargo builds with JSON artifact messages, gives each target and
test a distinct identity, then invokes the matching libtest binary. Source links
point to the target entry file. Rust doctests and custom test harnesses use normal
Cargo tasks rather than this binary explorer.

Node discovery lists files without importing them; case results appear on execution.
Later case runs use full ancestor names and the entry file. Identical full test names
in one Node entry file still share the runner's name filter. Use a current Node LTS
for nested test selection and entry-file metadata.

Python tests launch through debugpy, Go through Delve test mode, Node through
js-debug, and discovered Cargo binaries through lldb-dap. Rust Debug uses the last
built test binary; Discover again after source changes to rebuild it. Other runners
may reference an existing debug profile. C/C++ debug profiles name their prepared
executable.
Project-specific test dependencies remain in the project's environment.

A custom `tests` entry in `.koma/coding.json` has `id`, `label`, `kind: "json"`,
`command`, `args`, `discoverArgs`, and optional `debugProfile`. The command writes
newline-delimited `KOMA_TEST {json}` records on stdout with `id`, optional `label`,
`file`, one-based `line`, `suite`, `status`, `durationMs`, and `message`. Statuses
are `discovered`, `running`, `passed`, `failed`, `skipped`, `notRun`. Selected IDs are
appended as separate arguments for Run; no shell interpolation occurs. Ordinary
stdout/stderr remains available as task output. Built-in profiles can be overridden
by ID with a command/arguments suitable for project tooling.

Retained results are limited to 5,000 items and 4 MiB per run, with explicit
truncation. Task output keeps its separate bounded buffer. Test history retains
64 runs per native process. Selected multiple command invocations continue after
a failing test; the overall run remains failed. Cancellation stops later commands.

Native acceptance belongs to the user: discover/run/fail/cancel/rerun, collection
failure, teardown failure, multiple workspaces/hosts, slow output, large suites,
adapter debug paths, and remote disconnection. Parser regression sources compile
with `cargo check --features gui --tests`; they have not been executed here.

Primary protocol references: [pytest hooks](https://docs.pytest.org/en/stable/reference/reference.html),
[Go test2json](https://pkg.go.dev/cmd/test2json),
[Node test reporters](https://nodejs.org/api/test.html#test-reporters).
