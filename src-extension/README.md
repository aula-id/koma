# koma-extension

> **v0, unstable, will break.** This is the reference implementation of the
> extension protocol described in `docs/EXTENSIONS.md`, published early so the
> shape can be seen and discussed before it freezes at v1.

## What this is

`koma-extension` is the public Rust SDK for building a koma extension: a
small program that runs alongside koma and adds to what it can do. The crate
carries the protocol types — the manifest, the handshake, the contribution
and requirement shapes — plus a thin helper layer (`sdk.rs`) so that a sample
doesn't have to hand-roll any of it.

Next to the crate, `example/` holds seven small extensions. Each one is a
real, separately runnable binary that shows one corner of the protocol.
Every sample ships a scripted **demo mode**: with no koma process on the
other end (the default), it prints the handshake and the interaction it
would have with koma, frame by frame, so the shape is visible on its own.
Set `KOMA_EXT_SOCKET` (and `KOMA_EXT_TOKEN`) and a sample instead connects to
a real koma over that unix socket and runs for real — see
`docs/EXTENSIONS.md` for the full capability reference (every grant, every
verb, every payload shape) and `docs/ARCH_EXTENSION.md` for the byte-level
wire contract.

## Quickstart

New to the protocol? Start with `event-watcher-daemon` — it's the smallest
sample that exercises the koma→ext direction (`on_event`) end to end:

```sh
cargo run -p event-watcher-daemon
```

Then read `orchestrator-daemon`'s source top to bottom — it's
documentation-as-code for the ext→koma direction, walking through every
grant-gated verb with its real request/reply shape and every error mode
commented inline.

## The manifest

Every extension describes itself in a `manifest.json`. Here is
`example/echo-tool-daemon/manifest.json` in full:

```json
{
  "schema": "koma-extension/v0",
  "id": "run.koma.example.echo-tool-daemon",
  "name": "Echo Tool",
  "version": "0.0.0",
  "description": "A minimal daemon extension that contributes a single echo tool.",
  "tier": "free",
  "kind": "daemon",
  "runtime": { "exec": "echo-tool-daemon", "args": [] },
  "contributes": {
    "tools": [
      {
        "name": "echo",
        "description": "echo input back",
        "input_schema": { "type": "object", "properties": { "text": { "type": "string" } } }
      }
    ]
  },
  "requires": []
}
```

`id` is reverse-DNS so extensions don't collide. `tier` marks whether it's
free or paid. `runtime` tells koma how to launch it. `contributes` is what
the extension gives koma — sub-agents, models, panels, tools, events it
wants to listen for, OAuth login providers it backs — and `requires` is what
it asks to use in return: one or more of eight grants covering sub-agent
orchestration, session management, chat injection, direct model calls,
system-prompt context, OAuth delegation, and model/provider registration.
Everything in `manifest.json` deserializes straight into the
`ExtensionManifest` struct in `src/protocol.rs`; that struct is the
authoritative shape, and each sample loads its manifest at compile time with
`include_str!` and parses it at startup, so a bad manifest fails loudly
instead of silently drifting from the code.

`docs/EXTENSIONS.md` has the full manifest field reference (every
`contributes` array's item shape, the sub-agent model-slug binding chain,
every `OAuthProviderDef` field) and the full grants reference (every verb
each grant unlocks, with its exact request/response shape, every error
string, and every numeric limit) — this README stays a map of the samples,
not a restatement of that reference.

## Daemon vs. oneshot

An extension is either a **daemon** or a **oneshot**, and the difference is
about how long it lives. A daemon is a long-running process koma launches
once and keeps talking to for as long as it's installed — the natural shape
for something with a panel to keep alive, or a tool that holds state between
calls. A oneshot is launched, asked exactly one thing, and expected to answer
and exit — the natural shape for a tool call that doesn't need anything to
persist in between.

The two kinds share the same wire protocol and the same handshake; the only
difference is the lifecycle around it.

## The two directions

The other axis, independent of daemon-vs-oneshot, is which side is driving.
Sometimes koma calls into the extension — that's `contributes`, and it shows
up as an `Invoke` the extension answers with `on_invoke`. Sometimes the
extension calls into koma — that's `requires`, and it shows up as a `Call`
the extension makes through the `Koma` handle the SDK hands it. A single
extension can do both, but most samples here only exercise one direction at
a time to keep the demonstration readable.

## The seven samples

| sample | kind | tier | what it teaches |
| --- | --- | --- | --- |
| `echo-tool-daemon` | daemon | free | the simplest `on_invoke`: one contributed tool, no `requires` |
| `upper-tool-oneshot` | oneshot | free | the simplest oneshot: one tool, answered once, then exit |
| `agent-peek-oneshot` | oneshot | free | contributes nothing; only requires `agents:read` and calls `agents.list` |
| `event-watcher-daemon` | daemon | free | THE starter sample for `contributes.events` / `on_event` — subscribes to every broadcast event and counts them |
| `orchestrator-daemon` | daemon | free | documentation-as-code for the five grant-verbs that drive koma (`sessions.list`, `models.invoke`, `context.set`, `chat.prompt`, `agents.spawn`), every call commented with its real shape and every error mode |
| `fleet-board-daemon` | daemon | paid | the live bridge demo: a real panel (`ui/index.html` + the copyable `ui/koma-panel.js` helper) driving a real sub-agent spawn through the cmd-channel pattern the DEADLOCK RULE forces on you |
| `oauth-demo-daemon` | daemon | free | delegated OAuth (`oauth.begin`/`.poll`/`.cancel`), with teaching comments walking the full arc from a login to a resolvable model-provider gateway (`models.register` + `providers.register`) |

### Capability matrix

| sample | contributes | requires |
| --- | --- | --- |
| `echo-tool-daemon` | tools | — |
| `upper-tool-oneshot` | tools | — |
| `agent-peek-oneshot` | — | `agents:read` |
| `event-watcher-daemon` | tools, events | — |
| `orchestrator-daemon` | — | `agents:orchestrate`, `sessions:manage`, `chat:prompt`, `models:invoke`, `context:publish` |
| `fleet-board-daemon` | panels, sub_agents, events | `agents:orchestrate` |
| `oauth-demo-daemon` | oauth_providers | `oauth:contribute` |

Run any of them from `src-extension/`:

```sh
cargo run -p echo-tool-daemon
cargo run -p upper-tool-oneshot
cargo run -p agent-peek-oneshot
cargo run -p event-watcher-daemon
cargo run -p orchestrator-daemon
cargo run -p fleet-board-daemon
cargo run -p oauth-demo-daemon
```

Each prints its manifest id, the `Hello` / `Welcome` handshake, and then the
scripted interaction for its direction, with every frame pretty-printed and
labeled so it reads like a transcript. Nothing here opens a real socket by
default — set `KOMA_EXT_SOCKET` (and `KOMA_EXT_TOKEN`) and a sample instead
connects to a real koma over that unix socket and runs the same code for
real. `orchestrator-daemon`'s demo transcript is intentionally honest about
this split: the SDK's canned demo-mode `Koma` stub only fakes replies for
`agents.*` methods, so its other four calls print a real
`{"error":"unknown method: ..."}` canned reply in demo mode — that's
expected, not a bug; run it against a real koma to see the documented real
replies.

## Packaging for distribution

Once your extension is built, use `./pack.sh` to create distributable `.zip`
packages. The script builds all seven examples in release mode and packages
each one into a zip archive containing the manifest, executable, and (if
present) the extension's own `ui/` directory:

```sh
./pack.sh
```

Each produced zip (e.g., `dist/echo-tool-daemon.zip`) has this structure:

```
manifest.json
bin/echo-tool-daemon
ui/...          (only if the extension ships a panel — e.g. fleet-board-daemon)
```

The manifest's `runtime.exec` field is automatically set to `"bin/<name>"` to
reflect the binary location within the package. Zips are written to `dist/`.

## Building your own

Depend on the crate by path (there's no published version yet), implement
`Extension` for a small struct, and hand it to `run_daemon` or `run_oneshot`
along with a `manifest.json`. Start from `event-watcher-daemon` or
`echo-tool-daemon` (each under 100 lines) if you're contributing something
simple; start from `fleet-board-daemon` if you need a panel driving koma,
since it's the sample that works out the threading pattern the DEADLOCK RULE
forces on you (see `docs/EXTENSIONS.md`'s "The deadlock rule and the
threading model" section before you copy it).
