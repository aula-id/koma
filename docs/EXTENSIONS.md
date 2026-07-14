# Extensions

> **This is a v0 direction document.** The extension protocol is still being
> shaped and will change until it is frozen at v1. Nothing here is a stable
> contract yet — it describes where koma extensions are heading and why, so the
> shape can be discussed in the open before it sets.

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
requests to, a tool the agent can call, a panel it can render. The extension sits
there and answers.

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
panel to koma while it *drives* koma's sub-agents. But keeping the two directions
straight matters, because they place very different demands on the protocol.

The practical consequence is that the connection between koma and an extension has
to work **both ways over the same link**. koma has to be able to call into the
extension, and the extension has to be able to call back into koma. If we only
built the first direction — koma politely asking extensions for things — every
"extension that drives koma" would be impossible, and we'd only find that out after
the protocol was already frozen. So the two-way shape is baked in from the start,
even though the earliest examples only use the simple direction.

## What an extension gives koma: contributions

The things an extension adds to koma are called its *contributions*. There are four
kinds today.

**Sub-agents.** An extension can ship its own agent descriptions, and once it is
installed those show up as new agent types you can delegate work to, right
alongside the built-in ones.

**Models.** An extension can register a provider and a catalogue of models. From
then on those models are available to route to, exactly like any other provider,
and koma resolves to the extension at dispatch time.

**Panels.** An extension can bring its own user interface. It becomes a tab in the
main area with an icon in the sidebar, and koma frames it so the extension's UI runs
in its own process without leaking into koma's.

**Tools.** An extension can give the agent new tools to call — and this is worth a
short explanation, because the obvious idea is the wrong one. You might expect an
extension to register a "built-in" tool. But koma's built-in tools are compiled
directly into the agent; a separate process simply cannot inject one without a
forwarding layer in between, and that forwarding layer is exactly what MCP already
is. So extension tools *are* MCP. The difference from an MCP server you'd add by
hand is ownership: an extension's tools belong to the extension. They appear in the
tool list marked as coming from that extension, and you don't remove them one by
one — you remove them by uninstalling the extension. This reuses everything koma
already has for MCP and works no matter what language the extension is written in.

Whatever an extension contributes gets cleaned up when you uninstall it. Its models,
its sub-agents, its tools, its panel — all registered on install, all purged on
removal, so uninstalling actually leaves no trace.

## What an extension asks of koma: requirements

If contributions are what an extension gives, *requirements* are what it asks to
use. When an extension needs to drive one of koma's own systems, it declares that
up front as a requirement, and koma enforces it at the boundary. This doubles as the
permission list you see before installing: an extension that wants to run your
sub-agents has to say so, and you get to see it say so.

The first and most important requirement is access to the sub-agent system, and it
comes in two levels. The lighter one lets an extension *watch* — read the status and
output of sub-agents, enough to render a live view of what's happening. The stronger
one lets an extension *orchestrate* — actually spawn, queue, steer, and stop
sub-agents itself.

That stronger level is a genuine control surface, not a read-only window, and the
agentic kanban is what it is for:

```text
agentic kanban
  you give it a product spec
    → it spawns one sub-agent per card
    → each agent works on its card on its own
    → their status flows back and the board updates live
```

Even though every extension is first-party and trusted, koma still keeps these
requirements scoped tightly. A model gateway that only needs to borrow your account
session has no business holding orchestration rights, and koma shouldn't have to
assume it won't use them. Asking for exactly what you need, and no more, stays the
rule — not as a defense against the author, but as clean engineering.

## Installing and running

Browsing the store is open to anyone; installing is what needs an account. When you
install an extension, koma downloads it, checks its signature, and launches it as a
supervised sibling process. The two sides introduce themselves in a short handshake
— the extension presents what it is and what it contributes and requires, and koma
agrees on a protocol version — and from then on the extension is live. Its
contributions light up across koma and its requirements are enforced as it runs.
When you quit koma it is shut down with you, and when you uninstall it everything it
added is removed.

An extension describes itself in a manifest, which is where it declares its
identity, whether it is a free or paid extension, how koma should launch it, and both
its contributions and its requirements. That manifest is the whole agreement between
koma and the extension in one place.

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

## Two extensions to picture

Two examples make the two directions concrete.

**koma-gateway** is the "koma uses the extension" kind. It presents an
OpenAI-compatible endpoint, koma sends requests to it like any other provider, and it
routes them onward. It doesn't care what model you named — it takes the request and
handles the rest. It is a pure provider: it contributes models and nothing more.

**komatica** is the mirror image, the "extension uses koma" kind. It is an agentic
kanban that consumes a product spec and drives a fleet of sub-agents through koma to
build against it, each card its own working agent. It contributes its board as a
panel, but its real job is to orchestrate — it is in the driver's seat and koma is
the engine.

## The SDK

The tools for building an extension live in `src-extension/` in the open repo. There
is a `koma-extension` crate that carries the protocol — the manifest, the handshake,
the contribution and requirement types — along with a thin layer of helpers so that
opening the socket, completing the handshake, and registering what you contribute is
a few lines rather than a project. Next to it is a small `hello` example you can
actually run: it contributes a panel and a tool, the two things that need nothing
else behind them, so it stands on its own and shows the shape.

Rust is the source of truth and the first SDK; a generated TypeScript version comes
later. Models and sub-agent orchestration are already part of the v0 protocol even
though the hello example doesn't use them — the shapes are settled before the
services behind them exist, so that when koma-gateway and komatica arrive they slot
in without breaking anything.
