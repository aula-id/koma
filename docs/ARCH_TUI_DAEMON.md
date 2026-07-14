# Architecture: TUI Daemon

Detailed architectural documentation of koma's terminal user interface and its communication with the session daemon.

## System Overview

The TUI is a ratatui-based terminal application that runs as a thin client connecting to a headless session daemon over a Unix domain socket. The daemon owns the agent runtime, session locks, and all async work; the client handles rendering and input.

```
┌──────────────────────────────────────────────────────────────┐
│  TUI Client (--attach)                                       │
│  ┌──────────────────────┐  ┌──────────────────────────────┐  │
│  │  crossterm terminal   │  │  Client loop (tokio)          │  │
│  │  ratatui rendering    │  │  - send_frame / recv_frame    │  │
│  │  Key input polling    │  │  - shadow state (AppState)    │  │
│  │  Adaptive draw rate   │  │  - event dispatch             │  │
│  └──────────┬───────────┘  └──────────┬───────────────────┘  │
│             │ mpsc channels            │                      │
│             └──────────┬───────────────┘                      │
└────────────────────────┼─────────────────────────────────────┘
                         │ Unix domain socket
                         │ ~/.koma/run/<session_id>.sock
                         │ Length-prefixed JSON (4-byte BE + payload)
                         ▼
┌──────────────────────────────────────────────────────────────┐
│  Session Daemon (--daemon, or auto-spawned)                   │
│  ┌──────────────────────────────────────────────────────────┐│
│  │  daemon_loop (sync, main thread)                         ││
│  │  - service_all_sessions (advance turns, drain channels)   ││
│  │  - service_global (catalogue, compaction, toasts)         ││
│  │  - DaemonHub (drain client requests, stream deltas)       ││
│  └──────────────────────────────────────────────────────────┘│
│  ┌──────────────────────────────────────────────────────────┐│
│  │  accept_loop (tokio, background)                         ││
│  │  - binds ~/.koma/run/<session_id>.sock                   ││
│  │  - accepts client connections                            ││
│  │  - spawns per-client conn tasks                          ││
│  └──────────────────────────────────────────────────────────┘│
└──────────────────────────────────────────────────────────────┘
```

## Daemon-Per-Session Model

Every `koma` invocation follows the same protocol:

1. **Mint a UUID** for this session.
2. **Ensure a daemon** is running for that UUID: try to `connect` to `~/.koma/run/<uuid>.sock`. Success = daemon alive (attach as client). Connection refused = no daemon (spawn one and become it).
3. **Attach as a thin client** to the daemon's socket.

Each daemon owns exactly ONE session. Multiple sessions = multiple daemons, each bound to their own socket. A "session" in koma is a conversation with its own history, settings, and working directory — isolated from all others.

### Liveness Oracle

Daemon liveness is determined by **who holds the bound socket**, NOT by PID files. PIDs get reused, making PID-based checks unreliable. A successful `connect()` means a daemon is alive; `ConnectionRefused` means it is not, and this process may bind and become the daemon.

Stale sockets from crashed daemons are removed before binding (the crash left a socket file with no listener). This is safe because bind — not file existence — is the liveness oracle.

## IPC Protocol

### Wire Format

Every message on the socket uses a fixed framing:

```
┌──────────────────────────┬───────────────────────────────────┐
│ 4 bytes, big-endian u32   │ UTF-8 JSON payload (N bytes)       │
│ length prefix = N          │                                   │
└──────────────────────────┴───────────────────────────────────┘
```

- **Max frame**: 64 MiB (`MAX_FRAME_BYTES`)
- `FrameReader` handles partial reads, coalesced frames, and cap enforcement
- Both directions (client→daemon and daemon→client) use the same framing

### Message Types

#### Client → Daemon (`ClientRequest`)

| Request | Purpose |
|---|---|
| `Attach { foreground_id, cwd }` | Register as a client, receive full state snapshot |
| `Detach` | Disconnect (daemon keeps running) |
| `Status` | Lightweight probe: one-shot metadata about the session (used by hub/swapper for discovery) |
| `SubmitInput { text }` | Send a chat message |
| `Shell { cmd }` | Execute a shell command |
| `SendKey(KeyWire)` | Forward a keystroke to the daemon |
| `Paste { text }` | Paste text or a file path |
| `ApproveTool { approve }` | Answer a paused tool approval |
| `PlanDecision { decision }` | Answer a paused plan approval (`approve`/`compact`/`deny`) |
| `NewSession { name, cwd }` | Create a new session |
| `QuitSession { session_id }` | Close a session (tombstone) |
| `QuitDaemon` | Shut down the entire daemon |
| `ListSessions` | List all sessions (on-disk + live) |
| `Resync` | Request a fresh full snapshot (gap recovery) |
| `SetMode { mode }` | Change agent mode (`auto`/`normal`/`plan`/`yolo`) |
| `SetSessionMain { model_uuid }` | Set session-local Main model override |
| `Interrupt` | Stop the current turn |
| `RewindTo { index }` | Rewind conversation to a message |
| `Compact` | Summarize and trim history |
| `FileSearch { query, limit }` | Fuzzy-search workspace file index |
| `KillSubagent { id }` | Kill a running sub-agent |
| `BashKill { id }` | Kill a background bash job |
| `SetMcpServer`, `DeleteMcpServer`, `EnableMcpServer` | MCP server config management |
| `SetProvider`, `DeleteProvider` | Provider config management |
| `SetModel`, `DeleteModel` | Model config management |
| `SetTheme { name }` | Change the active theme |
| `SetupKomaFree` | Enable the keyless Koma Free tier |

#### Daemon → Client (`DaemonFrame`)

Each frame carries a **monotonic `seq`** number for gap detection.

| Frame | Purpose |
|---|---|
| `DaemonEvent::Hello { build_skew }` | Build-version handshake on connect |
| `DaemonEvent::Snapshot { ... }` | Full state (on attach or resync) |
| `StateDelta::TokenAppended` | Incremental text token |
| `StateDelta::ReasoningAppended` | Incremental reasoning text |
| `StateDelta::StatusChanged` | Status bar update |
| `StateDelta::InputChanged` | Input field changed |
| `StateDelta::ScrollChanged` | Scroll position changed |
| `StateDelta::SessionStatusChanged` | Session working/idle state |
| `StateDelta::ForegroundChanged` | Foreground session pointer changed |
| `StateDelta::SessionAdded` | New session appeared |
| `StateDelta::Toast` | Notification toast |
| `DaemonEvent::Ack` | Acknowledgement of a request |
| `DaemonEvent::Error { msg }` | Error response |
| `DaemonEvent::Status { ... }` | One-shot session metadata (discovery probe response) |
| `DaemonEvent::OpenSwapper` | Signal client to open session picker |
| `DaemonEvent::NewSession` | Signal client to spawn + attach new session |
| `DaemonEvent::FileSearchResults` | One-shot file search results |
| `DaemonEvent::ModelList` | Model catalogue for GUI picker |
| `DaemonEvent::ModelRoutes` | Provider route list for GUI picker |

### Gap Recovery

If a client detects a seq gap (missing frame), it sends `ClientRequest::Resync`. The daemon answers with a fresh full `DaemonEvent::Snapshot` so the client rebuilds its shadow state from scratch.

## DaemonHub: The Central Coordinator

The `DaemonHub` is the daemon's client registry and state broadcaster. It lives in the sync `daemon_loop` and manages:

### Client Registry

- Each connected client gets a monotonic `client_id` (assigned by the accept loop)
- The **first enrolled client** is the **controller** (has write access)
- Later clients are **read-only observers** (mutating requests rejected with `DaemonEvent::Error`)
- Per-client `mpsc::Sender<DaemonFrame>` channels for frame delivery

### Atomic Attach

A client's frame channel is enrolled NOT-yet-attached. It becomes delta-eligible ONLY in the same tick its `Attach` is handled:

1. Build the full snapshot
2. Send the snapshot to the client
3. Flip the client to "attached"

This eliminates any window where a delta could be born between building the snapshot and the client going live.

### State Streaming

Every tick, after servicing sessions and handling inbound requests:

1. `stream_deltas()` computes a diff of the render state since the last frame
2. Emits `StateDelta` frames to every attached client
3. Full snapshots are only sent on attach/resync — everything else is incremental

## Adaptive Tick Cadence

The daemon loop uses the same adaptive sleep as the TUI:

| State | Cadence | Reason |
|---|---|---|
| Live work (streaming, tools running) | 8 ms | Maintain >=60fps token flushing |
| Idle (no work, no sub-agents) | 100 ms | Minimal CPU usage |
| Parked on approval + detached | 100 ms | Nothing can advance without an operator |

The daemon detects "nothing to do" when:
- Every live session is idle or parked on tool-approval
- No global async work is in flight (catalogue fetch, sub-agent, loading splash)
- If any session is parked, no client is attached (attached = keep fast for responsive approve)

## Self-Exit Grace

When every session is closed (tombstoned) AND no client is enrolled, the daemon starts a grace timer:

- **10 consecutive qualifying ticks** (~1 second at idle cadence)
- Resets to 0 the instant any session is live or a client enrolls
- Before committing to exit: **accept-drain re-check** — drains pending connections once more, re-tests "no client". A client that connected during the grace window aborts the exit.

This prevents a momentary lull (e.g., closing a session while a new client is about to connect) from killing the daemon.

## Detached Approval Park Timeout

When a session is parked on tool-approval and no client is attached for **30 minutes**, the daemon auto-denies the pending risky calls. This prevents an immortal parked daemon holding a session lock with no operator on the wire.

While a client IS attached, the timer never runs — an attached operator can leave an approval pending indefinitely.

## Client Architecture (TUI Side)

### Client Loop

The TUI client runs its own tokio runtime with a loop that:

1. **Services the shadow session** — advances the local copy of session state
2. **Polls crossterm input** — reads key events
3. **Sends requests** to the daemon (keystrokes, submit, approve, etc.)
4. **Receives frames** from the daemon (snapshots, deltas, acks)
5. **Applies deltas** to the shadow state
6. **Renders** if dirty (using ratatui `terminal.draw()`)

### Shadow State

The client maintains a local `AppState` that mirrors the daemon's. This shadow is:

- **Built** from the full snapshot on attach/resync
- **Updated** incrementally via `StateDelta` frames
- **Never executed** — the daemon is the single source of truth

This design means the TUI client never runs the agent, never executes tools, never calls APIs. It only renders and captures input.

### Rendering

Framework: ratatui 0.30.2 with crossterm backend.

Terminal setup: `CrosstermBackend<Stdout>` → `Terminal::new(backend)`. A `TerminalGuard` ensures cleanup on panic/error.

View dispatch is mode-based:

```
AppState.mode →
  Chat        → chat::draw (messages + input bar)
  Settings    → settings::draw (in-app settings dashboard)
  Agents      → agents::draw (sub-agent definitions)
  Mcp         → mcp::draw (MCP server config)
  KeyInput    → key_input::draw (credentials form)
  Todo        → todo::draw (todo list)
  Bash        → bash::draw (background bash output)
  Help        → help::draw
  ...etc
```

### Input Handling

Keystrokes flow through a controller layer:

```
crossterm KeyEvent → controller::input::handle_key → Action enum
```

Actions are dispatched by the event loop against `AppState`. Some actions generate `ClientRequest` messages to send to the daemon (e.g., `Submit`, `Approve`); others are handled purely client-side (e.g., scroll, mode switch).

### Adaptive Rendering

- **8ms** while streaming (>=60fps token flushing)
- **100ms** when idle
- Dirty-flagged rendering: `terminal.draw()` only runs when state has changed

## Daemon ↔ Other Daemons

### MCP Proxy

Session daemons forward MCP tool calls to a global MCP daemon over `~/.koma/mcp.sock`:

```
Session Daemon ──(McpRequest/McpResponse)──→ Global MCP Daemon ──→ MCP Servers
```

The global MCP daemon owns all MCP server connections (stdio and HTTP). N session daemons share ONE copy of every MCP server via this proxy.

### No Cross-Daemon Communication

Session daemons do NOT communicate with each other. Each operates independently. The only shared resource is the on-disk session store (`~/.koma/session.sqlite` and per-session directories), which is accessed without locking (sessions are keyed by UUID, so collisions are impossible).

## Thread Model

### Daemon Side

| Thread | Runtime | Role |
|---|---|---|
| Main thread | None (sync) | `daemon_loop`: services sessions, drives hub, streams deltas |
| Accept loop | tokio (multi-thread) | Binds socket, accepts connections, spawns per-client tasks |
| Per-client tasks | tokio (multi-thread) | Read/write frames over Unix socket, bridge to hub via `mpsc` |
| Tool execution | `std::thread` (NOT tokio) | Inline tool runs (read, write, bash, grep, etc.) to avoid freezing |
| Signal handler | tokio task | Sets `shutting_down` flag on SIGTERM/SIGINT |

### Client Side (TUI)

| Thread | Runtime | Role |
|---|---|---|
| Main thread | tokio (multi-thread) | Client loop: poll input, recv frames, render, send requests |

## Session Storage Layout

```
~/.koma/
  session.sqlite                          ← registry (uuid → pwd_hash, name, timestamps)
  config.json                             ← global AppConfig
  run/
    <session_id>.sock                     ← daemon socket (ephemeral)
  sessions/
    <pwd_hash>/                           ← bucket per working directory
      settings.json                       ← shared LocalConfig (session_models)
      memory/
        MEMORY.md                         ← memory index (injected into system prompt)
        <slug>.md                         ← individual memories
      <uuid>/                             ← one directory per session
        settings.json                     ← per-session settings
        messages.json                     ← conversation history
        messages.sqlite                   ← message log
        plan.md                           ← approved plan
        plan_todos.md                     ← plan-mode todo checklist
        images/                           ← attached images
```

### pwd Hashing

Working directories are hashed via UUID v5 over the OID namespace to produce a stable hex string. This avoids filesystem path encoding issues and ensures the same directory always maps to the same bucket.

## Model/Provider Integration

### Five Model Roles

| Role | Purpose | Default |
|---|---|---|
| Main | Interactive chat | `openai/gpt-4o-mini` via OpenRouter |
| Awareness | Project-doc summary | `openai/gpt-oss-20b` via Groq |
| Safeguard | Safety classifier | `openai/gpt-oss-safeguard-20b` via Groq |
| Compactor | Conversation compaction | Rides Main |
| Planner | Plan mode execution | Falls back to Main |

### Resolution Chain

1. Find model assigned to role (session overrides → global catalogue)
2. Resolve model's provider by UUID against `config.providers`
3. Legacy fallback per role (settings.model/api_key for Main, etc.)

### Provider Types

- **OpenAICompatible**: Standard OpenAI API format
- **AnthropicCompatible**: (deferred)
- **KomaFree**: Keyless tier using `X-Koma` / `X-Session` headers

### OpenRouter Client

Two entry points:
- `stream_complete` — SSE streaming over HTTP, emits `StreamEvent`s
- `complete` — one-shot completion (for `/compact`, classifier)

## Tool System

### Dispatch

Tools are registered as trait objects implementing `Tool`:

```rust
trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn parameters(&self) -> Value;  // JSON Schema
    fn run(&self, ctx: &ToolCtx, args: &Value) -> Result<String>;
}
```

Built-in tools: filesystem (read/write/edit/delete/list), search (grep/glob), shell (bash/output/kill), memory (remember/forget/recall), internet (fetch/search/download), git (operator/cred/worktree), planning (enter/ready/think), task delegation (task/output/kill), and misc (pong/todo).

### Inline Tool Execution

Deferred tools (read, write, edit, bash, grep, glob, etc.) run on a **`std::thread`** (NOT tokio) to avoid freezing the event loop. Tool results are delivered back via unbounded channels.

### Approval Gate

Three modes:

1. **Normal**: Risky tools (write, edit, delete, bash, git, web_download) pause for `y/n` approval
2. **Auto**: Risky tools run inline unless the classifier blocks them
3. **Plan**: Only read-only tools allowed (enforced at both advertise and dispatch level)

### Tool-Call Classifier (TAC)

An off-thread classifier that:
- Sends recent conversation context + tool call to a classifier model
- Verdict: `allow` (run inline), `block` (record error, continue), or `unavailable` (degrade to human)
- Runs asynchronously — does not block the event loop

## System Requirements

### Build Dependencies

- Rust 2021 edition toolchain
- Platform-specific system libraries for webview (GUI feature only)

### Runtime Dependencies

- `~/.koma/` directory (auto-created)
- Python (optional): internet full-mode and security daemon
- MCP servers (optional): any MCP-compatible server

### Platform Support

| Platform | TUI | GUI | Notes |
|---|---|---|---|
| macOS | Full | Full | WKWebView, muda menus, objc2-app-kit |
| Linux | Full | Full | webkit2gtk, GTK webview attachment |
| Windows | Full | Partial | WebView2 (ships with Win 10+) |

### Key Constants

| Constant | Value | Description |
|---|---|---|
| `MAX_FRAME_BYTES` | 64 MiB | Max single IPC frame payload |
| `SELF_EXIT_GRACE_TICKS` | 10 (~1s) | Daemon grace before self-exit |
| `APPROVAL_PARK_TIMEOUT` | 30 min | Auto-deny if detached too long |
| `MAX_SUBAGENTS` | 5 | Max concurrent sub-agents |
| Default model | `openai/gpt-4o-mini` | Via OpenRouter |
| Max tool output | 400,000 chars | Truncation limit |
| Max sub-agent report | 50,000 chars | Completion cap |
| Tool poll interval | 4 ms | Per-client write poll |
| Idle cadence | 100 ms | Sleep when no work |
| Fast cadence | 8 ms | Sleep during active streaming |

## Detach/Resume Flow

### Detach

Client sends `Detach` → daemon deregisters the client → client exits. The daemon keeps running with the session locked. The session remains active and resumable.

### Resume

```
koma agents   # or koma --resume
```

1. Client enters `SessionHub` mode
2. Scans `~/.koma/run/*.sock` for live session-daemons (probes with `Status`)
3. Lists on-disk sessions from the SQLite registry
4. User picks a session
5. Client ensures daemon is running for that session
6. Attaches as client (receives full snapshot)

### Multiple Clients

Multiple clients can attach to the same daemon simultaneously. The first is the controller (write access); others are read-only observers. All receive the same delta stream. Closing one client does not affect others.

## Comparison with GUI Client

| Aspect | TUI | GUI |
|---|---|---|
| Rendering | ratatui + crossterm | React in wry webview |
| Event loop | tokio (adaptive 8ms/100ms) | tao (OS event loop) |
| Input | crossterm KeyEvent → Action | HTML forms + keyboard events |
| State model | AppState (Rust struct) | Zustand store (React) |
| Daemon connection | Client loop (same IPC) | Host-relay thread (same IPC) |
| Config mutations | Mode::Settings editors | Dual-routed (daemon or host-relay) |
| Session switching | SessionHub mode | Hub overlay (JS) |
| File attachments | Paste, @-palette | Drag-drop, paste, omnisearch |
