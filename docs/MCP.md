# MCP support

koma is an MCP client only. Each enabled server contributes its tools to the model as ordinary function-calling tools named `mcp__<server>__<tool>`. There is no koma MCP server, no resources/prompts support (non-text results are flattened to `[image content]`, `[embedded resource]` and similar markers), no legacy SSE transport and no OAuth. A server that needs a credential gets it from `env` or `headers`.

Code lives in `src-agent/src/app/mcp/`; the config types are in `src-agent/src/model/app_config.rs`; the `koma mcp` runner is `src-agent/src/app/runtime/manage/mcp_cli.rs`.

## Layers

`McpManager` (`app/mcp/mod.rs`, `connect.rs`) is a facade over two backends:

| Backend | Owns connections | Built by | Used by |
|---|---|---|---|
| `Local` | yes, `rmcp` services on the tokio runtime | `McpManager::connect_all` | the `--local` TUI, the fallback below, the project overlay |
| `Proxy` | no, forwards to the global daemon | `McpManager::connect_proxy` | session daemons |

The global MCP daemon (`koma --mcp-daemon`, `app/runtime/mcp_daemon.rs`, socket `~/.koma/mcp.sock`) owns one `Local` manager so N session daemons share one copy of every server. Its request protocol, reaper and lifecycle are in [ARCH_DAEMON.md](ARCH_DAEMON.md#global-mcp-daemon). `ensure_mcp_manager` (`app/runtime/actions/mcp.rs`) builds the session manager: it does nothing when no server is enabled, otherwise ensures the daemon and connects a proxy, and falls back to `connect_all` if either step fails. `Proxy` serves `tool_defs` and `tool_names` from a cache and does a connect-per-call to the socket for `Call` and `Status`. The proxy read budget is the longest enabled `tool_timeout` plus 5 s, never under `PROXY_IO_TIMEOUT` (65 s).

Connecting is non-blocking. Each server connects on its own task; the whole connect (spawn or HTTP setup, `initialize`, first `tools/list`) is one timeout. Results are guarded by a snapshot `generation` (bumped by `reconnect`) and a per-server epoch (bumped by `restart_server`), so a connect that finishes after its config was replaced is discarded. If two server names sanitize to the same `<server>` segment, the second one connecting is skipped and the error log names both; rename one.

Config changes go through `save_and_reload_mcp`: save `config.json`, then `reconnect` on a throwaway thread (a `Proxy` reconnect blocks on socket round-trips), then `refresh_project_mcp`.

Extension tools (`contributes.tools`) and extension-owned MCP servers (`ext_id` set on the entry) share the same naming and manager but extension tools execute in the session daemon through `ExtHostManager`, never in the global daemon. See [EXTENSIONS.md](EXTENSIONS.md).

## Server entry

`McpServerEntry` is a row of `mcp_servers` in `~/.koma/config.json`. Every field has a serde default, so an old or partial entry loads.

| Field | Default | Meaning |
|---|---|---|
| `uuid` | fresh v4 (`new_uuid`) | identity; the manager keys connections on it |
| `name` | `""` | display name and source of the tool prefix |
| `enabled` | `false` | disabled servers are never connected |
| `transport` | `stdio` | `stdio` or `http` (snake_case) |
| `command`, `args` | `""`, `[]` | stdio: program and arguments |
| `env` | `[]` | stdio: list of `[key, value]` pairs, not a map, so order round-trips |
| `url` | `""` | http: streamable-HTTP endpoint |
| `headers` | `[]`, omitted when empty | http: request headers as pairs |
| `cwd` | `None` | stdio working directory |
| `startup_timeout_ms` | `None` = 20 s | connect budget |
| `tool_timeout_ms` | `None` = 60 s | per tool call |
| `allowed_tools`, `disabled_tools` | `[]` | glob lists, see below |
| `ext_id` | `None` | extension that registered the row |

Fields of the other transport are ignored. The tool prefix is `sanitize_server_name`: lowercase, every run of characters outside `[a-z0-9_]` becomes one `_`, edges trimmed, empty becomes `server`. `My Server` + `echo` is `mcp__my_server__echo`.

Timeouts: unset or `0` gives the default, anything else is clamped to 1 s..10 min (startup) or 1 s..1 h (tool call) in `app/mcp/util.rs`. `allowed_tools`, when non-empty, must match the tool's original name; `disabled_tools` is applied afterwards and wins. Patterns are `globset` globs; a pattern that does not compile only matches by equality.

## Transports

- stdio: spawns a child through `rmcp` `TokioChildProcess`, stderr piped. The last 2048 bytes of stderr (`STDERR_TAIL_BYTES`) are appended to a failed connect error.
- http: `rmcp` `StreamableHttpClientTransport` with `headers` as custom headers, over native TLS (`reqwest-native-tls`, features in `src-agent/Cargo.toml`). Header values are never echoed in errors.

The importers accept `type: "sse"` but map it to streamable HTTP with a warning; `koma mcp add -t sse` does the same.

## Variable expansion

`app/mcp/expand.rs` runs at connect time on an in-memory copy, so the stored text keeps `${...}`. It covers `command`, `args`, `env` values, `url`, `headers` values and `cwd`; names (env keys, server name) are never expanded. Forms: `${NAME}`, `${env:NAME}`, `${NAME:-default}` (default used when unset or empty; the default text is not itself expanded). `NAME` must match `[A-Za-z_][A-Za-z0-9_]*`. An unset name without a default stays literally in the text and is logged as not set. Malformed or unterminated `${` is kept as is.

## Command resolution

`app/mcp/resolve.rs` is pure so the Windows rules test on any OS (`windows` is a parameter).

- The PATH searched is the entry's own `PATH` in `env` (case-insensitive on Windows) or else `coding::environment::host_path()`. The child gets the host PATH first, then the entry's `env`, so a configured `PATH` wins.
- A command with a separator (or a drive letter on Windows) is a location: absolute as is, relative joined to the server `cwd`. A bare name is searched in each PATH directory.
- Windows tries only `.com`, `.exe`, `.bat`, `.cmd` taken from `PATHEXT` in that order (other `PATHEXT` entries are ignored; the four are the default when unset). The bare name is tried first only when it already has an extension. Windows always spawns the resolved path and fails with a "not found on PATH" message when nothing resolves.
- Unix keeps a bare name for std to look up, and uses the resolved path when the command has a separator.
- Spawn errors are rewritten by `describe_spawn_error`. On Windows an `InvalidInput` for a `.cmd` or `.bat` means an argument cannot be passed safely to the batch file (`%`, `"`, newlines); the fix is the real executable, for example `node.exe` with the script path.

`resolve_for_doctor` runs the same resolution without spawning, for `koma doctor`.

## User scope and project scope

User scope is `mcp_servers` in `config.json`. Project scope is `<session workdir>/.mcp.json`, read by `refresh_project_mcp` when a session starts, when the config reloads, and after an approval change.

Accepted shapes (`parse_servers_json`, shared by the file, the GUI import and the CLI): `{"mcpServers": {...}}` (Claude Code, Cursor), `{"servers": {...}}` (VS Code), a bare `{name: {...}}` map, a single server object (needs a name), and a README fragment `"name": {...}` without outer braces. Each server decodes on its own; a bad one becomes a warning. Per-server keys are camelCase: `type` (`stdio`, `http`, `streamable-http`, `sse`), `command`, `args`, `env` and `headers` as maps, `url`, `cwd`, `disabled`/`enabled`, `startupTimeoutMs`, `toolTimeoutMs`, `allowedTools` (alias `enabledTools`), `disabledTools`. Without `type`, a `url` and no `command` means http.

Limits on the file: 1 MiB, and it must not resolve outside the project root (a symlink out gives a layer carrying only an error). Writes apply the same checks.

Import into the user list (`import_into`) replaces a user row with the same name in place and keeps its uuid, and skips a name owned by an extension with a warning.

Override rule: an active project server (approved and not disabled) hides every user server whose sanitized name equals its own, for that session only (`hidden_prefixes` strips `mcp__<name>__` tools from the base set). The user config is untouched. Project servers never go to the global daemon, since it serves one user set to every session. They run in a private `Local` manager (`ProjectOverlay`); a session with approved project servers but no user servers gets an empty local base manager and no daemon is spawned.

Approval gate. A `.mcp.json` row is forced to `enabled = false` until the user approves it. An approval is `McpProjectApproval { root, name, fingerprint }` in `config.json` under `mcp_project_approvals`, keyed by the canonicalized root (`root_key`) and the name. `fingerprint` is the sha256 hex of a JSON of transport, command, args, env, url, headers and cwd as written in the file, before `${}` expansion and before the cwd default. It excludes name, enabled, timeouts and tool lists. States: `approved` (fingerprint matches), `pending` (no record), `changed` (record with another fingerprint). Editing anything that decides what runs therefore needs a new approval; revoking removes the record. A stdio project server runs in the project root unless it names a `cwd` (relative is joined to the root; one starting with `${` or absolute is kept). The uuid is a v5 of root and name, stable across reloads.

## Secrets and masking

`AppConfig::for_each_secret` seals `env` values and `headers` values on disk with the other credentials (`SecretId::Mcp`, `SecretId::McpHeader`). Project files are plain text and meant to be committed, so use `${VAR}` there; `koma mcp add --scope project` warns when a literal secret is written.

Toward the webview, header values never leave the daemon: `mask_headers` (`app/mode/mcp/state.rs`) replaces every value with `HEADER_KEEP` (`********`), and on save a blank or unchanged `********` takes the stored value of the same-named header. `env` values are sent as is (`env_pairs`). The CLI masks differently (`mcp_cli.rs`): `list` and `get` print `***` for any env or header value except a pure `${VAR}` reference, and `scrub_url` replaces URL userinfo and query with `***`.

## CLI

`koma mcp` runs without a TUI or a session (short-circuit in `main.rs`). Parsing is `app/mcp/cli.rs`; `execute` is pure over the config and cwd. Everything after a literal `--` is the server command, verbatim.

```
koma mcp list [--json]
koma mcp get <name>
koma mcp add <name> [--scope user|project] [-t stdio|http] [-e KEY=VALUE] [-H 'Name: value'] [--cwd DIR] [--disabled] -- <command|url> [args...]
koma mcp add-json <name> '<json>' [--scope user|project]
koma mcp remove <name> [--scope user|project]
koma mcp approve <name>|--all [--revoke]
```

```
koma mcp add github -t http -H 'Authorization: Bearer ${GITHUB_TOKEN}' -- https://api.githubcopilot.com/mcp/
koma mcp add fs --scope project -- npx -y @modelcontextprotocol/server-filesystem .
koma mcp add-json search '{"command":"uvx","args":["mcp-search"],"env":{"KEY":"${SEARCH_KEY}"}}'
koma mcp approve --all
```

Notes from the code: scope defaults to `user`; a transport defaults to http only for a single `http(s)://` argument; `--env` is stdio only and `--header` http only; `add` refuses an existing name while `add-json` replaces; removing an extension-owned row is refused; `approve` and `list`/`get` read `./.mcp.json` of the current directory; removing a project server also drops its approval. Exit code 2 prints the usage, 1 is an error. After a change the CLI saves `config.json`, asks running sessions to reload (`broadcast_reload_global_catalogue_report`) and, when the global daemon is alive and the user list changed, sends it `Reconnect`.

## Surfaces

- GUI: `src-webgui/src/components/panels/McpPanel.tsx` with `mcp/McpListView.tsx` (user and project lists, approve/revoke, restart), `McpEditView.tsx`, `McpImportView.tsx` and the draft/request mapping in `mcp/mcpForm.ts`. Requests: `SetMcpServer`, `DeleteMcpServer`, `EnableMcpServer`, `ImportMcpJson`, `RestartMcpServer`, `ApproveMcpProjectServer` (`app/runtime/gui/proto.rs`, `gui/dispatch.rs`; handlers in `event_loop/daemon/hub/requests_config.rs`). Live status arrives in the `McpStatus` push envelope, cached by `server_status_cached` for 2 s. See [ARCH_GUI_DAEMON.md](ARCH_GUI_DAEMON.md).
- TUI: `/mcp` opens the manager; the draft state is `src-agent/src/app/mode/mcp/` and the view is `mcpView` in `src-term/src/modes/manage.tsx`. A TUI edit keeps headers, cwd, timeouts and tool lists it cannot show.

## Tests

From the repository root (the package is `agent`):

```
cargo test -p agent mcp
```

That matches `app::mcp::{expand,resolve,util,project,activation}_tests`, `manage::mcp_cli_test`, `app::mode::mcp::state_tests` and `mcp_serde_tests` in `app_config`. Narrow with a module path, for example `cargo test -p agent app::mcp::resolve_tests`. The GUI form logic is `src-webgui/src/components/panels/mcp/mcpForm.vitest.test.ts`:

```
cd src-webgui && npx vitest run --project logic mcpForm
```
