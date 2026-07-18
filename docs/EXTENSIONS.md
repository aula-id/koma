# Extensions

> **This is a v0 direction document.** The extension protocol is still being
> shaped and will change until it is frozen at v1. This version reflects the
> feature set landed through wave W12b (delegated OAuth, `models.register`,
> `providers.register`, the full grants surface, the panel bridge, and event
> fan-out) plus everything landed since: manifest sub-agents can declare a
> `tools` allow-list and now inherit connected MCP tools automatically; a
> manifest can declare a `workspace_dir` — an extension-owned writable root
> injected into every session; `models.invoke` takes `format: "json"` and its
> timeouts were raised (every other verb gets a 120s wire timeout;
> `models.invoke` alone gets 360s wire / 330s inner); `agents.send` lets an extension steer an
> already-spawned sub-agent at its next turn boundary; sub-agents an
> extension spawns via `agents.spawn` are completely silent in the human's
> chat (the spawner gets the result through `agents.done` instead); panels
> are theme-aware (`KomaPanel.onTheme`/`getTheme`); and `koma ext install
> --dev` sideloads an unsigned local build with no store, no signature, no
> sign-in — everything described below is real and runnable today, not a
> proposal. For the byte-level wire contract (framing, timeouts, handshake
> state machine) see `docs/ARCH_EXTENSION.md`; this document is the
> capability reference — what an extension can do and exactly how, with
> every payload shape verified against the source that implements it
> (`src-extension/src/protocol.rs`, `src-extension/src/sdk.rs`,
> `src-agent/src/app/ext/broker.rs`, and the daemon hub under
> `src-agent/src/app/runtime/event_loop/daemon/hub/`).

## What an extension is

An extension is a small program that runs alongside koma and adds to what koma can
do. It is not a script that gets loaded into the agent, and it is not tied to any
particular language. An extension is its own process — it might be a Rust binary, a
long-running daemon, or a single self-contained Node build — and koma launches it,
supervises it, and talks to it over a local socket. The only thing that makes
something a koma extension is that it speaks the extension protocol.

Extensions are first-party. koma writes them, koma signs them, and you install them
from the koma.run store while signed in to your koma.run account. There is no
open marketplace where anyone can publish, and there is no plugin sandbox trying to
contain untrusted code — the trust comes from the fact that koma published it. If
you want to build your own and compile it yourself, the SDK is public and you are
free to, but the store ships koma's.

## The two kinds of extension

The most important idea in the whole design is that an extension and koma can use
each other in *both* directions, and you should keep that distinction in mind
whenever you think about what an extension is for.

Sometimes **koma uses the extension**. koma is in charge, and the extension is
something koma reaches into when it needs it — a model provider it can send
requests to, a tool the agent can call, a panel it can render, an OAuth login it
delegates. The extension sits there and answers.

Other times **the extension uses koma**. Now the extension is in charge, and koma
is the engine it drives. Picture an agentic kanban board: you hand it a product
spec, and it turns around and asks koma to spawn a whole fleet of sub-agents, one
per card, then steers them as they work. The extension is the one calling the
shots; koma is doing the heavy lifting underneath.

```text
koma uses the extension        the extension uses koma
  koma ── request ──▶ ext        ext ── spawn / steer ──▶ koma
```

A single extension can be both at once — the kanban board *provides* its own UI
panel to koma while it *drives* koma's sub-agents, and can *also* subscribe to
koma's events to keep that panel live. `src-extension/example/fleet-board-daemon`
is exactly this: panel + drive + events, all in one small daemon — read it start
to finish as the reference for what "both directions" looks like in real code.

The practical consequence is that the connection between koma and an extension has
to work **both ways over the same link**. koma has to be able to call into the
extension, and the extension has to be able to call back into koma. So the two-way
shape (`ExtMsg`/`KomaMsg`, described in full below) is baked in from the start, and
every sample in `src-extension/example/` exercises at least one direction of it.

## What an extension gives koma: contributions

The things an extension adds to koma are called its *contributions*, declared under
`contributes` in the manifest. There are six kinds today.

**Sub-agents** (`contributes.sub_agents`). An extension can ship its own agent
descriptions — including a system prompt, a preferred model slug, and a default
effort — and once it is installed those show up as new agent types you can delegate
work to, right alongside the built-in ones. See the manifest reference below for
the full `SubAgentDef` shape and how its `model` slug resolves.

**Models** (`contributes.models`). Declared shape only in the manifest — the
catalogue an extension actually SERVES is built at runtime through the
`models:contribute` grant's `models.register` verb (see the grants reference),
once a user has connected the extension's backing account or key.

**Panels** (`contributes.panels`). An extension can bring its own user interface. It
becomes a tab in the main area with an icon in the sidebar, and koma frames it so
the extension's UI runs in its own process (technically: its own webview origin)
without leaking into koma's. See "Panel bridge" below for the live message-passing
contract a panel uses to actually talk to its backing daemon.

**Tools** (`contributes.tools`). An extension can give the agent new tools to
call — and this is worth a short explanation, because the obvious idea is the wrong
one. You might expect an extension to register a "built-in" tool. But koma's
built-in tools are compiled directly into the agent; a separate process simply
cannot inject one without a forwarding layer in between, and that forwarding layer
is exactly what MCP already is. So extension tools *are* MCP. The difference from
an MCP server you'd add by hand is ownership: an extension's tools belong to the
extension. They appear in the tool list marked as coming from that extension, and
you don't remove them one by one — you remove them by uninstalling the extension.

**OAuth providers** (`contributes.oauth_providers`). An extension can back its own
row in koma's OAuth picker and run the entire login flow itself — koma delegates,
never sees a client secret, and only stores the resulting token. See "OAuth
providers" below for the full `oauth.begin`/`oauth.poll`/`oauth.cancel` contract and
how a provider becomes a resolvable model gateway on top of that.

**TUI screens** (`contributes.tui_screens`). An extension can drive a full-screen
view koma's TERMINAL UI renders on its behalf — the same idea as a panel, but for
someone running koma's TUI instead of the GUI, and it costs zero protocol change: it
reuses the exact same `panel.msg`/`panel.push` verbs a panel already uses, just
distinguished by a `kind` tag in the payload. See "TUI screens" below for the full
`Screen`/`Node` model and the host-side navigation contract.

**Events** (`contributes.events`) sit slightly apart from the five contribution
arrays above — an extension doesn't *provide* an event, it *subscribes* to ones
koma already emits. It's declared in the same `contributes` object because the
mechanics are the same (an array in the manifest that changes what koma sends you),
so it's covered here for completeness and in full under "Events" below.

Whatever an extension contributes gets cleaned up when you uninstall it. Its models,
its registered providers, its OAuth connections, its sub-agents, its tools, its
panel — all registered on install or at runtime, all purged on removal (see
"Lifecycle" below for the exact purge list), so uninstalling actually leaves no
trace.

## What an extension asks of koma: requirements and grants

If contributions are what an extension gives, *requirements* are what it asks to
use. When an extension needs to drive one of koma's own systems, it declares that
up front in `requires: [...]`, and koma enforces it at the boundary — every single
ext→koma `Call` verb is gated by exactly one grant, checked before koma reads or
mutates any session state. This doubles as the permission list you see before
installing: an extension that wants to spawn sub-agents has to say so, and you get
to see it say so.

There are eight grants today, covering five broad surfaces: reading/orchestrating
sub-agents, managing sessions, injecting chat turns, invoking a model directly,
publishing system-prompt context, backing an OAuth login, and registering models or
key-backed providers into the catalogue. **The full reference — every grant, every
verb it unlocks, every request/response shape, every error string, every numeric
limit — is its own section below** ("Grants reference"); this section stays the
narrative overview.

Even though every extension is first-party and trusted, koma still keeps these
requirements scoped tightly. A model gateway that only needs to borrow your account
session has no business holding orchestration rights, and koma shouldn't have to
assume it won't use them. Asking for exactly what you need, and no more, stays the
rule — not as a defense against the author, but as clean engineering. Concretely:
`agents:orchestrate` implies `agents:read` (the only implication in the whole grant
lattice — verified exhaustively by broker.rs's `grant_gate_truth_table` test); every
other grant is an exact match with no implicit unlock in either direction.
`oauth:contribute` is the one grant that gates no ext→koma `Call` verb at all — it
gates the opposite direction, koma calling INTO the extension for `oauth.*`.

```text
agentic kanban
  you give it a product spec
    → it spawns one sub-agent per card
    → each agent works on its card on its own
    → their status flows back and the board updates live
```

## Installing and running

Browsing the store is open to anyone; installing is what needs an account. When you
install an extension, koma downloads it, checks its signature, and launches it as a
supervised sibling process. The two sides introduce themselves in a short handshake
— the extension presents what it is and what it contributes and requires, and koma
agrees on a protocol version — and from then on the extension is live. Its
contributions light up across koma and its requirements are enforced as it runs.
When you quit koma it is shut down with you, and when you uninstall it everything it
added is removed. See "Lifecycle" below for exactly what triggers a daemon
extension to start and exactly what gets purged on uninstall.

An extension describes itself in a manifest, which is where it declares its
identity, whether it is a free or paid extension, how koma should launch it, and both
its contributions and its requirements. That manifest is the whole agreement between
koma and the extension in one place — the full field-by-field reference is next.

Building your own extension? `koma ext install --dev <zip|dir>` sideloads it locally
with no store, no signature, no sign-in — see "Dev install (offline, unsigned)"
under Lifecycle below.

## How it looks in the app

Installed extensions live in the sidebar, each with its own icon sitting next to
koma's built-in views. You can drag those icons around to reorder them however you
like, and the order sticks. When there are more than fit, the overflow folds into a
small "additional views" menu rather than pushing everything off-screen.

Extensions are tab-first. Clicking an extension's sidebar icon opens *its* tab in the
main area — the side panel is a launcher for what you've installed, not a search box
you dig through.

The store itself is meant to feel like a real storefront — featured extensions,
categories, screenshots, something you browse — rather than a plugin search list. You
can look around freely; you sign in to install.

Your koma.run account lives quietly inside the Settings menu. There is no in-app
profile page or dashboard; when you want to manage your account you follow a link out
to the koma.run dashboard in your browser. One account is all it takes to unlock the
store, and extensions borrow what they need from koma rather than each making you log
in again.

The terminal UI has the same two surfaces as its own modes rather than sidebar icons:
`/extension` opens the browse/detail/uninstall flow (browse your installed
extensions, Enter for an extension's detail — its permissions, its workspace
directory if it has one, and, for an extension that declares
`contributes.tui_screens`, a selectable row per screen that jumps straight into the
extension-driven full-screen view described under "TUI screens" below), and `/store`
mirrors the GUI's storefront browse for discovering new ones. Nothing here needs a
panel's webview — the TUI never renders one — so `contributes.tui_screens` is a
GUI-less extension's only route to a full-screen UI of its own.

## Two extensions to picture

Two examples make the two directions concrete.

**koma-gateway** is the "koma uses the extension" kind. It presents an
OpenAI-compatible endpoint, koma sends requests to it like any other provider, and it
routes them onward. It doesn't care what model you named — it takes the request and
handles the rest. It is a pure provider: it contributes models and nothing more. The
`oauth-demo-daemon` sample's teaching comments walk through exactly this shape (a
gateway declared via `chat_endpoint`/`api_type` plus `models.register` /
`providers.register`), even though the sample's own demo flow stays account-login
only so it needs no real backing service to run.

**komatica** is the mirror image, the "extension uses koma" kind. It is an agentic
kanban that consumes a product spec and drives a fleet of sub-agents through koma to
build against it, each card its own working agent. It contributes its board as a
panel, but its real job is to orchestrate — it is in the driver's seat and koma is
the engine. `fleet-board-daemon` is the runnable version of this idea: a live panel,
a declared sub-agent, and the full spawn-from-the-UI round trip.

## The SDK

The tools for building an extension live in `src-extension/` in the open repo. There
is a `koma-extension` crate that carries the protocol — the manifest, the handshake,
the contribution and requirement types — along with a thin layer of helpers
(`sdk.rs`) so that opening the socket, completing the handshake, and answering
`Invoke`s is a few lines rather than a project. Every sample ships a **standalone
demo mode**: run any of them with `cargo run -p <name>` and, with no koma process on
the other end, it prints the handshake and the scripted interaction it would have
with koma, frame by frame, so the protocol's shape is visible without a host to talk
to. Set `KOMA_EXT_SOCKET` (and `KOMA_EXT_TOKEN`) and a sample instead connects to a
real koma over that socket and runs for real — a unix socket path on Linux/macOS,
or a `\\.\pipe\koma-ext-<id>` named pipe on Windows.

Rust is the source of truth and the first SDK; a generated TypeScript version comes
later.

### The samples, as a learning path

Read them in roughly this order — each adds one new piece:

1. **`echo-tool-daemon`** / **`upper-tool-oneshot`** — the smallest possible daemon
   and oneshot: one contributed tool, no `requires`, koma invokes it once. Start
   here to see `Extension::on_invoke` and the daemon/oneshot lifecycle split.
2. **`agent-peek-oneshot`** — contributes nothing; only requires `agents:read` and
   calls `agents.list`. The smallest possible "extension drives koma" sample.
3. **`event-watcher-daemon`** — THE starter sample for `contributes.events`:
   subscribes to every broadcast event, counts them, answers a tool with the
   counts. Start here for `Extension::on_event`.
4. **`orchestrator-daemon`** — documentation-as-code for the five grant-verbs that
   drive koma (`sessions.list`, `models.invoke`, `context.set`, `chat.prompt`,
   `agents.spawn`), each call commented with its real request/reply shape and
   every error mode. Read this like a manual, not a template.
5. **`fleet-board-daemon`** — the live bridge demo: a real panel
   (`ui/index.html` + the copyable `ui/koma-panel.js` helper) driving a real
   sub-agent spawn through the cmd-channel pattern that the DEADLOCK RULE forces
   on you. This is the sample to fork for a real "extension with a UI that drives
   koma" project.
6. **`oauth-demo-daemon`** — the delegated-OAuth reference: a fake device-code
   flow exercising `oauth.begin`/`oauth.poll`/`oauth.cancel`, with teaching
   comments walking through the full W12/W12b arc from a login to a resolvable
   model-provider gateway (`models.register` + `providers.register`).

## The deadlock rule and the threading model

This is the single most important operational fact about writing an extension, and
it comes up in every sample past the simplest ones, so it gets its own section.

The host-mode serve loop is **single-threaded**: one thread reads every frame off
the socket, dispatches `Invoke`s to your `on_invoke`, dispatches `Event`s to your
`on_event`, and routes `Result` frames back to whichever `Koma::call` is waiting for
them. If you call `Koma::call` from inside `on_invoke` or `on_event`, you deadlock:
`call` blocks waiting for a `KomaMsg::Result`, but the only thing that could ever
read that `Result` off the socket is the very serve-loop thread your handler is
currently blocking. There is no other reader.

The safe pattern: **reply immediately** from `on_invoke` (or just return from
`on_event`), and hand any real work off to a driver thread (or your own worker
thread) via a channel you own. `Koma::notify` and `Koma::panel_push` are
write-only — no reply is ever awaited for them — so they ARE safe to call from
either handler, provided you have a live `Koma` handle to call them on in the first
place.

That last caveat is exactly what makes `fleet-board-daemon` worth reading closely:
`DaemonDemo::driver` is a bare `fn(&mut Koma)` — a function pointer, not a closure —
so it can't capture a channel from `main()`, and the `Extension` struct's
`on_invoke`/`on_event` don't get handed a `Koma` handle at all (only the driver
does). The sample's solution is one `std::sync::mpsc` channel with the receiver
parked in a `std::sync::OnceLock`: both handlers just push a `Cmd` and return; the
driver thread — the only place holding a live `Koma` — drains the channel and does
all the `koma.call`/`koma.panel_push` work. `Koma::try_clone()` exists and would let
you hand a second live handle to a background thread instead (it shares the
underlying connection via `Arc`), but routing everything through one channel onto
one thread is simpler here and sidesteps synchronizing two independent socket
writers — read the module doc comment at the top of `fleet-board-daemon/src/main.rs`
for the full reasoning.

---

## Manifest reference

`manifest.json` deserializes straight into `ExtensionManifest`
(`src-extension/src/protocol.rs`); that struct is the authoritative shape, and every
sample loads its manifest at compile time with `include_str!` and parses it at
startup, so a bad manifest fails loudly instead of silently drifting from the code.

| Field | Type | Required | Notes |
| --- | --- | --- | --- |
| `schema` | string | required | Must be `"koma-extension/v0"` (`MANIFEST_SCHEMA`). |
| `id` | string | required | Reverse-DNS, e.g. `run.koma.example.fleet-board-daemon`. Install guards enforce `[A-Za-z0-9._-]`, at least one alphanumeric, no leading/trailing dot. |
| `name` | string | required | Display name. |
| `version` | string | required | |
| `description` | string | omitted → `""` | |
| `tier` | `"free"` \| `"paid"` | required | |
| `kind` | `"daemon"` \| `"oneshot"` | required | See "Daemon vs. oneshot" below. |
| `runtime` | object | required | `{ "exec": <string>, "args"?: [<string>] }` — `exec` is relative to the package root; `args` defaults to `[]`. |
| `contributes` | object | omitted → `{}` | See below. |
| `requires` | `[Grant]` | omitted → `[]` | Wire strings, e.g. `"agents:orchestrate"`. See the grants reference. |
| `workspace_dir` | string | omitted → none | An extension-owned state directory koma creates and injects as a session workspace root. Must resolve strictly under `$HOME`. See "`workspace_dir`" below. |
| `mcp_servers` | `[ManifestMcpServer]` | omitted → `[]` | Bundled stdio MCP servers auto-registered into koma's MCP catalogue at install time. See "`mcp_servers`" below. |

### `workspace_dir`

An optional dedicated state directory the extension owns, declared as a path string —
typically `"~/.<ext-name>"` (the `event-watcher-daemon` sample uses `"~/.event-watcher"`):

```json
{ "workspace_dir": "~/.event-watcher" }
```

When present, koma validates the path, **creates it if missing**, and injects its
canonical form as an extra workspace root of every session. It appears as an `[N]` root
alongside the launch directory, so the agent's file tools and `bash` may read and write
there (an extension's own sub-agents can persist state that survives a restart) — it is
exempt from the safety harness the same way any configured workspace root is, and is
named in the system prompt's "Extension workspaces" note by its `[N]` index and owning
extension id.

**Validation rules.** A path that fails ANY rule is logged to `~/.koma/error.log` and
skipped — it never blocks the extension from starting:

- `~` / `~/…` expands to `$HOME` (`%USERPROFILE%` on Windows); a `~user` form is rejected.
- The resolved path must be **strictly under `$HOME`** — `$HOME` itself is rejected, as is anything outside `$HOME`.
- koma's own `~/.koma` tree is rejected.
- The credential stores `~/.ssh`, `~/.aws`, `~/.gnupg` — and everything under them — are rejected.
- `~/.config` **itself** is rejected, but its subdirectories (e.g. `~/.config/my-ext`) are allowed.
- Any other `$HOME` subdirectory — including a dotdir like `~/.babalic-extension` — is allowed.

Comparison is on canonicalized paths (symlinks and `..` resolved), so a symlinked escape
can't slip past. Injection happens at daemon/TUI startup, and again the moment an
extension is installed at runtime (no restart needed). It is in-memory and re-derived
from the currently **enabled** extension set on every start, so disabling or uninstalling
an extension drops its workspace root on the next start.

### `mcp_servers`

A bundled stdio MCP server an extension ships is a genuinely SEPARATE binary from its own
`runtime.exec` daemon — e.g. the Workflow extension's `bin/workflow-mcp`, spawned
alongside `bin/office-daemon`. Before this field, koma had no way to learn about it: the
binary unpacked (its exec bit preserved by the zip-mode-preservation step of install), but
nothing ever registered it as an `McpServerEntry`, so a fresh install showed "No MCP
servers" until the user hand-added one through the MCP settings.

`mcp_servers` closes that gap — declare each one and koma auto-registers it at install
time:

```json
{
  "mcp_servers": [
    { "name": "workflow", "exec": "bin/workflow-mcp", "args": [] }
  ]
}
```

| Field | Type | Notes |
| --- | --- | --- |
| `name` | string | Display name; becomes the registered `McpServerEntry.name` (and thus the `mcp__<name>__<tool>` advertise prefix) unless it collides with a row this extension doesn't already own, in which case koma prefixes it with the extension's own id (`"<ext_id>:<name>"`) to disambiguate. |
| `exec` | string | Path to the stdio MCP server executable, RELATIVE to the package root — the SAME containment discipline as `runtime.exec`: no `..`/absolute escape, and it must exist under the extension's install dir after unpack, or the WHOLE registration fails closed (no partial/broken rows). |
| `args` | `[string]`, optional | Arguments passed to `exec` at spawn. Defaults to `[]`. |

**Registration is an UPSERT, keyed on `(ext_id, name)`.** `app::ext::register::register_mcp_servers`
runs at every install (fresh, reinstall, or upgrade) and:

- **replaces its own entries in place** — a row this SAME extension registered under the
  same declared name keeps its `uuid` (so it doesn't lose its identity) and its `enabled`
  flag (so a user who disabled it stays disabled across an upgrade); only `command`/`args`/
  `env` move to match the new version;
- **drops stale rows** — a name this extension declared before but no longer does is
  removed, so an upgrade that stops shipping a server doesn't leave a dead orphan behind;
- **never touches a user-created row** (`ext_id: None`) or a row belonging to a DIFFERENT
  extension — provenance-tagged (`McpServerEntry.ext_id`) exactly like every other
  ext-owned config row.

`command` is always resolved to an ABSOLUTE path under `extensions/<id>/` via a plain
`PathBuf::join` (platform-native separators — no hardcoded `/`, so this is Windows-safe
without any special-casing). Uninstall's complete nuke removes these rows the same way it
removes a hand-added one — see step 5 of "Uninstall" below.

**Enable/disable**: there is no extension-level enable/disable GUI flow today (only whole-
extension install/uninstall) — a registered `McpServerEntry` can still be individually
toggled through the existing MCP settings UI (`set_mcp_enabled_by_uuid`), and that toggle
survives a reinstall as described above.

### `contributes`

All arrays default to `[]` and are omitted entirely when empty on serialization —
an old manifest missing a field added in a later wave still parses (this is tested:
`protocol.rs`'s `old_style_contributes_without_events_or_subagent_extras_parses`).

| Array | Item shape |
| --- | --- |
| `sub_agents` | `{ "name": <string>, "description": <string>, "prompt"?: <string>, "model"?: <string>, "effort"?: <string>, "tools"?: [<string>] }` |
| `models` | `{ "id": <string>, "display_name": <string> }` (declared shape only — see `models:contribute` in the grants reference for the runtime catalogue) |
| `panels` | `{ "id": <string>, "title": <string>, "icon"?: <string, default "">  }` |
| `tools` | `{ "name": <string>, "description": <string>, "input_schema"?: <JSON Schema value> }` |
| `events` | `[<string>]` — event names this extension wants delivered to `on_event`. See "Events" below for the fixed vocabulary. |
| `oauth_providers` | see below |
| `tui_screens` | `{ "id": <string>, "title": <string> }` (`TuiScreenDef`) — a full-screen view koma's terminal UI renders for this extension. See "TUI screens" below. |

**`sub_agents` and the model slug binding chain.** `prompt` becomes the sub-agent's
system prompt; `model` and `effort` are a slug and an effort hint applied at spawn
time, resolved through `resolve_agent()` (`src-agent/src/app/resolve.rs`). For a
sub-agent whose `model` is a bare slug (no separately-pinned `provider_uuid` — this
is exactly the shape a manifest-declared `SubAgentDef.model` or an `agents.spawn`
override produces), the resolution order is:

1. **Ext-owned, session-scoped** — if this sub-agent belongs to an extension
   (`agent.ext_id` set), the slug is first matched against models served by THAT
   extension's own OAuth-connected providers (`ext_preferred_provider_uuids`,
   matched by `oauth_conns` uuid — key-backed `providers.register` entries are
   NOT included in this preference set), restricted to the active session's
   `session_models` overrides.
2. **Ext-owned, global** — same ext-owned restriction, against the global
   `config.models` catalogue.
3. **Any, session-scoped** — the slug matched against any session-scoped model,
   ext-owned or not.
4. **Any, global** — the slug matched against any global model.
5. **Fallback to Main** — if nothing matched, `resolve_role(Main)`: the sub-agent
   runs on the user's fully-resolved Main route instead. A UI toast is emitted at
   spawn time — `"agent '<name>' model unresolved — using main"` — whenever the
   agent (or a spawn-time override) declared a model that didn't resolve to
   anything but this fallback (`agent_model_resolves`,
   `src-agent/src/app/runtime/stream/spawn.rs`).

This is the **binding guarantee**: an extension's own sub-agents route through that
SAME extension's models first, so a same-named model elsewhere in the user's
catalogue can never hijack an extension's route — but the sub-agent still runs
either way, just on Main until the extension's account is connected and its models
are registered.

**`sub_agents[].tools`** — the tool allow-list this sub-agent installs with. Names
must match koma's selectable tool set (the same names the `/agents` editor's tool
picker offers — things like `"read"`, `"grep"`, `"glob"`, `"dir_list"`, `"edit"`,
`"write"`, `"bash"`; unknown names are dropped with a logged warning rather than
failing the extension). Omitted or empty means koma's safe read-only default
(`read`, `grep`, `glob`, `dir_list`) — the same behavior as before this field
existed. This is a **seed, not a hard override**: it applies fresh from the
manifest on every load, but the moment a user edits and saves that sub-agent from
the `/agents` dashboard, their edit persists as a session-scope override that wins
over the manifest from then on. Separately from this allow-list, every sub-agent —
manifest-declared or not — also inherits whatever MCP tools are currently
connected, exactly like the main agent does (there is no per-agent MCP picker);
this `tools` field only ever narrows koma's own built-in tools, it never affects
MCP tool availability.

**`oauth_providers`** (`OAuthProviderDef`) — each becomes a row in koma's OAuth
picker (see "OAuth providers" below for the full flow):

| Field | Type | Notes |
| --- | --- | --- |
| `id` | string | Unique within this extension. koma keys the picker row as `ext:<extension_id>:<id>` and passes `{"providerId": <id>}` on every `oauth.*` invoke. |
| `name` | string | Picker row label. |
| `method` | string | `"browser"` → `pkce` badge (a URL the user opens); `"device_code"` → `device` badge (a user code + verification URL); `"paste"` → `paste` badge; anything else falls back to the browser badge. |
| `chat_endpoint` | string, optional (W12) | The chat-completions endpoint this provider resolves to once connected. Absent → account-login-only. |
| `api_type` | string, optional (W12) | Must normalize to `"openai"` or `"anthropic"` (case-sensitive, trimmed) to count — anything else (including legacy strings like `"openai_compatible"`) normalizes to "absent", i.e. account-login-only. |
| `refresh` | `{ "token_url": <string>, "client_id": <string> }`, optional (W12) | **Ignored in v1** — the extension owns the whole token lifecycle; koma never refreshes on its own yet. Declared now so a manifest that specifies it round-trips without a later re-touch. |

A connection is a usable **model-provider gateway** if and only if both
`chat_endpoint` and a recognized `api_type` are present (`OAuthConn::ext_model_route()`,
`src-agent/src/model/app_config.rs`); otherwise the connection is account-login-only
and any model resolution referencing it is dangling.

### Daemon vs. oneshot

An extension is either a **daemon** or a **oneshot**, and the difference is about
how long it lives. A daemon is a long-running process koma launches and keeps
talking to for as long as it's installed (and, per the auto-start rules under
"Lifecycle" below, running) — the natural shape for anything with a panel to keep
alive, state to hold between calls (a delegated OAuth flow's pending device code!),
or events to keep listening for. A oneshot is launched, asked one thing, and
expected to answer and exit.

Delegated OAuth (`oauth.begin`/`oauth.poll`/`oauth.cancel`) specifically **requires**
`kind: "daemon"` — the begin→poll handshake holds state across invokes that a
respawned-per-invoke oneshot could never remember.

---

## Grants reference

Every ext→koma `Call` verb is gated by exactly one grant, checked in
`broker.rs::required_grant()` before the method dispatches — denied or unknown
methods reply immediately with `{"error": "grant denied: <method> requires
<grant>"}` or `{"error": "unknown method: <method>"}` and never touch session
state. `agents:orchestrate` implies `agents:read`; nothing else implies anything.

### `agents:read` — read-only sub-agent status

Satisfied by `agents:read` OR `agents:orchestrate`. All three verbs scope strictly
to the calling extension's OWN `ExtAgentRegistry` — never the raw session
`subagents` collection, never another extension's or the user's own agent IDs.

| Verb | Params | Success | Errors |
| --- | --- | --- | --- |
| `agents.list` | `{}` | `[{ "agentId", "agent", "status": "running"\|"done"\|"killed"\|"error" }, ...]`; a still-queued entry is `{ "agentId", "status": "queued" }`; a gone/closed one is `{ "agentId", "status": "gone" }` | — |
| `agents.status` | `{ "agentId": <u64 or numeric string> }` | `{ "agentId", "agent", "status", "liveTextLen": <usize> }` (queued: `{agentId,status:"queued"}`) | missing `agentId` → `"agents.status requires an 'agentId'"`; unknown id → `"unknown agentId: <id>"`; session closed → `"session closed"` |
| `agents.result` | `{ "agentId" }` | done: `{ "agentId","status":"done","output": <string> }`; error: `{...,"status":"error","error":<string>}`; also `"running"`/`"queued"`/`"killed"` markers | same as `agents.status` |

### `agents:orchestrate` — spawn/steer/stop sub-agents

| Verb | Params | Success | Errors |
| --- | --- | --- | --- |
| `agents.spawn` | `{ "task": <string, required non-empty>, "agent"?: <string, default `"general"`>, "model"?: <string slug>, "effort"?: <string>, "workspace"?: <string, absolute path>, "notify"?: <bool, default false> }` | `{ "agentId": <u64, ext-facing>, "status": "spawned" }` or, once the 5-slot `MAX_SUBAGENTS` cap is full, `{ "agentId", "status": "queued" }` | empty task → `"agents.spawn requires a non-empty 'task'"`; no foreground session → `"no active session"`; unresolvable agent/client → `"failed to spawn agent '<agent>' (no client/session or unknown agent)"`; rejected `workspace` → `"workspace '<path>' could not be resolved: <io error>"` or `"workspace '<path>' is outside the session's allowed workspace root(s)"` |
| `agents.kill` | `{ "agentId" }` | `{ "killed": true }` — idempotent; killing an already-terminal agent still returns `true` without re-firing a terminal event | missing/unknown/closed → same shapes as `agents.status` |
| `agents.send` | `{ "agentId": <u64 or numeric string>, "message": <string, required non-empty> }` | `{ "sent": true }` — the message is injected as a follow-up USER turn, delivered at the sub-agent's next TURN BOUNDARY (never mid-stream); a still-queued agent stashes it and returns `{ "sent": true, "status": "queued" }` (delivered at promotion) | empty message → `"agents.send requires a non-empty 'message'"`; a terminal (done/killed/error) agent → `"agent is terminal"`; missing `agentId`/unknown id/closed session → same shapes as `agents.status` |

Spawn targets the ACTIVE (foreground) session, through the same `spawn_or_queue`
path the model's own `task` tool uses, non-detached with no `tool_call_id` (so
completion never auto-wakes the chat model on its own). `agents.send` STEERS a
sub-agent already spawned this way: its `message` lands in the agent's isolated
history as a fresh user turn at the next turn boundary (the same mechanism the
main agent's own `task_send` tool uses), so you can add context or correct course
without killing and re-delegating — the agent's result still arrives via
`agents.result` / the `agents.done` event as usual. `notify: true` additionally
arms a private `agents.done` event to the SPAWNING extension on terminal state —
see "Events" below; this is independent of `contributes.events`. The returned
`agentId` is a fresh id from this extension's own registry, permanently bound to a
stable session UUID — it is never reused across sessions or shared with any other
extension.

`workspace` (both `agents.spawn` and the LOCAL branch of `sessions.spawn_into` —
see below) is an OPTIONAL absolute path that confines the spawned sub-agent's
file tools + `bash` to a single directory INSTEAD OF the whole session
checkout. **Absent = session default**: with no `workspace`, the sub-agent
inherits the session's own workspace root(s) exactly as before this param
existed. When present, koma canonicalizes it and requires it to resolve INSIDE
(or equal to) one of the session's EXISTING workspace roots — checked by path
component, never a raw string prefix, so a sibling directory whose name merely
starts with a root's name (e.g. `/a/bc` against a root `/a/b`) is correctly
rejected. This is a sandbox trust boundary: a `workspace` that fails to
canonicalize (doesn't exist) or resolves outside every root REJECTS the whole
spawn outright — koma never silently falls back to the wider session
workspace. A typical use is a git-worktree "desk" per delegated task (e.g. the
Workflow extension confining each worker under its own `~/.koma-workflow/...`
subtree) rather than handing every sub-agent the full session tree.

### `sessions:manage`

| Verb | Params | Success | Notes / errors |
| --- | --- | --- | --- |
| `sessions.list` | `{}` | `[{ "id": <uuid>, "name": <string\|null>, "workdir": <string>, "live": <bool>, "working": <bool> }, ...]` | A registry snapshot merged with a live-daemon probe sweep — **v1 limit: no cross-daemon polling**, this is a point-in-time merge, not a subscription. |
| `sessions.create` | `{ "workdir"?: <string>, "name"?: <string> }` | `{ "id": <uuid> }` | `workdir` absent/blank → daemon's own launch cwd; present → must be an absolute EXISTING path, else `"workdir must be an absolute existing path"`. Spawns a detached `koma --daemon --session <uuid>`, connect-polled with a 3s timeout. A `name` set failure (registry lag) is retried once after 500ms but never fails the create itself. |
| `sessions.switch` | `{ "session": <string uuid, required non-empty> }` | Local (live in this daemon): `{ "ok": true, "delivery": "local" }`. Cross-daemon: latches a one-shot signal for attached clients → `{ "ok": true, "delivery": "signaled" }` (the actual attach is the CLIENT's job — GUI wires it, TUI may ignore it). | missing `session` → `"sessions.switch requires a 'session'"` |
| `sessions.spawn_into` | `{ "session": <string, required>, "task": <string, required non-empty>, "agent"?, "model"?, "effort"?, "workspace"?, "notify"? }` | Local session: identical shape to `agents.spawn` (including `workspace`, see above), and IS tracked in `ExtAgentRegistry`. Cross-process: `{ "status": "sent", "session": <uuid> }` — fire-and-forget, **NOT tracked**, no `agentId`, no polling possible; `workspace` is **not honored on the cross-process branch** (v1: only the LOCAL branch threads it through — a cross-daemon `workspace` is silently ignored, same as any other unrecognized field on that transport). | missing `session` → `"sessions.spawn_into requires a 'session'"`; empty task → `"sessions.spawn_into requires a non-empty 'task'"`; target down → `"session not live"`; other transport error → `"target daemon incompatible or unavailable"`; rejected `workspace` (local branch only) → same two messages as `agents.spawn` above |

### `chat:prompt`

| Verb | Params | Success | Limits / errors |
| --- | --- | --- | --- |
| `chat.prompt` | `{ "text": <string, required non-empty> }` | `{ "queued": <new buffer length> }` | **16KB cap** (byte length) → `"prompt exceeds 16KB"`; empty/whitespace → `"chat.prompt requires a non-empty 'text'"`; **queue cap 5** → `"prompt queue full (5)"` (an exact repeat of the buffer's last entry is silently deduped instead, returning the unchanged length); **turn budget 10** — once this extension has injected 10 turns without real user activity in between, `"extension turn budget exhausted; waiting for user activity"` (resets on genuine user input); no foreground session → `"no active session"` |

This does **not** inject immediately — it buffers onto the active session's
`pending_ext_prompts`, and the event loop injects it as one synthetic user turn the
next time that session goes idle, so it can never corrupt an in-flight turn's
tool-call/tool-result ordering.

### `models:invoke`

| Verb | Params | Success | Limits / errors |
| --- | --- | --- | --- |
| `models.invoke` | `{ "prompt": <string, required non-empty>, "role"?: `"main"` \| `"awareness"` \| `"safeguard"` \| `"compactor"` \| `"planner"` (default `"main"`), "system"?: <string>, "format"?: `"json"` }` | `{ "output": <string>, "model": <model id string> }` | **32KB cap** on `prompt` → `"prompt exceeds 32KB"`; unrecognized role → `"unknown role"` (never silently falls back to Main); no route → `"no usable route for role <role>"`; route not dispatchable → `"role <role> route is not dispatchable (Anthropic-compatible not wired)"`; no usable auth → `"role <role> route has no usable auth"`; no client → `"no llm client"`; stuck backend → `"model call timed out"` after koma's internal **330s** budget (deliberately under the reader's **360s** verb-scoped cap for this method, so you always get a value back) |

`format: "json"` pins strict OpenAI-dialect JSON output
(`response_format: {"type":"json_object"}`) on the request. **Dialect caveat:**
this only takes effect when the resolved `role`'s route speaks the
OpenAI/OpenRouter chat-completions dialect. Routes on the Codex
(ChatGPT-subscription Responses API) or Anthropic-compatible dialects have no
`json_object` wire equivalent — for those, `format` is silently IGNORED (never
an error), and you get today's free-form text back. Any value other than the
literal string `"json"`, or the field absent, is also today's free-form
behavior.

### `context:publish`

| Verb | Params | Success | Limits |
| --- | --- | --- | --- |
| `context.set` | `{ "text": <string> }` | `{ "ok": true }` | **8KB cap per extension** (byte length) → `"context exceeds 8KB"`. Empty/whitespace `text` CLEARS the entry instead of erroring (still `{"ok":true}`). |
| `context.clear` | `{}` | `{ "ok": true }` | Idempotent — clearing an absent entry still succeeds. |

Published text rides the system prompt's volatile tail on every turn, appended
AFTER the prompt-cache split, so publishing here never busts the cached prompt
head. Keyed by the calling extension's own `ext_id` — one entry per extension.

### `models:contribute`

Shared by `models.register`/`.unregister` AND `providers.register`/`.unregister` —
distinct from `models:invoke`; neither direction unlocks the other.

**`models.register`** — `{ "models": [{ "id": <string, ≤200 chars>, "name": <string, ≤200 chars>, "default"?: <bool> }, ...] (1–100 entries), "provider"?: <string uuid> }`

- Validation is atomic: a bad entry anywhere rejects the WHOLE batch. Empty array →
  `"models.register requires at least one model"`; `>100` → `"too many models (max
  100)"`; empty/overlong id or name → `"each model requires a non-empty 'id' and
  'name'"` / `"model id/name too long (max 200)"`; more than one `default: true` →
  `"multiple defaults in one call"`.
- **Anchor resolution** picks which of the caller's own provider/conn uuids the
  models attach to. Explicit `provider` must be caller-owned and model-capable, else
  `"provider is account-login only"` or `"provider not owned by this extension"`.
  Omitted `provider`: exactly one eligible anchor → used automatically; zero →
  `"provider is account-login only"` or `"no connected oauth account for this
  extension"`; more than one → `"multiple providers; specify provider uuid"`.
- Dedupe key is `(provider_uuid, model_id)`: re-registering the same pair UPDATES
  the display name in place and KEEPS the existing uuid — anything already bound to
  it (a sub-agent, a picker selection) keeps resolving.
- Success: `{ "registered": <n>, "uuids": [<uuid>, ...] }`, plus `"defaultUuid":
  <uuid>` if one entry was flagged `default: true`.
- **"default" vacuum-fill semantics**: a `defaultUuid` records the extension's
  preferred model (`config.ext_preferred_models`), then vacuum-fills it onto the
  Main role ONLY if Main is currently unset or koma-free-only in BOTH the global
  catalogue and the active session's overrides. First vacuum-fill wins — once a
  real model holds Main anywhere, a later extension's default only ever surfaces
  as a `recommendedBy` picker hint, it never displaces what's there. A successful
  vacuum-fill also strips any session-local `/free` override pointing at koma-free
  (so `/free` can't shadow the new Main) and sets a toast: `"model {name} set by
  extension {ext_id}"`.

**`models.unregister`** — `{ "ids"?: [<string, matches model_id or uuid, case-insensitive>] }`. Absent `ids` removes ALL caller-owned entries; present removes only
matches. **Ownership wall**: only ever removes models anchored to a provider/conn
this extension owns — another extension's or the user's own models are untouchable.
Success: `{ "removed": <n> }`.

**`providers.register`** (key-backed gateways) — `{ "name": <string, ≤200 chars>, "endpoint": <http(s) URL>, "api_type": "openai"\|"anthropic", "key": <string, ≤4096 chars> }`

- `name` empty/overlong → `"providers.register requires a non-empty 'name'"` /
  `"provider name too long (max 200)"`; bad endpoint → `"endpoint must be a valid
  http(s) URL"`; bad `api_type` → `"api_type must be 'openai' or 'anthropic'"`;
  empty/overlong key → `"providers.register requires a non-empty 'key'"` / `"key
  too long (max 4096)"`.
- **Key rotation**: dedupe key is `(caller ext_id, name)` — re-registering the
  same name UPDATES endpoint/key/api_type IN PLACE and KEEPS the existing uuid, so
  a leaked key can be rotated without re-registering the models bound to it.
- Success: `{ "uuid": <stable uuid> }`.

**`providers.unregister`** — `{ "ids"?: [<string, matches uuid or name, case-insensitive>] }`. Ownership wall same as `models.unregister`. **Orphan-model sweep**:
removal ALSO purges every model anchored to the removed provider (the exact same
sweep the uninstall path uses — `config.remove_models_by_providers`) so no model
is ever left pointing at a deleted provider. Success: `{ "removed": <n> }`.

**Host-enforced delete guard (separate from the sweep above)** — this is about the
OTHER direction: stopping a *user*, not the owning extension, from deleting or
dropping a key-backed provider (`ProviderConn::ext_id` set). Only the extension
itself (via `providers.register`/`providers.unregister`) or an uninstall may ever
remove one; every user-facing config-editing surface refuses it explicitly,
independently, in THREE places (plus a fourth belt-and-braces restore), so no
single UI can orphan an extension's gateway:

1. Attached daemon path — `requests_config.rs::delete_provider` rejects with a
   structured `DaemonEvent::Error` (`"managed by extension {id} — uninstall to
   remove"`) before any mutation.
2. Pre-session / detached path — `host_config.rs::apply_global_config_req`'s
   `DeleteProvider` arm refuses in place (no save, no push) and logs the same
   reason (this path has no structured-error reply channel).
3. TUI Settings screen — `settings/state/provider_ops.rs::prov_arm_or_delete`
   refuses to even ARM the delete, surfacing a footer message instead.
4. TUI Settings **bulk save** (`runtime/actions/settings.rs`) — the whole-Vec
   provider-drafts replace RESTORES every ext-managed provider verbatim from the
   existing config after building the draft list, so a stale/older client that
   dropped or edited an `ext_id` entry can't silently delete or mutate it either.

---

## Events

koma fans out a small, LOCKED set of event names to any RUNNING extension that
listed that exact name in its manifest's `contributes.events` — nothing else gates
delivery (no grant required; `contributes.events` is independent of `requires`).
Delivery is **best-effort, fire-and-forget**: the wire frame (`KomaMsg::Event {
name, params }`) carries no `id` and expects no `Result` reply, there is no retry,
and a not-currently-running extension simply doesn't get it (no queueing).

| Event | Payload | Fires when |
| --- | --- | --- |
| `subagent.done` | `{ "session": <uuid>, "subagentId": <usize>, "agent": <string>, "status": "done"\|"error"\|"killed" }` | Any sub-agent's `Running → {Done\|Error\|Killed}` edge, broadcast to EVERY subscribed extension regardless of who spawned it. |
| `agent.turn_end` | `{ "session": <uuid> }` | A session's raw `working → !working` edge — fires on every turn boundary whether or not any client is watching. |
| `session.foreground_change` | `{ "session": <uuid> }` | Any in-daemon foreground switch (daemon-hub `SwitchForeground`, TUI session-hub Enter, post-close reassignment) — fires once per switch, no double-fire. In daemon-per-session, a switch to a session owned by a DIFFERENT daemon is invisible to THIS daemon's extensions. |

**`agents.done`** is a related but DIFFERENT mechanism, worth calling out
separately: `{ "agentId": <u64, the EXT-FACING id from agents.spawn>, "status":
"done"\|"error"\|"killed", "error"?: <string> }`, delivered ONLY to the single
extension that spawned that agent with `agents.spawn { "notify": true }` — and
delivered regardless of whether `"agents.done"` appears in that extension's
`contributes.events` at all. A spawn with `notify: false` (today's default) gets no
`agents.done` — only the broadcast `subagent.done` others may also be subscribed to.
See `event-watcher-daemon` for the subscribed-broadcast side and
`fleet-board-daemon`/`orchestrator-daemon` for the `notify: true` side.

The `"error"` field is ADDITIVE. It is present when `"status": "error"` — carrying
the sub-agent's failure text (the same string `agents.status`/`agents.result` would
report), so a `notify: true` spawner learns *why* its sub-agent died without a
separate poll — and ALSO on a `"killed"` event when the daemon is shutting down
(including the build-skew auto-restart after an upgrade), where it reads
`"error": "daemon restart"`: an extension that wants restart-resilience should treat
that reason as RESPAWNABLE (re-spawn the agent once reconnected to the fresh daemon)
rather than as a real failure. A `"done"` payload never carries the field, and a
`"killed"` from an explicit `agents.kill` carries none either. Older extensions that
don't read the field are unaffected; the full terminal payload — including a `"done"`
sub-agent's report text, which never travels over this event — remains available via
the `agents.result` pull path.

Separately from delivery, every sub-agent an extension spawns through
`agents.spawn` is marked ext-owned, and an ext-owned agent's completion is
**completely silent in the human's chat** — no fold note, no nudge, nothing
pushed to the transcript — regardless of `notify`. This is unlike a human-run
`/task`, whose completion folds a compact checkmark line into the chat. The
extension gets its result through `agents.result` / `agents.done` (when
`notify: true`) instead; usage and the persisted sub-agent record are still
tracked either way.

No event payload ever carries a sub-agent's report text or transcript — only ids,
names, short status labels, and (on `agents.done` with `"status": "error"`, or a
daemon-shutdown `"status": "killed"`) the short reason string described above. There is no batching, coalescing, or
rate-limiting on delivery; each trigger fans out individually and synchronously at
the point of the state transition.

---

## Panel bridge

A panel's `ui/index.html` (and whatever else it loads) is served straight off the
installed extension's own directory at `koma://extension/<id>/<path>` — a
DIFFERENT origin from koma's own chrome (`koma://localhost/`). The iframe has no
`sandbox` attribute (that would break the custom scheme) but cannot script the host
origin, does not inherit `window.ipc`/`window.__komaClient`, and has no documented
host IPC beyond the bridge described here.

The exact origin panels load from is **platform-dependent** — `koma://extension/<id>/…`
on macOS/Linux, but `http://koma.extension/<id>/…` on Windows (WebView2 can't
register a real custom scheme, so the host serves the same content over an http
fake-domain). Never hardcode this origin in your panel: reference your own assets
with **relative** paths (`<script src="koma-panel.js">`, not an absolute URL) and let
the `koma-panel.js` helper handle host messaging (it posts to `window.parent` with
`targetOrigin: '*'`, so it works unchanged on every platform).

### The envelope, both directions, verbatim

Panel → host:
```json
{ "koma": "panel", "v": 1, "kind": "msg", "reqId": "<string>", "payload": <any> }
```
posted to `window.parent` with `targetOrigin: '*'` (the two sides are different
origins, so there is no meaningful same-origin target to pass instead).

Host → panel, reply (correlates to a `reqId`):
```json
{ "koma": "host", "v": 1, "kind": "reply", "reqId": "<string>", "ok": <bool>, "payload"?: <any>, "error"?: "<string>" }
```

Host → panel, unsolicited push (no `reqId` — the daemon extension called
`Koma::panel_push`, no request behind it):
```json
{ "koma": "host", "v": 1, "kind": "push", "payload": <any> }
```

### Attribution: never trust the payload

All panel iframes load off the same LOGICAL koma-extension origin family
(`koma://extension/<id>/...`), so a hostile or buggy panel could, in principle,
claim to be any `extId`/`panelId` inside its own message payload. The GUI-side
bridge (`src-webgui/src/lib/panelBridge.ts`) never trusts that: every inbound
`message` event is resolved through a registry keyed by the iframe's actual
`Window` object (captured off `iframe.contentWindow` when the panel frame mounts),
never by anything the message claims about itself. `origin` checks are
defense-in-depth only, not the security boundary.

### The size-cap chain

Three independent caps apply at three different layers of the same pipe — they are
NOT the same number wearing different hats:

| Layer | Cap | Enforced by |
| --- | --- | --- |
| Panel → host (browser `postMessage` payload) | 256 KiB (`262_144` chars, JSON-stringified) | `panelBridge.ts` — over cap, the host replies `{ok:false, error:'payload too large'}` locally and never forwards to the daemon |
| Extension → panel (`Koma::panel_push` payload, SDK-side) | 1 MiB (`1_048_576` bytes) | `src-extension/src/sdk.rs` — over cap, the push is logged and DROPPED, not sent |
| Daemon ↔ extension-process wire frame (the underlying NDJSON transport, not panel-specific) | 4 MiB (`4 * 1024 * 1024` bytes) | `src-agent/src/app/ext/wire.rs` — over cap is FATAL to the connection |

Separately, the daemon's outbound panel-push queue (not a byte cap — a count cap)
holds at most 256 pending pushes, drop-oldest, so a daemon that pushes faster than
clients drain never grows unbounded memory.

### Auto-start and the round trip

Opening a panel tab does **not** eagerly start anything — the iframe just loads
static assets off disk. The FIRST real `panel.msg` the panel's own JS sends is what
lazily auto-starts the backing daemon extension (if it's enabled, daemon-kind, and
not already running — "a panel being open implies user intent; a blocking start is
fine on the pool"). With no daemon attached at all, the GUI-side bridge replies
`{error: 'no active koma session'}` locally without ever reaching the daemon.

Full round trip for a request (`kind: 'msg'`):

```text
panel iframe JS
  KomaPanel.send(payload) -> postMessage({koma:'panel',v:1,kind:'msg',reqId,payload}, '*')
window `message` listener (panelBridge.ts)
  attribute sender via registry (never via payload) -> 256KiB cap check
Zustand store -> wry IPC -> Rust GUI host
  GuiReq::ExtPanelMsg decode -> forward as ClientRequest::ExtPanelMsg (no cap here)
attached daemon (requests_ext.rs)
  panel_start_decision -> auto-start if needed -> invoke "panel.msg" over the
  4MiB-capped wire, 10s timeout, params { "panelId": <id>, "payload": <payload> }
extension daemon
  on_invoke("panel.msg", params) -> ExtMsg::Result
daemon -> StoreReply::PanelReply -> DaemonEvent::ExtPanelReply
GUI host re-pushes -> store/koma.ts -> postToPanel
panel iframe JS receives {koma:'host',v:1,kind:'reply',reqId,ok,payload,error}
```

The unsolicited push lane (`Koma::panel_push` on the daemon side) is identical but
skips the `reqId` correlation entirely and is fire-and-forget in both directions —
capped at 1 MiB before it ever leaves the extension process, fanned out to every
attached client as `DaemonEvent::ExtPanelPush`.

### `KomaPanel` helper

`src-extension/example/fleet-board-daemon/ui/koma-panel.js` is a small,
dependency-free, copyable implementation of the panel side of the envelope above —
drop it into any panel's `ui/` directory as-is:

```js
KomaPanel.send({ action: 'spawn', task: 'demo card' })   // -> Promise<payload>, default 15s timeout
  .then(reply => /* ... */)
  .catch(err => /* ... */);

KomaPanel.onPush(payload => { /* handle an unsolicited push */ });
```

`send()` assigns and tracks its own `reqId`, resolves/rejects the returned promise
when the matching `reply` envelope arrives (or on timeout), and `onPush()`
registers a handler fanned out on every `push` envelope. See
`fleet-board-daemon/ui/index.html` for it wired up to a real "Spawn card" button.

### Theme

Panels are theme-aware: the GUI host pushes the current palette to every registered
panel automatically, and a panel can also pull it on demand. This is a THIRD,
distinct sub-contract on top of the `msg`/`push` envelopes above — its own `kind`
values, so a panel never confuses a theme repaint with an extension-originated
push.

Host → panel, unsolicited (a live palette change, AND once immediately when the
panel iframe registers — so a panel opened mid-session doesn't have to wait for the
next theme switch to paint itself correctly):
```json
{ "koma": "host", "v": 1, "kind": "theme", "payload": { "palette": { /* roles */ }, "name": "<string>", "dark": <bool> } }
```

Panel → host, one-shot query (works even detached — no active koma session
required, since theme is a pure UI concern):
```json
{ "koma": "panel", "v": 1, "kind": "theme?", "reqId": "<string>" }
```
answered as an ordinary `reply` envelope with the same `{ palette, name, dark }`
payload shape.

`palette` carries the same nine roles the chat chrome itself paints with (host
`PushPalette`, `src-agent/src/app/runtime/client/push_rows.rs`), each a `#rrggbb`
string:

| Role | Meaning |
| --- | --- |
| `bg` | Canvas background |
| `fg` | Primary text |
| `accent` | Highlights — rails, active state, primary buttons |
| `dim` | Muted text / secondary borders |
| `panel` | Raised surface — cards, bands, overlays |
| `warn` | Amber warning cues |
| `success` | Green success cues |
| `info` | Blue info cues |
| `error` | Red error cues |

`name` is the active entry's key in the host's named-palette registry
(`view::theme::PALETTES` — round-trips as a `SetTheme { name }` request from the
Settings tab, not something a panel sends). `dark` is a bool the host derives from
`bg`'s relative luminance (`project_config.rs`'s `palette_is_dark`) — cheaper than
every panel re-deriving it from a hex string, and guaranteed to agree with the
host's own dark/light classification.

`koma-panel.js` wraps both directions:

```js
KomaPanel.onTheme(theme => {
  // theme = { palette: {...}, name: 'dark', dark: true }
  document.documentElement.style.setProperty('--koma-bg', theme.palette.bg);
  document.documentElement.style.setProperty('--koma-fg', theme.palette.fg);
});

KomaPanel.getTheme()               // -> Promise<{ palette, name, dark }>, default 15s timeout
  .then(theme => /* ... */);
```

`onTheme()` fires on every host `theme` push (register-time delivery + live
changes) AND once on the module's own initial `getTheme()` query it fires on load —
so a handler registered any time after the script loads still gets the CURRENT
theme immediately, not just the next change. See `fleet-board-daemon/ui/index.html`
for it wired up to `--koma-*` CSS custom properties with the panel's original
hardcoded colours kept as fallbacks.

---

## TUI screens (`contributes.tui_screens`)

A panel needs a webview, and koma's terminal UI has none — so an extension that
wants a full-screen view of its own when the user is running the TUI instead of the
GUI declares `contributes.tui_screens` instead of (or alongside) `contributes.panels`.
The whole exchange reuses the panel bridge's `panel.msg` invoke + `panel.push` notify
verbs VERBATIM — the screen id rides as `panelId` — so this costs the wire protocol
nothing new: a GUI panel message and a TUI screen message are the same bytes on the
socket, distinguished only by a `kind` tag inside the opaque payload. The
koma-side authority for everything in this section is the module doc comment on
`src-agent/src/app/ext/screen.rs`; this section restates it for the SDK-facing side.

### The manifest field

```json
"contributes": {
  "tui_screens": [
    { "id": "demo", "title": "TUI Demo" }
  ]
}
```

Each entry is a `TuiScreenDef { id: <string>, title: <string> }` (both required, no
optional fields — see the manifest reference table above). `id` is the stable id
koma passes back as `panelId` on every invoke for this screen and matches on every
`panel.push` targeting it; `title` is the human-facing label — it's the row shown
under the extension's entry in the terminal UI's `/extension` detail view, AND the
header shown until (and unless) the extension's first `Screen` reply supplies its
own `title`. Declaring more than one entry gives the user more than one selectable
row; each is its own independent screen with its own `id`.

### The payload envelopes

Host → ext (koma invokes `panel.msg`, `panelId` = the screen's `id`):

| verb | payload | when |
| --- | --- | --- |
| open | `{ "kind": "tui-open" }` | the screen is opened (the row was selected in `/extension` detail) |
| select | `{ "kind": "tui-select", "item": "<menu item id>" }` | Enter on a menu row |
| close | `{ "kind": "tui-close" }` | Esc, or the screen is otherwise torn down — best-effort; the reply is ignored |

Ext → host reply (the `Result` value of the `panel.msg` invoke):

- `{ "screen": <Screen> }` — render this screen.
- `{ "close": true }` — pop back to the extension's detail view (the same
  destination `tui-close` leads to, just extension-initiated instead of
  user-initiated — see `tui-demo-daemon`'s "close" menu row for exactly this).

Ext → host push (the extension calls `Koma::panel_push(screen_id, ...)` — a
fire-and-forget `panel.push` notify, `panel_id` = the screen's `id`):

- `{ "kind": "tui-screen", "screen": <Screen> }` — folded LIVE into the open screen,
  no user action required. This is how a screen shows something changing on its own
  (a counter ticking, a job finishing) instead of only reacting to input.

### The `Screen` / `Node` model

```text
Screen = { "title"?: <string>, "body": [<Node>], "footer"?: <string> }
Node   = { "t": "text",    "text": <string> }
       | { "t": "kv",      "k": <string>, "v": <string> }
       | { "t": "divider" }
       | { "t": "menu",    "items": [ { "id": <string>, "label": <string> } ] }
```

`title` and `footer` are optional; `body` is the ordered list of content nodes koma
renders top to bottom. A node with an unrecognized `"t"` is SKIPPED, not an error —
forward-compat for a future node type an older extension SDK doesn't know about yet.
There is no per-node styling; the terminal UI decides how each `t` paints.

### Host-side navigation

koma's terminal UI owns exactly one piece of state for an open screen: the menu
cursor. It is NOT told which row means what — it just walks the UNION of every
`{ "t": "menu" }` node's `items`, in body order, across every menu node in the
screen (so a `Screen` with two menu nodes separated by a divider still has one
continuous, ↑/↓-navigable list):

- **↑ / ↓** move the cursor over that union, locally, with no round trip to the
  extension.
- **Enter** sends `tui-select` with the id of the item under the cursor, and shows a
  one-line "loading…" state until the reply lands (Enter is inert while a reply is
  already in flight, and inert on a screen with no menu at all).
- **Esc** fires a best-effort `tui-close` (the reply is discarded either way) and
  pops back to the extension's `/extension` detail view — the SAME thing a `{
  "close": true }` reply does, just triggered by the user instead of the extension.

Everything else — what the screen says, what the menu options mean, whether an
action changes anything — is entirely up to the extension; koma has no opinion on
it beyond rendering the `Node`s it's handed.

### The push channel and its caps

A screen's live-update push is the EXACT SAME `Koma::panel_push` a GUI panel uses
(see "The size-cap chain" under "Panel bridge" above for the full three-layer
byte-cap table and the 256-pending drop-oldest queue cap) — nothing about the caps
changes just because the receiving surface is a terminal screen instead of a
webview panel. The only TUI-specific rule sits above those caps: a push is only
folded into a screen that's currently OPEN for that `(ext_id, screen_id)` pair; a
push for a screen nobody has open is simply not applicable to any client-side state
and has nowhere to land.

Round-trip budget: `tui-open`/`tui-select` are each bounded at **30s** (shorter than
the 120s default `panel.msg` timeout elsewhere — a screen redraw should be prompt,
and a hung extension must not leave the terminal UI spinning for two minutes). Same
auto-start rule as a panel: opening a screen implies user intent, so a not-yet-running
ENABLED daemon extension is started lazily on the first `tui-open`; a disabled
extension, a `oneshot`-kind extension (no persistent backend to hold a screen's
state against), or an extension that isn't installed at all all surface as the
screen's one-line error `"extension not available"` instead.

### No screens declared: detail + uninstall only

An extension with an empty (or absent) `contributes.tui_screens` shows nothing extra
in `/extension`'s detail view beyond its own info and the uninstall action — no
screen list, no ↑/↓-selectable rows, no cursor. This is the same additive/optional
shape every other `contributes` array gets: an extension predating this field, or
one that simply has no use for a full-screen view, parses and runs completely
unchanged.

### Walkthrough: `tui-demo-daemon`

`src-extension/example/tui-demo-daemon` is the runnable reference for everything
above: one screen (`id: "demo"`) showing a counter as three `kv` rows (`counter`,
`last action`, `uptime`), a `divider`, and a `menu` with four items — `increment`,
`reset`, `refresh`, `close`. Opening it (`tui-open`) shows the current state without
changing it; selecting a menu item mutates a small shared `State` behind a
`std::sync::Mutex` and replies with the freshly-built `Screen`; selecting `close`
replies `{ "close": true }`. A driver thread — the same thread shape
`fleet-board-daemon` uses for the reasons the deadlock rule forces — sleeps 5
seconds at a time and calls `Koma::panel_push("demo", { "kind": "tui-screen",
"screen": ... })`, so the `uptime` row visibly advances even if nobody touches the
menu. Run `cargo run -p tui-demo-daemon` for the demo transcript (a scripted
`tui-open` plus one immediate push, since the demo harness has no live socket to
keep a 5-second loop meaningful against); install it for real and open the "TUI
Demo" row from its `/extension` detail view to see the live screen for real.

---

## OAuth providers (delegated flow)

koma delegates a login's ENTIRE flow to the backing extension over three
`on_invoke` methods, each carrying `{ "providerId": <the provider's manifest id> }`.
koma never sees the extension's client secret; it only relays progress phases and
stores whatever token comes back.

| Invoke | Extension replies | koma phase | Notes |
| --- | --- | --- | --- |
| `oauth.begin` | `{"url": "https://..."}` | `waiting_url` | Browser method. koma does NOT auto-open the URL. |
| `oauth.begin` | `{"userCode": "...", "verificationUrl": "..."}` | `waiting_code` | Device-code method; device code wins if both are somehow present. |
| `oauth.begin` | `{"error": "..."}` | `failed` (terminal) | |
| `oauth.poll` | `{"status": "pending"}` | (still waiting) | Polled roughly every **3s**. |
| `oauth.poll` | `{"status": "success", "token": {"access_token": <required>, "refresh_token"?, "expires_at"?, "email"?, "label"?}}` | `success` | Empty `access_token` on a `success` status is treated as `failed`: `"extension reported success without an access_token"`. |
| `oauth.poll` | `{"status": "failed", "error"?: "..."}` | `failed` | Missing `error` defaults to `"extension OAuth failed"`. |
| `oauth.cancel` | anything (ignored) | — | Best-effort teardown; fire-and-forget with a 2s budget. |

Budgets: each individual `begin`/`poll` invoke is bounded at **25s**; the WHOLE
begin→success loop is bounded at **5 minutes**, after which koma gives up with
`failed: "extension OAuth timed out"`. `oauth:contribute` gates whether an
extension's `oauth_providers` even surface as picker rows and whether these three
invokes are ever sent — it gates NO ext→koma `Call` verb (it's the one exception
noted in the grants overview above).

**Requires `kind: "daemon"`** — the begin/poll handshake needs a pending device
code (or equivalent) held in memory ACROSS invokes, which a respawned-per-invoke
oneshot cannot do.

### Account-login vs. gateway

A provider that only declares `id`/`name`/`method` is account-login-only: the
resulting connection is stored and that's the end of the story in v1 — useful for
"sign in with X" without X being a model provider at all.

A provider that ALSO declares `chat_endpoint` + a recognized `api_type` becomes a
resolvable model-provider gateway once connected. The full walkthrough, once
`oauth.poll` reports `success` (driven from the DRIVER THREAD — never from
`on_invoke`/`on_event`, per the deadlock rule):

1. **connect** — the token lands as an `OAuthConn` with `ext_id` set to this
   extension and `chat_endpoint`/`api_type` stamped from the manifest.
2. **`models.register`** — register the models this account can serve (see the
   `models:contribute` grant reference above for the full request/response shape,
   the dedupe/update-in-place semantics, and the 100-model/200-char caps).
3. **vacuum-fill / `recommendedBy`** — a `default: true` entry either silently
   becomes the user's Main (if Main is unclaimed) or surfaces as a `recommendedBy`
   hint in the model picker (if something already holds Main) — see the same grant
   reference section for the exact "first vacuum-fill wins" rule.
4. **sub-agent binding chain** — any of this extension's `contributes.sub_agents`
   whose `model` slug matches now resolves through THIS connection first (the
   ext-owned-first resolution order under "Manifest reference" above), rather than
   falling through to Main.

`oauth-demo-daemon`'s doc comments walk through steps 2–3 (and the sibling
`providers.register` path for a static-key gateway instead of an OAuth-riding one)
as commented, non-executed code — read them for the exact call shapes with the
error modes inline.

---

## Lifecycle

### Install

Verifies the zip's SHA-256 then an Ed25519 signature over it before any disk write;
rejects unsafe zip paths; unpacks under `~/.koma/extensions/<id>/`; persists an
enabled registry entry. `kind: "daemon"` extensions are started immediately after a
successful install (one of four auto-start triggers — see below). Any declared
`mcp_servers[]` are auto-registered into the MCP catalogue (see "`mcp_servers`" above)
in the SAME config mutation as the registry upsert, before the live reload:

- **Attached daemon** (`requests_ext.rs::finish_install`) — one `save_and_reload_mcp`
  call persists both the registry entry and the registered server rows, then
  reconnects the LIVE session `McpManager` from the just-saved set.
- **Detached GUI host** (`store_host.rs::finish_install_detached`) — no live
  per-session `McpManager` exists pre-session, so after the save it BOUNCES the
  GLOBAL MCP daemon instead (`stop_mcp_daemon(true)`); the next session's
  `ensure_mcp_daemon_running` respawns it fresh off the new config, cheaply and
  safely via the build-skew fingerprint handshake.
- **CLI dev install** (`koma ext install --dev`) — registers + saves inline, no live
  daemon to reload (it runs pre-daemon); picked up by the next `koma` session like
  everything else a dev install touches.

### Dev install (offline, unsigned)

`koma ext install --dev <zip|dir>` sideloads a LOCAL extension with none of the
store's discipline — no signature, no koma.run sign-in, no network at all. It's the
CLI-level escape hatch for iterating on an extension you're building: point it at the
`.zip` `src-extension/pack.sh` produces, or at an already-staged directory (a
`manifest.json` + `bin/<exec>` at its root — the same shape a zip unpacks to, e.g.
what `pack.sh` stages before zipping). `koma ext` / `koma ext install` with no
`--dev` just prints usage; there is no other CLI surface for install — the in-app
store is the normal, signed path for anyone who isn't developing the extension
itself.

- **zip vs. dir**: a `.zip` goes through the same unpack pipeline as a signed
  install, minus the integrity/signature check. A directory is validated
  (`manifest.json` must be present and parse) then COPIED — never symlinked, since
  Windows has no reliable unprivileged symlink — into `~/.koma/extensions/<id>/`.
  Either way, `runtime.exec` must already resolve to a real file at the target
  location — raw crate source (no `bin/` staging, no built binary) will fail with a
  clear error rather than installing something that can't spawn.
- **auto-grants**: every capability the manifest's `requires` lists is granted
  automatically (dev installs skip any confirmation step), and each one is printed
  to the terminal as it's granted (`[dev] granting agents:orchestrate ...`) so you
  can see the surface you just gave the extension.
- **tier marker**: the registry entry's `tier` is forced to `"dev"` (instead of
  whatever the manifest declares) so it's visually distinguishable from a real
  store install wherever tier is shown.
- **reinstall**: installing the same id again replaces the existing entry in place
  (`[dev] replacing existing <id> vX.Y.Z`) — install → test → reinstall is a normal
  loop, not something you need to uninstall first.
- **`mcp_servers`**: any declared bundled MCP servers are registered/upserted the same
  as a store install (`[dev] registered N mcp server(s)`), so iterating on a manifest
  that declares one doesn't need a hand-added `McpServerEntry` either.
- **enabled by default**: a dev install is always `enabled: true` — it's for
  testing right now, not sitting dormant.
- Runs before the daemon starts, so a freshly-installed extension is live for the
  *next* `koma` session immediately; anything already running needs a restart to
  pick it up.

### Uninstall — the complete nuke

Uninstall is a COMPLETE nuke — it leaves nothing behind, on disk or in memory, on this
daemon or any other. Both paths run it: the attached daemon
(`requests_ext.rs::uninstall_extension`) and the detached GUI host
(`store_host.rs::spawn_uninstall`), differing only in the session-scoped steps a detached
host has no live managers for (called out below). In order:

1. **Snapshot the manifest** — its `contributes.sub_agents` names + `workspace_dir` are read
   ONCE up front, because step 4 deletes the manifest they come from.
2. **Stop the local child** (attached only) and **purge its contributed MCP tools** from the
   live manager; clear its in-memory footprint — the published `context.set` blob, any
   still-buffered `chat.prompt` entries across every session, and its `ExtAgentRegistry`.
   Detached has no live managers, so this is skipped and self-heals on the next daemon boot.
3. **Fan-out unload** — a fire-and-forget `UnloadExtension` is sent to EVERY OTHER live
   session-daemon's socket so each drops the same in-memory footprint immediately instead of
   at its next boot (best-effort; a daemon too old to know the verb error-replies or drops
   the connection, and is ignored).
4. **`~/.koma/extensions/<id>/` is removed** from disk (guarded against a path-escaping id).
5. **`purge_extension(id)`** removes every key-backed provider this extension registered and
   every OAuth connection it backed, every model anchored to those (the same sweep
   `providers.unregister`'s "host delete guard" uses), and its `ext_preferred_models` record.
   **`remove_ext_mcp_servers(id)`** additionally deregisters any configured MCP-server row
   that belongs to the extension — one tagged with its id, OR whose command path lives under
   `extensions/<id>/` (a bundled MCP-server binary, now a dead orphan). Never touches
   per-session `session_models` overrides (they self-heal to koma-free at next dispatch); a
   purged GLOBAL Main role is reported as a foreground toast (also self-healing, not
   force-reassigned).
6. **The registry entry is removed** and everything above is persisted in ONE `config.save()`,
   followed by a live MCP reload so a removed orphan server's connection drops immediately:
   the attached path reconnects its manager; the detached path bounces the global MCP daemon
   so the next `ensure` respawns it off the new config.
7. **Agent-override sweep** — for each snapshotted sub-agent name, `~/.koma/agents/<name>.md`
   AND every `~/.koma/sessions/*/*/agents/<name>.md` override (a copy a user saved after
   editing the extension's sub-agent) is deleted.

   > **Same-name caveat**: the delete key is the sub-agent NAME (the registry is name-keyed
   > and carries no ownership tag), so an UNRELATED user agent that happens to share a name
   > with an uninstalled extension's sub-agent is swept too.
8. **Workspace-dir nuke** — if the manifest declared a `workspace_dir`, that directory is
   deleted (`remove_dir_all`) after being re-validated through the SAME policy install uses
   (tilde-expand + strictly-under-`$HOME`, never `~/.koma` / `~/.ssh` / `~/.aws` / `~/.gnupg`
   / `~/.config`'s root — enforced on the canonical path). A missing or policy-rejected dir is
   skipped; a nonexistent dir is never created just to delete it. This is user data: the
   GUI's uninstall confirm names this directory before the request is ever sent ("… and its
   data directory (`<path>`). This cannot be undone.").
9. **Reindex + rebuild the system prompt** (attached only, when a foreground session exists) so
   the "# Extension workspaces" note drops the uninstalled extension immediately.

The GUI never fires an uninstall without a two-step confirm (files, agents, MCP servers, and
the data directory are all named) — it is genuinely irreversible.

### Daemon auto-start — four triggers

1. **Boot** — koma's own startup best-effort starts every enabled, `daemon`-kind
   extension.
2. **Install** — a freshly installed `daemon`-kind extension is started
   immediately.
3. **Panel-open** — the first `panel.msg` a panel iframe actually sends
   auto-starts its backing daemon if it's enabled and not already running (merely
   opening the tab does not).
4. **OAuth-begin** — starting a delegated OAuth flow (`oauth.begin`) auto-starts
   the backing daemon if it isn't already running.

There is no automatic restart after an unexpected exit in any case — a later
explicit trigger (a fresh panel message, a fresh OAuth attempt, the next boot)
starts a new instance.

### Turn budget UX

`chat.prompt`'s turn budget (10 injected turns without real user activity, see the
`chat:prompt` grant reference) is enforced purely as a call-time error today —
`"extension turn budget exhausted; waiting for user activity"`. There is currently
no proactive warning banner or UI affordance before the budget is hit; an extension
driving `chat.prompt` in a loop should treat that error string as the signal to
back off, not rely on koma to warn it first.

---

## Compatibility (v0)

The protocol is additive by design so old and new extensions keep working across
waves without a re-touch:

- Every new field on `ExtensionManifest`/`Contributes`/`SubAgentDef`/
  `OAuthProviderDef` is `#[serde(default)]` (or `Option<T>` with
  `skip_serializing_if = "Option::is_none"`) — an old manifest missing a field a
  later wave added still parses, and a new manifest omitting a field it doesn't
  need serializes without it. This is directly tested in
  `src-extension/src/protocol.rs` (`old_style_contributes_without_events_or_subagent_extras_parses`,
  `oauth_provider_def_roundtrips`).
- New `Grant` variants are additive; an old koma build simply never grants a wire
  string it doesn't recognize (`parse_grants` drops unknown strings, fail-closed)
  rather than erroring.
- The `ExtMsg`/`KomaMsg` envelopes are `#[serde(tag = "t", rename_all =
  "lowercase")]` — a frame with an unrecognized `t` fails to PARSE (an `Err`, not a
  panic; tested by `unknown_tag_fails_cleanly`), so a genuinely new frame type from
  a future protocol version is something both sides can choose to handle
  gracefully (log-and-skip) at the transport boundary rather than crashing.
- `params`/`result`/`payload` are always arbitrary JSON `Value` — adding a new
  field to an existing call's shape never breaks a receiver that only reads the
  fields it knows about.

Nothing here is a stable v1 contract yet — see the top-of-file note — but within
v0, additive changes should never require you to touch a working extension.
