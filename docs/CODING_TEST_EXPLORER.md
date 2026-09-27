# Test Explorer

Open **Tests: Discover, Run and Debug** in the command palette. Choose a workspace
and runner, then explicitly Discover or Run. Discovery may import or compile
project code. Results are grouped by source file or suite and expose selection,
run all, run file/suite, cancel, rerun failed, source navigation, failure details
and retained output in Tasks. Native supervision survives panel/chat changes.

Built-ins are pytest (project-selected Python), Go, Cargo and Node's own test
runner. Jest/Vitest and other frameworks need a project runner profile. Pytest
uses its collection/report hooks; Go uses `go test -json`; Cargo uses stable test
listing and text results; Node uses its custom reporter interface. Node discovery
lists files without importing them; case results appear on execution. Node case
selection currently reruns the containing file, including for debug. Cargo names
are exact-filtered but duplicate names across separate test binaries are not yet
separate explorer identities. These limitations require follow-up before claiming
full framework parity. Go subtests require additional filter handling.

Python tests launch through debugpy, Go through Delve test mode, Node through
js-debug. Other runners may reference an existing debug profile. Compiled Rust
and C/C++ test debug profiles must currently name their prepared executable.
Project-specific test dependencies remain in the project's environment.

A custom `tests` entry in `.koma/coding.json` has `id`, `label`, `kind: "json"`,
`command`, `args`, `discoverArgs`, and optional `debugProfile`. The command writes
newline-delimited `KOMA_TEST {json}` records on stdout with `id`, optional `label`,
`file`, one-based `line`, `suite`, `status`, `durationMs`, and `message`. Statuses
are `discovered`, `running`, `passed`, `failed`, `skipped`. Selected IDs are
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
