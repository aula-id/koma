# koma-extension

> **v0, unstable, will break.** This is the reference implementation of the
> extension protocol described in `docs/EXTENSIONS.md`, published early so the
> shape can be seen and discussed before it freezes at v1. There is no koma
> host to connect to yet — everything here runs standalone.

## What this is

`koma-extension` is the public Rust SDK for building a koma extension: a
small program that runs alongside koma and adds to what it can do. The crate
carries the protocol types — the manifest, the handshake, the contribution
and requirement shapes — plus a thin helper layer so that a sample doesn't
have to hand-roll any of it.

Next to the crate, `example/` holds four small extensions. Each one is a
real, separately runnable binary that shows one corner of the protocol. None
of them talk to a real koma process, because there isn't one yet — instead,
each runs a scripted **demo mode** that prints the handshake and the
interaction it would have with koma, frame by frame, so the shape is visible
on its own.

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
the extension gives koma — sub-agents, models, panels, tools — and
`requires` is what it asks to use in return, such as reading or driving
koma's sub-agent system. Everything in `manifest.json` deserializes straight
into the `ExtensionManifest` struct in `src/protocol.rs`; that struct is the
authoritative shape, and each sample loads its manifest at compile time with
`include_str!` and parses it at startup, so a bad manifest fails loudly
instead of silently drifting from the code.

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

## The four samples

| sample | kind | tier | direction it shows |
| --- | --- | --- | --- |
| `echo-tool-daemon` | daemon | free | koma invokes a contributed tool |
| `upper-tool-oneshot` | oneshot | free | koma invokes a contributed tool, once |
| `fleet-board-daemon` | daemon | paid | the extension drives koma's sub-agents |
| `agent-peek-oneshot` | oneshot | free | the extension only reads koma state |

Run any of them from `src-extension/`:

```sh
cargo run -p echo-tool-daemon
cargo run -p upper-tool-oneshot
cargo run -p fleet-board-daemon
cargo run -p agent-peek-oneshot
```

Each prints its manifest id, the `Hello` / `Welcome` handshake, and then the
scripted interaction for its direction, with every frame pretty-printed and
labeled so it reads like a transcript. Nothing here opens a real socket —
set `KOMA_EXT_SOCKET` and a sample will instead print that host mode isn't
implemented yet and exit, since there is genuinely nothing on the other end
to connect to in v0.

## Packaging for distribution

Once your extension is built, use `./pack.sh` to create distributable `.zip`
packages. The script builds all four examples in release mode and packages each
one into a zip archive containing the manifest and executable:

```sh
./pack.sh
```

Each produced zip (e.g., `dist/echo-tool-daemon.zip`) has this structure:

```
manifest.json
bin/echo-tool-daemon
```

The manifest's `runtime.exec` field is automatically set to `"bin/<name>"` to
reflect the binary location within the package. Zips are written to `dist/`.

## Building your own

Depend on the crate by path (there's no published version yet), implement
`Extension` for a small struct, and hand it to `run_daemon` or `run_oneshot`
along with a `manifest.json`. The four samples in `example/` are the best
starting point — each is on the order of 30 lines.
