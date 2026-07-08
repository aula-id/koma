# Architecture: GUI Daemon

Detailed architectural documentation of koma's desktop GUI (webview-based) and its communication with the session daemon.

## System Overview

The GUI is a native desktop window powered by **wry** (webview) + **tao** (event loop) hosting a React/TypeScript frontend. It communicates with the same session daemon as the TUI client — the daemon is the single source of truth for all session state.

```
┌─────────────────────────────────────────────────────────────┐
│  tao main thread (event loop, runs until window close)       │
│  ┌───────────────────────────────────────────────────────┐  │
│  │  wry WebView (koma:// protocol, serves React app)     │  │
│  │  React 19 + Zustand + Monaco + TailwindCSS v4         │  │
│  └──────────────┬────────────────────────────────────────┘  │
│                 │ ipc.postMessage / evaluate_script          │
│  ┌──────────────▼────────────────────────────────────────┐  │
│  │  IPC handler (ClientMsg → HostCtl / ClientRequest)    │  │
│  └──────────────┬────────────────────────────────────────┘  │
│                 │ mpsc channels                              │
│  ┌──────────────▼────────────────────────────────────────┐  │
│  │  Host-relay client-thread (own tokio runtime)          │  │
│  │  - daemon connection (Unix socket)                     │  │
│  │  - snapshot/delta fold loop                            │  │
│  │  - push_loop (JSON → UserEvent::Push → evaluate_script)│  │
│  └──────────────┬────────────────────────────────────────┘  │
└─────────────────┼───────────────────────────────────────────┘
                  │ Unix domain socket (~/.koma/run/<id>.sock)
                  ▼
        ┌──────────────────┐
        │  Session Daemon   │  (headless, owns agent runtime + session lock)
        └──────────────────┘
```

## Technology Stack

| Layer | Technology | Version |
|---|---|---|
| Windowing | tao | 0.35 |
| Webview | wry | 0.52 |
| Frontend framework | React | 19 |
| State management | Zustand | — |
| Code editor | Monaco Editor | — |
| Styling | TailwindCSS | v4 |
| Build tooling | Vite | — |
| File dialogs | rfd | 0.15 |
| macOS menus | muda | 0.19 |
| macOS transparency | objc2-app-kit | — |

## Feature Gate

The GUI is compiled only with `--features gui`. Without it, the `koma gui` subcommand is unavailable. The `Cargo.toml` gates all webview/tao/rfd/muda dependencies behind this feature.

## Window Construction

The `run_gui()` function (in `src-agent/src/app/runtime/gui/mod.rs`) performs:

1. Creates a **frameless, transparent** tao window (1024×680 logical, `with_decorations(false)`, `with_transparent(true)`).
2. Installs a native macOS Edit menu bar (via `muda`) to make Cmd+V/C/X/A work through WKWebView's responder chain.
3. Spawns the **host-relay client-thread** (background tokio runtime).
4. Builds the wry WebView pointed at `koma://localhost/index.html` — the embedded React app served through a custom protocol handler.
5. On Linux, attaches via `build_gtk(window.default_vbox())` to avoid X11 foreign-window reparenting issues.
6. On macOS, clears `underPageBackgroundColor` on the WKWebView to fix opaque backgrounds behind transparent corners.

## Custom Protocol: koma://

The WebView loads `koma://localhost/index.html`. The `handle_koma_request()` function serves files from the compiled-in `src-webgui/dist/` directory (embedded via `include_dir!`). This means the entire React app is a static asset tree baked into the binary — no external file serving needed.

MIME types are inferred from file extensions. Unknown extensions fall back to `application/octet-stream`.

## Host-Relay Bridge

The critical architectural decision: the GUI host IS the daemon client, but the main thread is owned by tao's event loop (which diverges — `run` never returns). So the daemon connection runs on a separate thread.

### Host-Relay Thread

Spawns via `run_host_relay()` with its own tokio runtime. Responsibilities:

- Connects to the session daemon over a Unix socket
- Runs the **fold loop**: receives `DaemonFrame` (snapshots + deltas) and re-derives the authoritative React state envelope
- Runs the **push_loop**: serializes each state envelope as a complete JSON object and sends it to the main thread via `EventLoopProxy::send_event(UserEvent::Push(json))`
- Handles `HostCtl` messages from the main thread for session lifecycle operations

### Communication Channels

```
JS ←─────────────── wry evaluate_script ──── main thread
  │                                              ↑
  │ ipc.postMessage(JSON)                   UserEvent::Push(json)
  ▼                                              │
IPC handler ──→ mpsc (HostCtl) ──────────→ host-relay thread
             ──→ mpsc (ClientRequest) ──→ host-relay thread → daemon
```

Two channel types flow from main thread to host-relay:

| Channel | Type | Purpose |
|---|---|---|
| `ctl_tx` | `mpsc::Sender<HostCtl>` | Session lifecycle: Ready, SelectSession, NewSession, RefreshHub, ToSwapper, ConfigMutate, FileDiff, ListModels, ListRoutes |
| `live_req` | `Mutex<Option<mpsc::Sender<ClientRequest>>>` | Live daemon requests: SubmitInput, ApproveTool, Interrupt, Compact, etc. Only present when a session is attached. |

## JS ↔ Rust Protocol

### Inbound (JS → Rust)

The React app calls `window.ipc.postMessage(JSON.stringify(msg))`. Messages are deserialized into `ClientMsg` (internally tagged on `t`):

```typescript
// Window commands (custom titlebar)
{ t: "win", a: "drag" | "min" | "max" | "close" }
{ t: "winresize", dir: "e" | "w" | "n" | "s" | "ne" | "nw" | "se" | "sw" }

// Host-relay bridge (native React client)
{ t: "req", r: "Ready" }
{ t: "req", r: "Submit", text: "..." }
{ t: "req", r: "SelectSession", id: "..." }
{ t: "req", r: "NewSession" }
{ t: "req", r: "RefreshHub" }
{ t: "req", r: "CancelSwitch" }
{ t: "req", r: "AttachFile", name: "...", mime: "...", bytesB64: "..." }
{ t: "req", r: "AttachPath", path: "..." }
{ t: "req", r: "RemoveAttachment", markerN: 0 }
{ t: "req", r: "FileSearch", query: "...", limit: 50 }
{ t: "req", r: "Rename", name: "..." }
{ t: "req", r: "Interrupt" }
{ t: "req", r: "Compact" }
{ t: "req", r: "RewindTo", index: 3 }
{ t: "req", r: "KillSubagent", id: 1 }
{ t: "req", r: "KillBash", id: 2 }
{ t: "req", r: "SetSessionMain", modelUuid: "..." }
{ t: "req", r: "SetMode", mode: "auto" | "normal" | "plan" | "yolo" }
{ t: "req", r: "ApproveTool", approve: true }
{ t: "req", r: "PlanDecision", decision: "approve" | "compact" | "deny" }
{ t: "req", r: "FileDiff", path: "..." }
{ t: "req", r: "SetMcpServer", ... }
{ t: "req", r: "DeleteMcpServer", uuid: "..." }
{ t: "req", r: "EnableMcpServer", uuid: "...", enabled: true }
{ t: "req", r: "SetProvider", ... }
{ t: "req", r: "DeleteProvider", uuid: "..." }
{ t: "req", r: "SetModel", ... }
{ t: "req", r: "DeleteModel", uuid: "...", scope: "global" }
{ t: "req", r: "ListModels", provider: "..." }
{ t: "req", r: "ListRoutes", provider: "...", modelId: "..." }
{ t: "req", r: "SetTheme", name: "..." }
{ t: "req", r: "SetupKomaFree" }
```

### Outbound (Rust → JS)

The host-relay thread calls `window.__komaClient.push(jsonString)` via `evaluate_script`. Each push is a **complete JSON object** (not a delta), tagged on `k`:

```typescript
// Full state snapshot (after attach, reconnect, or config change)
{ k: "Snapshot", ... }

// Incremental state update (token append, status change, etc.)
{ k: "StreamMsg", ... }

// Reasoning text stream
{ k: "Reasoning", ... }

// Status bar update
{ k: "Status", ... }

// Session hub (list of live + on-disk sessions)
{ k: "Hub", ... }

// File search results (omnisearch)
{ k: "SearchResults", ... }

// Model catalogue for Connector picker
{ k: "ModelList", ... }

// Provider route list for Connector picker
{ k: "RouteList", ... }
```

## Dual Routing: Attached vs Pre-Session

A key architectural challenge: config mutations and catalogue fetches must work both when a session daemon is attached AND during onboarding (no session exists yet).

### Config Mutations (SetProvider, SetModel, SetTheme, SetupKomaFree, etc.)

```
Attached?  ──yes──→  Forward as ClientRequest to daemon (daemon owns AppConfig)
    │
    no
    │
    └──→  Forward as HostCtl::ConfigMutate to host-relay thread
          (applies directly to ~/.koma/config.json, re-pushes Config)
```

### Catalogue Fetches (ListModels, ListRoutes)

```
Attached?  ──yes──→  Forward as ClientRequest to daemon (daemon runs GET, replies out-of-band)
    │
    no
    │
    └──→  Forward as HostCtl::ListModels/ListRoutes to host-relay thread
          (host-relay runs the GET itself, pushes result envelope)
```

### Host-Only Operations (FileDiff)

`FileDiff` always routes to the host-relay thread regardless of attach state — the host has direct filesystem + git access and never needs the daemon for this.

## File Attachments

Three attachment paths converge into the daemon's paste/attachment ingest:

1. **Raw bytes** (clipboard paste, drag-drop, file picker): JS sends `AttachFile { name, mime, bytesB64 }` → host base64-decodes → writes to `<tmp>/koma/gui-attach/<uuid>-<name>` → forwards path as `ClientRequest::Paste`.

2. **On-disk path** (omnisearch pick): JS sends `AttachPath { path }` → host forwards path as `ClientRequest::Paste`.

3. **Drop staged chip**: JS sends `RemoveAttachment { markerN }` → forwarded as `ClientRequest::RemoveAttachment`.

### Attachment Marker Reconciliation

When JS sends a `Submit`, the host appends any staged `[Image #N]` markers that React's text doesn't already carry. This ensures the daemon's submit-time reconcile (which keeps only attachments whose marker survived in the sent text) doesn't drop staged images.

## macOS Platform Details

### Transparency Fix

wry's `with_transparent(true)` clears the legacy `drawsBackground` flag but not `underPageBackgroundColor`. On macOS 12+, WKWebView paints this opaque color behind the page, creating an opaque square behind rounded corners. The fix: access the raw `WKWebView` handle via `WebViewExtMacOS` and call `setUnderPageBackgroundColor(Some(&NSColor::clearColor()))`.

### Native Menu Bar

WKWebView dispatches editing commands (paste/copy/cut/select-all) through NSMenu Edit-menu items via the responder chain. A frameless window with no menu bar breaks Cmd+V/C/X/A. The fix: install a minimal `muda` menu bar with App (Quit) and Edit (Undo, Redo, Cut, Copy, Paste, Select All) submenus, then `init_for_nsapp()`.

### Linux Webview Attachment

wry's default `.build(&window)` on Linux uses X11 foreign-window reparenting, which can render to an uncomposited surface (blank/gray window on some GPUs). The fix: use `.build_gtk(window.default_vbox())` to attach directly to tao's GTK widget hierarchy.

## Daemon Lifecycle from GUI Perspective

1. **Window opens**: IPC handler receives `GuiReq::Ready` → sends `HostCtl::Ready` to host-relay.
2. **Host-relay boots**: Scans for live session daemons (`~/.koma/run/*.sock`), probes with `Status` request. If found, attaches to one and pushes `Hub` envelope. If none, pushes empty `Hub` with new-session prompt.
3. **Session selected**: `GuiReq::SelectSession` → `HostCtl::Select(id)` → host-relay attaches to that daemon, fold loop starts receiving `DaemonFrame` → pushes `Snapshot`.
4. **Chat**: `GuiReq::Submit` → `ClientRequest::SubmitInput` → daemon processes → fold loop receives stream events → pushes `StreamMsg` envelopes.
5. **Window close**: tao event loop exits → host-relay thread drops → daemon connection closes. The daemon itself keeps running (resumable via the swapper or a new terminal client).

## System Requirements

### Build Dependencies

- Rust 2021 edition toolchain
- `npm` / Node.js (for building `src-webgui/` via Vite)
- Platform-specific webview libraries:
  - **macOS**: WKWebView (ships with macOS)
  - **Linux**: webkit2gtk (e.g., `libwebkit2gtk-4.1-dev` on Ubuntu)
  - **Windows**: WebView2 (ships with Windows 10+)

### Runtime Dependencies

- `~/.koma/` directory (auto-created) for session store, config, sockets
- Python (optional): for internet full-mode and security daemon features
- MCP servers (optional): any MCP-compatible server over stdio or HTTP

### Socket Locations

| Socket | Owner | Purpose |
|---|---|---|
| `~/.koma/run/<session_id>.sock` | Session daemon | Client↔daemon IPC |
| `~/.koma/mcp.sock` | Global MCP daemon | Session daemon↔MCP proxy |

### Key Constants

| Constant | Value | Description |
|---|---|---|
| `MAX_FRAME_BYTES` | 64 MiB | Max single IPC frame payload |
| Default model | `openai/gpt-4o-mini` | Via OpenRouter |
| Max tool output | 400,000 chars | Truncation limit for tool results |
| Max sub-agent report | 50,000 chars | Sub-agent completion cap |
| Approval park timeout | 30 minutes | How long a tool approval waits |
| Self-exit grace | ~1 second | Daemon waits before exiting after last client |

## Comparison with TUI Client

| Aspect | GUI | TUI |
|---|---|---|
| Rendering | React in wry webview | ratatui + crossterm |
| Event loop | tao (OS event loop) | tokio + adaptive polling (8ms streaming, 100ms idle) |
| Input | HTML forms + keyboard events | crossterm KeyEvent → Action |
| State model | Zustand store (React) | AppState (Rust struct, redraw on dirty flag) |
| Daemon connection | Host-relay thread (same IPC protocol) | Client loop (same IPC protocol) |
| Config mutations | Dual-routed (daemon or host-relay) | Driven through Mode::Settings editors |
| Session switching | Hub overlay (JS) | SessionHub mode (ratatui) |
| File attachments | Drag-drop, paste, omnisearch picker | Paste from clipboard, @-palette |
