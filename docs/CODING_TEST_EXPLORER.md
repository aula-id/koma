# Test Explorer

Open **Tests: Discover, Run and Debug** in the command palette. Choose a workspace
and runner, then explicitly Discover or Run. Discovery may import or compile
project code. Results are grouped by source file or suite and expose selection,
run all, run file/suite, cancel, rerun failed, source navigation, failure details
and retained output in Tasks. Native supervision survives panel/chat changes.

Built-ins are pytest (project-selected Python), Go, Cargo, Node's own test runner,
Jest, Vitest, PHPUnit and CTest. Pytest
uses its collection/report hooks; Go uses `go test -json` and slash-aware exact
subtest filters. Cargo builds with JSON artifact messages, gives each target and
test a distinct identity, then invokes the matching libtest binary. Source links
point to the target entry file. A separate Cargo doctests profile uses
`cargo test --doc`, listing and parsing its libtest-style output. Custom harnesses
still require a configured task/JSON adapter.

Node discovery lists files without importing them; case results appear on execution.
Later case runs use full ancestor names and the entry file. Identical full test names
in one Node entry file still share the runner's name filter. Use a current Node LTS
for nested test selection and entry-file metadata.

Python tests launch through debugpy, Go through Delve test mode, Node through
js-debug, and Cargo tests through lldb-dap. Rust Debug rebuilds and resolves the
selected target/test identity before launch; a failed/canceled build prevents
launch. It uses saved source files. Jest/Vitest debug their project runner with
js-debug. A custom framework command requires an explicit debugProfile. Other runners
may reference an existing debug profile. C/C++ debug profiles name their prepared
executable.
Project-specific test dependencies remain in the project's environment.

## Additional built-in frameworks

| Kind | Automatic detection | Discovery and results |
| --- | --- | --- |
| `jest` | package.json dependency/devDependency | File discovery via `--listTests --json`; cases/results via Jest's JSON report |
| `vitest` | package.json dependency/devDependency | `list --json` cases; `run --reporter=json` results |
| `phpunit` | phpunit.xml, phpunit.xml.dist or vendor/bin/phpunit | `--list-tests`; JUnit results via `--log-junit` |
| `ctest` | CTestTestfile.cmake at root or build/ | `--show-only=json-v1`; `--output-junit` results |
| `cargo-doc` | Cargo.toml | Cargo doctest list/run output |

Jest/Vitest use the project's node_modules runner and selected Node executable;
PHPUnit uses vendor/bin/phpunit with selected PHP. CTest detects `build/` as its
test directory when appropriate. Configure a profile for other package layouts,
build directories or workspace packages. No dependency installation runs on
discovery. CTest needs version 3.21+ for JUnit output; Vitest must support `list
--json`. Collection can compile/import project code. Framework reports/listings
are bounded to 32 MiB and malformed/missing reports are visible task failures.

Selected cases use escaped name filters and their file where supported. Identical
full names within a file, or the same JS file run in multiple framework projects,
may share a filter and result identity; use separate configured profiles to
separate those projects. PHPUnit/CTest/doctests use an explicit debugProfile when
debugging is needed. Framework flags remain part of `args`, e.g. CTest
`["--test-dir", "out/debug"]`.

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
[Node test reporters](https://nodejs.org/api/test.html#test-reporters),
[Jest CLI](https://jestjs.io/docs/cli),
[Vitest CLI](https://vitest.dev/guide/cli.html),
[PHPUnit CLI](https://docs.phpunit.de/en/12.5/textui.html),
[CTest CLI](https://cmake.org/cmake/help/latest/manual/ctest.1.html).
