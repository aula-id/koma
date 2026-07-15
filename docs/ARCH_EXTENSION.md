# Extension architecture and communication contract

> **Status: v0 / unstable.** This is an implementation contract, not a frozen public API. The wire types in `src-extension/src/protocol.rs` and the host implementation are authoritative. In particular, oneshot lifecycle (the host still runs the full duplex serve loop rather than a strict one-request-then-exit cycle) and automatic restart after an unexpected exit are not complete features yet. Panel and model/provider runtime wiring — once the gaps this note used to describe — are now live: see `docs/EXTENSIONS.md`'s "Panel bridge" and `models:contribute` grants-reference sections, both verified against the same broker/hub source this document cites. Sections 2 and 8 below cover only the manifest shape and the original `agents.*` broker contract in low-level wire detail; `docs/EXTENSIONS.md` is the up-to-date, complete capability reference for everything landed since (the full 8-grant surface, events, the panel bridge, delegated OAuth) and is the doc to read first — come back here for byte-level framing/timeout/handshake detail it doesn't restate.

## 1. Scope and actors

The actors and ownership boundaries are:

| Actor | Owns | Does not own |
|---|---|---|
| **Extension process** | Its executable, manifest, contribution implementation, and SDK duplex client | Agent/session state, grants, installation, GUI host DOM |
| **`src-agent` extension host/broker** | Per-extension process, Unix socket (named pipe on Windows), handshake, framing, pending calls, generation, grant checks, contribution invocation | The iframe UI; the web store UI |
| **Session daemon** | `AppState`, active session, sub-agents, `AppConfig`, MCP registrations, client/daemon IPC | Extension process code and webview DOM |
| **`src-webgui` host** | Wry window, `GuiReq` dispatch, host-local work, daemon client relay, `PushEnvelope` delivery | Extension agent grants and extension socket protocol |
| **Extension iframe** | Static installed `ui/` document and its local JS state | Host DOM, host JS globals, `window.ipc`, agent/session authority |

One extension has one child and one live socket connection. A daemon extension is intended to stay connected; an oneshot is intended to be spawned for an invocation, but the current manager does not boot-start oneshots and the per-invocation lifecycle is not fully wired. There is no automatic restart after an unexpected exit.

## 2. Manifest JSON contract

`ExtensionManifest` is deserialized with serde from `manifest.json` and sent again inside `Hello`. Required fields have no serde defaults; defaulted fields may be omitted. Enum values are lowercase strings.

| JSON field | Rust type | Required/default | Serialization and meaning |
|---|---|---|---|
| `schema` | `String` | required | String; the current schema constant is `koma-extension/v0` (`MANIFEST_SCHEMA`). Consumers should use that value; manifest parsing alone does not make this a general version-negotiation mechanism. |
| `id` | `String` | required | String. Install/UI guards allow non-empty ASCII `[A-Za-z0-9._-]`, at least one alphanumeric, and no leading/trailing dot. |
| `name` | `String` | required | String. |
| `version` | `String` | required | String. |
| `description` | `String` | omitted => `""` | String; `#[serde(default)]`. |
| `tier` | `Tier` | required | `"free"` or `"paid"`. |
| `kind` | `ExtensionKind` | required | `"daemon"` or `"oneshot"`. |
| `runtime` | `Runtime` | required | Object described below. |
| `contributes` | `Contributes` | omitted => empty object | Object; `#[serde(default)]`. Empty arrays are omitted when serializing. |
| `requires` | `Vec<Grant>` | omitted => `[]` | Array; `#[serde(default)]`. Eight wire values today: `"agents:read"`, `"agents:orchestrate"`, `"sessions:manage"`, `"chat:prompt"`, `"models:invoke"`, `"context:publish"`, `"oauth:contribute"`, `"models:contribute"` — see `docs/EXTENSIONS.md`'s grants reference for what each unlocks. |
| `workspace_dir` | `Option<String>` | omitted => none | String, e.g. `"~/.event-watcher"`. An extension-owned state directory koma creates and injects as an extra workspace root of every session (must resolve strictly under `$HOME`; a path failing validation is logged and skipped, never blocks startup). See `docs/EXTENSIONS.md`'s `workspace_dir` section for the full validation rule set. |

`runtime` is `{ "exec": String, "args": Vec<String> }`; `exec` is required, relative to the package root, and `args` defaults to `[]` and serializes as an array. `contributes` contains arrays (all default `[]`, all empty arrays skipped on serialization):

| Contribution array | Item JSON fields and types | Current integration status |
|---|---|---|
| `sub_agents` | `{name: String, description: String, prompt?: String, model?: String, effort?: String, tools?: [String]}` | **Integrated:** merged on the next `AgentRegistry::load` while enabled. `model` resolves through the ext-owned-first slug binding chain in `src-agent/src/app/resolve.rs::resolve_agent()` — see `docs/EXTENSIONS.md`'s manifest reference for the full 5-step order. `tools` is a seed allow-list narrowing koma's own built-in tools (omitted/empty = the safe read-only default); it never affects MCP tool availability, which every sub-agent inherits automatically regardless of this field. |
| `models` | `{id: String, display_name: String}` | **Declared shape only** — the runtime catalogue is built through the `models:contribute` grant's `models.register`/`models.unregister` broker verbs (`src-agent/src/app/ext/broker.rs`), not from this array. |
| `panels` | `{id: String, title: String, icon: String}`; `icon` defaults to `""` | **Integrated, including the live message bridge:** installed metadata drives the activity bar/tab, and an open panel's iframe talks to its backing daemon over the `panel.msg`/`panel.push` bridge — see `docs/EXTENSIONS.md`'s "Panel bridge" section for the full envelope spec, size caps, and auto-start rule. |
| `tools` | `{name: String, description: String, input_schema: JSON Value}`; `input_schema` defaults to JSON `null` | **Integrated where an `McpManager` exists:** registered as namespaced MCP tools and invoked through the host. |
| `events` | `[String]` | **Integrated:** subscribes to koma's fixed event vocabulary (`subagent.done`, `agent.turn_end`, `session.foreground_change`) — best-effort, only-subscribed delivery to `on_event`. See `docs/EXTENSIONS.md`'s "Events" section for exact payloads. |
| `oauth_providers` | `{id, name, method, chat_endpoint?, api_type?, refresh?}` (W11/W12) | **Integrated:** each becomes an OAuth picker row backed by this extension's `oauth.begin`/`oauth.poll`/`oauth.cancel`, gated by `oauth:contribute`. `chat_endpoint`/`api_type` (W12) additionally make a connected provider a resolvable model gateway via `models.register`/`providers.register`. See `docs/EXTENSIONS.md`'s "OAuth providers" section. |

The installer persists only registry fields (`id`, `version`, tier/kind wire strings, `granted`, `enabled`, `exec`); the on-disk manifest remains the contribution source. `Welcome.granted` currently echoes `Hello.manifest.requires`; it is informational and is not the enforcement authority.

## 3. Transport and ownership

The host binds `~/.koma/run/ext-<id>.sock` (unix) — or the named pipe `\\.\pipe\koma-ext-<id>` on Windows — before spawning the child. It sets:

* `KOMA_EXT_SOCKET` to the socket path;
* `KOMA_EXT_TOKEN` to a fresh UUID for that start.

The child connects to the host socket. The accepted connection is one connection owned by that extension: the host accepts the child connection, and the child must not expect a second connection. The host owns an async read half and a writer task; all outbound frames are serialized through the writer. The SDK clones a Unix stream, uses a mutex-protected writer, and has a reader loop.

Frames are newline-delimited JSON objects (NDJSON): one JSON object, one `\n`; `\r\n` is accepted on read. Writers flush every frame. The host caps frame content at **4 MiB** (`4 * 1024 * 1024` bytes, excluding the newline), including the first `Hello`; a frame crossing the cap is fatal. Empty lines are ignored after handshake. JSON strings must be UTF-8. This extension socket is separate from the daemon/client IPC length-prefixed protocol (whose cap is 64 MiB).

## 4. Exact extension envelopes

Both enums use `#[serde(tag = "t", rename_all = "lowercase")]`. `params` and `result` are arbitrary JSON values. IDs are unsigned 64-bit JSON numbers and correlate only within this connection/generation.

### `ExtMsg` (extension -> koma)

| Tag and exact fields | Direction | Meaning |
|---|---|---|
| `{"t":"hello","protocol":String,"token":String,"manifest":ExtensionManifest}` | extension -> host, first frame only | Handshake identity and manifest. |
| `{"t":"call","id":u64,"method":String,"params":Value}` | extension -> host | Requirement request; currently only `agents.*` is implemented. |
| `{"t":"result","id":u64,"result":Value}` | extension -> host | Reply to host `Invoke` with the same `id`. There is no separate error field: errors are JSON values such as `{ "error": "..." }`. |
| `{"t":"health","ok":bool}` | extension -> host | Advisory health flag; does not restart or grant capabilities. |

### `KomaMsg` (koma -> extension)

| Tag and exact fields | Direction | Meaning |
|---|---|---|
| `{"t":"welcome","protocol":String,"koma_version":String,"granted":[Grant]}` | host -> extension, handshake reply | Accepted protocol and echoed requested grants. |
| `{"t":"reject","reason":String}` | host -> extension, handshake failure | Human-readable rejection reason; host then kills the child. |
| `{"t":"invoke","id":u64,"method":String,"params":Value}` | host -> extension | Contribution invocation; extension must answer `ExtMsg::Result` with this ID. |
| `{"t":"result","id":u64,"result":Value}` | host -> extension | Reply to extension `Call`; broker errors are values with an `error` string. |
| `{"t":"ping"}` | host -> extension | Liveness probe; SDK answers `Health {ok:true}`. |
| `{"t":"shutdown"}` | host -> extension | SDK serve loop exits cleanly. |

Example serialized frames are therefore `{"t":"invoke","id":7,"method":"tool.call","params":{"x":1}}` and `{"t":"result","id":7,"result":{"output":"ok"}}`.

## 5. Handshake state machine

1. **Stopped -> Starting:** host increments the extension generation, removes/binds the socket, validates the persisted relative `exec`, spawns with package directory as cwd and the two environment variables, and waits up to 10 seconds for one connection.
2. **Starting -> Hello-wait:** accepted child connection is read with the 4 MiB cap and a 10-second handshake timeout.
3. **Hello-wait acceptance:** the first complete line must parse as `ExtMsg`. A parsed message is accepted only when it is `Hello`, `protocol == "v0"`, and `token == KOMA_EXT_TOKEN`. Host then sends and flushes `Welcome { protocol:"v0", koma_version: current_version(), granted: manifest.requires }`, and enters Running.
4. **Reject/error transitions:**
   * no connection within 10 seconds: kill child; error `extension did not connect within 10s`;
   * clean EOF before any Hello: kill; `extension closed the connection before Hello`;
   * EOF in a partial frame, socket error, invalid UTF-8, or read timeout: kill; `ext handshake read failed: ...` / `ext handshake timed out`;
   * frame over 4 MiB: kill; `ext Hello frame exceeds 4194304 bytes`;
   * invalid JSON: kill; `ext Hello was not valid JSON: <serde error>` (no Reject can be reliably formed because no valid envelope was parsed);
   * valid first frame other than Hello: send `{"t":"reject","reason":"expected Hello as the first frame"}`, kill;
   * Hello protocol mismatch: send `reason = "protocol mismatch: expected v0, got <value>"`, kill;
   * token mismatch: send `reason = "token mismatch"`, kill;
   * Welcome write failure: kill; `ext Welcome write failed: ...`.
5. **Running:** only after Welcome succeeds are reader/writer tasks committed. A post-handshake Hello is ignored. A stop or stale generation kills/discards the child and removes the socket.

## 6. Steady-state concurrency, correlation, and failure

The host assigns monotonically wrapping `u64` IDs from the per-entry `next_id` for `Invoke`; the SDK starts extension-call IDs at 1 from an atomic counter. Each side has a separate pending map: host `HashMap<u64, oneshot::Sender<Result<Value,String>>>` for invokes, SDK `HashMap<u64, Sender<Value>>` for calls. A result removes only its matching entry. Unknown result IDs are ignored. Multiple calls may be in flight, and detached broker reply tasks mean replies and requests can be out of order.

The host reader and writer run concurrently. The writer owns the write half and flushes every queued frame; the reader owns the read half, routes results, and never waits inline for a broker result. SDK host mode uses a mutex around the write stream so its driver and serve loop cannot interleave bytes; its reader concurrently dispatches invokes, pings, and results. Host contribution invokes have a **120 second** timeout, except `models.invoke` which gets **360 seconds** (its own broker-side call runs a **330 second** inner budget, deliberately undercutting the 360s so the extension always gets a value back rather than hitting the transport timeout first); SDK requirement calls have a **120 second** receive timeout. A host `agents.*` (or any other non-`models.invoke`) broker request shares that same **120 second** verb-scoped timeout — it is not a separate cap (see `wire.rs`'s `EXT_CALL_TIMEOUT`/`EXT_MODELS_CALL_TIMEOUT`).

Malformed steady-state JSON is logged and ignored by the host reader (the SDK logs/ignores malformed frames). An oversized frame is fatal and host `stop()` kills the child. Clean EOF or any other read error drains/fails all pending host invokes with `extension closed connection`, marks stopped, and does not restart automatically. A write failure fails the relevant invoke as `extension not running`; a stopped host drains all pending calls with `extension stopped`. Timeout removes the pending entry and returns `extension '<id>' invoke '<method>' timed out` (host), or `{ "error":"koma call: timed out" }` (SDK).

Generation is monotonic and bumped on every start/stop. A start captures its generation before slow spawn/handshake and stores the child only if it still matches; otherwise the child is killed and the socket removed. EOF from an old reader cannot mark a newer generation stopped. The invoke path rechecks generation atomically with pending insertion, preventing a stop race from orphaning a waiter.

## 7. SDK behavior

Implement `sdk::Extension::manifest()` and `on_invoke(&mut self, method, params) -> Value`. In host mode, `run_daemon` and `run_oneshot` detect **only** the presence of `KOMA_EXT_SOCKET`, read `KOMA_EXT_TOKEN`, connect over Unix, send Hello, require Welcome (Reject or any other reply exits), then run the same duplex loop: Invoke calls `on_invoke`, Ping yields Health, Result completes a pending `Koma::call`, Shutdown/EOF exits. `Koma::call` sends `Call` and blocks up to 120 seconds.

If `KOMA_EXT_SOCKET` is unset, the SDK does **not** emulate a host: it runs a scripted demo, prints frames, and returns canned `agents.*` values. Demo output is not a host integration test and demo IDs/results do not prove broker behavior. Host mode's real duplex client runs on unix (a unix socket) AND Windows (a named pipe, `\\.\pipe\koma-ext-<id>` — see `sdk.rs`'s `WindowsPipeStream`); only on some OTHER, non-unix/non-Windows platform does it print a notice and exit instead. The current oneshot helper in host mode still runs the duplex serve loop; the intended “one request then exit” lifecycle is not fully implemented by the host.

## 8. `agents.*` broker contract

Calls are queued by the socket reader into `AppStateRest::ext_call_tx`, drained on an event-loop tick, and handled with `AppState`. Grant checking occurs before reading or mutating session state. Every response is a JSON value; errors are never left pending. If the broker channel is absent the exact response is `{ "error":"grant broker not initialized" }`; if closed, `{ "error":"grant broker unavailable" }`; if it drops, `{ "error":"grant broker dropped request" }`; after the verb-scoped timeout (120s for `agents.*`, 360s for `models.invoke`), `{ "error":"grant broker timed out" }`.

`agents:orchestrate` implies `agents:read`; read does not imply orchestration. Unknown methods return `{ "error":"unknown method: <method>" }`.

| Method | Required grant | Parameters and validation | Success examples |
|---|---|---|---|
| `agents.spawn` | `agents:orchestrate` | `task` must be a non-empty trimmed string. Optional non-empty trimmed `agent`; default `"general"`. | `{ "agentId":0,"status":"spawned" }` or `{ "agentId":1,"status":"queued" }` |
| `agents.list` | read or orchestrate | Parameters are ignored; lists only this extension's registry, oldest ID first. | `[{"agentId":0,"agent":"general","status":"running"}]`; missing/closed target is `{agentId,status:"gone"}`. |
| `agents.status` | read or orchestrate | `agentId` required; accepts JSON u64 or numeric string. Must be an ID previously handed to this extension. | `{ "agentId":0,"agent":"general","status":"running","liveTextLen":42 }`; queued has `{agentId,status:"queued"}`. |
| `agents.result` | read or orchestrate | Same `agentId` validation/isolation. | Done: `{ "agentId":0,"status":"done","output":"..." }`; running/queued/killed markers; error: `{ "agentId":0,"status":"error","error":"..." }`. |
| `agents.kill` | `agents:orchestrate` | Same `agentId` validation/isolation. | `{ "killed":true }`; idempotent for an already-killed agent. |

Denied calls return `{ "error":"grant denied: <method> requires agents:read" }` or `...agents:orchestrate`. Missing params return exact strings: `agents.spawn requires a non-empty 'task'`, `agents.status requires an 'agentId'`, `agents.result requires an 'agentId'`, and `agents.kill requires an 'agentId'`. Unknown IDs return `unknown agentId: <n>`; closed sessions return `session closed`; spawn with no active session returns `no active session`; failed spawn returns `failed to spawn agent '<agent>' (no client/session or unknown agent)`.

Spawn targets the active/foreground session (or first non-closed fallback), uses the normal `spawn_or_queue` path/cap, and is non-detached with no tool-call ID, but is marked ext-owned; completion updates usage/the persisted sub-agent record but is now (since the two-tier silent-done change) COMPLETELY SILENT in the human chat — no display note, no nudge — and does not wake the chat model. The extension gets its result via `agents.result`/`agents.done` instead; see `docs/EXTENSIONS.md`'s "Events" section for the exact contrast with a human-run `/task`'s chat-visible completion fold. The returned extension-facing ID is private to this extension and permanently maps to a stable session UUID plus local sub-agent ID. List/status/result/kill never resolve raw IDs or another extension's/user's IDs, even after foreground switches. Uninstall removes that extension's registry.

**This section only covers `agents:read`/`agents:orchestrate`, the first two grants the broker shipped with.** Six more grants (`sessions:manage`, `chat:prompt`, `models:invoke`, `context:publish`, `oauth:contribute`, `models:contribute`) have landed since, each unlocking its own set of broker verbs (`sessions.*`, `chat.prompt`, `models.invoke`, `context.*`, `oauth.*` invokes, `models.register`/`.unregister`, `providers.register`/`.unregister`) with the same "grant-gated, JSON-value replies, errors never left pending" shape as the table above. `docs/EXTENSIONS.md`'s grants reference has the complete, current table — every verb, every request/response shape, every error string, every numeric limit — kept in lockstep with `broker.rs`; treat it as authoritative for anything not `agents.*`.

## 9. Contribution invocation and limitations

`contributes.tools` is registered as `mcp__<sanitized-extension-id>__<tool>` where an MCP manager is available; a model tool call reaches `ExtHostManager::invoke`, which sends `Invoke` and awaits `ExtMsg::Result`. `contributes.sub_agents` is merged by the next `AgentRegistry::load`, with `model`/`effort` resolved through the binding chain in `src-agent/src/app/resolve.rs` (see section 2 above). `contributes.models` stays declared-shape-only in the manifest — the actual runtime catalogue is built through `models.register`/`providers.register` (the `models:contribute` grant), not from this array. Panels are metadata-driven (`PanelWire` with `id/title/icon`) for activity-bar/tab display AND have a live runtime bridge: an open panel's iframe reaches its backing daemon's `on_invoke("panel.msg", ...)` over the host, with pushes flowing back via `Koma::panel_push` — see `docs/EXTENSIONS.md`'s "Panel bridge" section for the full envelope, caps, and auto-start rule; this document's section 10 below covers only the GUI IPC boundary framing, not that bridge's own contract. Registration is pushed after successful daemon startup; purge removes extension tools on disable/uninstall (and, for `models:contribute`/`oauth:contribute` state, the fuller purge described in `docs/EXTENSIONS.md`'s "Lifecycle" section).

## 10. Store, install, and GUI IPC boundaries

The extension store is daemon-owned for mutations. `StoreBrowse`/`StoreDetail` use public `https://koma.run/api/v1/extensions`; install needs a KomaRun OAuth bearer, downloads artifact metadata, verifies/unpacks/registers/spawns on the daemon event loop, and emits `ExtensionOpResult` then `InstalledExtensions`. Uninstall purges tools, stops the child, removes `~/.koma/extensions/<id>/`, removes/persists the registry entry, and clears extension agent ownership. A missing platform or login is a deterministic failure. Installed projections contain display fields and panels, never token or executable path.

For the desktop GUI the exact boundary is:

```text
src-webgui host React -> window.ipc.postMessage({t:"req", r:<GuiReq>, ...fields})
  -> ClientMsg::Req(GuiReq) in src-agent GUI host
  -> handle_gui_req
  -> (HostCtl host-local work) OR ClientRequest to attached session daemon
  -> daemon DaemonFrame / host result
  -> PushEnvelope {k:<tag>, ...}
  -> UserEvent::Push -> evaluate_script(window.__komaClient.push(JSON))
  -> src-webgui host React/Zustand reducer
```

An installed extension iframe is not part of this host bridge. It receives neither `window.ipc` nor `window.__komaClient`; it only loads its static `ui/` content from the separate `koma://extension/...` origin.

`GuiReq` is the webview-to-host union (`r` tags such as `Ready`, `Submit`, `SelectSession`, `InstallExtension`, and `ListInstalledExtensions`). `ClientRequest` is the separate daemon-client union (including extension store requests); it crosses the daemon's length-prefixed Unix IPC, not the extension socket. `PushEnvelope` is host-to-iframe JSON tagged `k`; extension store replies include `StoreCatalogue`, `StoreItemDetail`, `InstalledExtensions`, `InstalledExtensionDetail`, and `ExtensionOpResult`. The extension socket `ExtMsg`/`KomaMsg` channel is **not** used by the iframe and is never exposed through `GuiReq` or `PushEnvelope`.

`GetInstalledExtensionDetail { id }` is a host-local GUI request. It is deliberately separate from marketplace `StoreDetail`: the host first validates `id` against the installed registry, then reads only `extensions/<id>/manifest.json`, whether the GUI is attached to a daemon or detached at the session hub. The matching `InstalledExtensionDetail` push echoes the requested `id` and carries either a safe projection (`id`, `name`, `version`, `description`, `tier`, `kind`, `requires`, and contribution metadata) or an explicit error for a missing registry entry or missing, unreadable, or invalid manifest. The projection excludes `runtime.exec`, runtime arguments, tokens, and filesystem paths. Installed rows in the sidebar and Store tab open closeable `installed-ext:<id>` Tab-B tabs; the marketplace Store tab remains catalogue/detail-only. An authoritative `InstalledExtensions` refresh removes Tab-B tabs for extensions no longer installed and selects the left neighbor.

A panel is served as `koma://extension/<id>/<path>` from `~/.koma/extensions/<id>/ui/`; host chrome is `koma://localhost/`. The authority difference is an origin boundary. The iframe has no `sandbox` attribute because it would prevent the custom scheme from loading, but it cannot script the host origin. It does **not** inherit host `window.ipc`, `window.__komaClient`, or host `window.komaIpc`; a panel must use its own static UI and has no documented host IPC bridge.

## 11. Lifecycle transitions

* **Install:** verify raw zip SHA-256, then Ed25519 signature over the raw 32-byte digest, before disk writes; reject unsafe zip paths, bound each entry at 256 MiB and total unpacked data at 1 GiB, validate ID/relative executable, unpack, chmod executable, persist enabled registry. Debug-only unsigned install exists under `debug_assertions`.
* **Startup:** `build_startup` creates the host, wires the broker channel before readers can run, and best-effort starts enabled daemon-kind entries in blocking tasks. Failed starts are logged; no automatic restart follows. Oneshots are not boot-started.
* **Stop/disable:** stop bumps generation, fails pending callers, drops writer, kills child, and unlinks socket. Tool purge is separate and must be called by lifecycle/command handling; sub-agent definitions disappear on the next registry load when disabled/removed.
* **Uninstall:** stop/purge/remove package/config and extension agent registry as above; unsafe client IDs are refused for filesystem removal.
* **Graceful shutdown:** `QuitDaemon`, SIGTERM/SIGINT, or lifecycle teardown calls `ExtHostManager::stop_all`; runtime drop cancels tasks and `kill_on_drop` protects children. The daemon's own client writer drains queued final requests with a 200 ms ceiling; this is separate from extension framing.
* **Unexpected child/socket exit:** reader fails pending invokes, marks that generation stopped, logs stderr/read errors, and leaves the extension stopped. It is not restarted or re-registered automatically. A later explicit/startup attempt may create a new generation.

## 12. Trust and security boundaries

Production packages are trusted, signed first-party processes, not a sandbox. SHA-256 and pinned Ed25519 checks precede writes. Zip entries reject absolute paths, `..`, and embedded backslashes; IDs are whitelist-validated; `runtime.exec` is checked both at install and every spawn and must remain under the install directory. Store/UI paths repeat the ID and relative-path guards. `~/.koma` and `~/.koma/run` are mode 0700 on Unix. Socket tokens are per start; a child with a mismatched token is rejected.

The grant list is parsed fail-closed: unknown persisted grant strings are dropped. Handshake echo is not trust elevation; broker checks the persisted grant set on every method. Grant denial happens before session access. Extension ownership maps prevent cross-extension, cross-session, user-agent reads or kills. The iframe origin boundary prevents host scripting, but the installed process itself is trusted and can use its granted capabilities; do not treat panel HTML as a process sandbox.

## 13. End-to-end trace and implementation checklist

For `fleet-board-daemon` the trace is:

```text
install signed zip -> ~/.koma/extensions/run.koma.example.fleet-board-daemon/
startup host binds ~/.koma/run/ext-<id>.sock
host spawns bin/fleet-board-daemon with KOMA_EXT_SOCKET/TOKEN
extension connects -> Ext Hello(manifest requires agents:orchestrate)
host validates -> Welcome(granted echo)
ext driver -> Call(id=1, agents.spawn, {task:"card 1"})
socket reader -> ext_call_tx -> event-loop drain -> grant broker
broker -> Result(id=1, {agentId:0,status:"spawned"})
ext -> Call agents.status -> Result(status/output)
model tool call (if registered) -> host Invoke -> extension Result
panel click -> React GuiReq/host Push bridge -> iframe loads koma://extension/<id>/ui/index.html
shutdown/uninstall -> stop/purge/remove
```

Checklist for a custom extension based on `src-extension/example/fleet-board-daemon`:

1. Depend on `koma-extension` by path; implement `Extension`, parse a checked-in manifest, and choose daemon only if persistent host state is required (delegated OAuth and a live panel both require it).
2. Keep `id`, `runtime.exec`, contribution names, and requested grants intentional; use `bin/<name>` in the packaged manifest.
3. Implement every declared invocation method and return JSON values, including explicit error values.
4. Use `Koma::call` only from a driver/worker thread you own, never from `on_invoke`/`on_event` — see the DEADLOCK RULE in `src-extension/src/sdk.rs`'s `Extension` trait docs and `docs/EXTENSIONS.md`'s "deadlock rule and threading model" section; validate and handle timeout/error values for whichever grant-gated verb you call (all eight are covered there, not just `agents.*`).
5. Exercise demo mode, but test host mode with the real daemon, token, handshake, out-of-order calls, EOF, and oversized/malformed frames.
6. Put panel assets under `ui/`, use relative URLs, and do not expect host globals or IPC in the iframe — talk to the host through the `panel.msg`/`panel.push` envelope instead (copy `ui/koma-panel.js` from `fleet-board-daemon` rather than hand-rolling it).
7. Package `manifest.json`, `bin/<name>`, and optional `ui/`; production installation requires a signed archive.
8. Verify contribution limitations that are STILL true: tools need a live MCP manager, sub-agents wait for the next registry load. Panel and model/provider runtime wiring are NOT limitations anymore (see section 9) — don't design around gaps that have since closed.

## 14. Sources and verification

| Contract area | Source of truth |
|---|---|
| Manifest, grants, `ExtMsg`/`KomaMsg`, SDK | `src-extension/src/protocol.rs`, `src-extension/src/sdk.rs` |
| Host lifecycle, generations, timeouts | `src-agent/src/app/ext/mod.rs`, `wire.rs` |
| Install and path/signature checks | `src-agent/src/app/ext/install.rs` |
| Contributions | `src-agent/src/app/ext/register.rs` |
| Agent grants and ownership; `models.register`/`providers.register` | `src-agent/src/app/ext/broker.rs` (all 8 grants; event-loop drains feed it) |
| Sub-agent model slug binding | `src-agent/src/app/resolve.rs` (`resolve_agent`, `ext_conn_route`, `ext_preferred_provider_uuids`) |
| Event fan-out (`subagent.done`/`agent.turn_end`/`session.foreground_change`/`agents.done`) | `src-agent/src/app/ext/events.rs`, `mod.rs` (`notify`/`subscribers`); trigger sites in `runtime/event_loop/sessions/{subagents,deferred}.rs`, `runtime/actions/session/attach.rs` |
| Delegated OAuth (`oauth.begin`/`.poll`/`.cancel`) | `src-agent/src/app/runtime/event_loop/daemon/hub/requests_oauth.rs` |
| Extension uninstall purge (`purge_extension`) | `src-agent/src/model/app_config.rs` |
| Startup/shutdown | `src-agent/src/app/runtime/lifecycle/mod.rs` |
| Daemon/client, extension store, and panel-bridge requests | `src-agent/src/ipc/proto/mod.rs`; `runtime/event_loop/daemon/hub/requests_ext.rs` |
| GUI request/push boundaries | `runtime/gui/proto.rs`, `dispatch.rs`, `mod.rs`; `runtime/client/{connect.rs,bridge.rs,push_proto.rs}` |
| GUI-side panel bridge (iframe postMessage layer) | `src-webgui/src/lib/panelBridge.ts`, `components/ExtensionPanelFrame.tsx` |
| Installed metadata/detail projection | `src-agent/src/ipc/proto/snapshot/ext.rs`; host-local reads in `src-agent/src/app/runtime/client/store_host.rs` |
| Web request/store/panel behavior | `src-webgui/src/koma.d.ts`, `store/koma.ts`, `routes/index.tsx`, `components/ActivityBar.tsx`, `components/TabBar.tsx`; `docs/WEBGUI_IPC.md` |
| Full capability reference (all grants, events, panel bridge, OAuth, lifecycle) | `docs/EXTENSIONS.md` |

From `src-extension/`: `cargo check --workspace`, `cargo test --workspace`, `cargo run -p <any of the 7 examples in src-extension/example/>`, `./pack.sh` (builds and packages all 7). `src-extension` is its own workspace — the root `Cargo.toml` explicitly `exclude`s it, so root-level `cargo check --workspace`/`cargo test --workspace` never touch it; always `cd src-extension` (or pass `--manifest-path`) first. From root: `cargo check -p agent`, `cargo build -p agent`, `cargo build -p agent --features gui` (Linux requires WebKitGTK/GTK3/libsoup3 development packages), `npm --prefix src-webgui run build`, and `cargo test -p agent`. Before install testing, confirm the archive has `manifest.json`, `bin/<runtime.exec>`, and (for a panel) `ui/index.html` plus any other `ui/` assets (`cp -r` in `pack.sh` copies the whole directory, not a filtered glob); confirm manifest parsing, ID/exec guards, intentional grants, and a Hello/Welcome frame below 4 MiB.
