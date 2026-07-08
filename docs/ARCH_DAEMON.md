# Architecture: Daemon System

Comprehensive documentation of koma's session daemon — its lifecycle, IPC protocol, state management, streaming layer, client coordination, self-test infrastructure, and all supporting subsystems.

## Table of Contents

1. [Design Philosophy](#design-philosophy)
2. [Daemon-per-Session Model](#daemon-per-session-model)
3. [Startup Sequence](#startup-sequence)
4. [The Daemon Loop](#the-daemon-loop)
5. [DaemonHub: Client Coordination](#daemonhub-client-coordination)
6. [IPC Protocol](#ipc-protocol)
7. [Snapshot and Delta System](#snapshot-and-delta-system)
8. [Session Lifecycle](#session-lifecycle)
9. [Signal Handling](#signal-handling)
10. [Self-Exit and Grace](#self-exit-and-grace)
11. [Daemon Management CLI](#daemon-management-cli)
12. [Global MCP Daemon](#global-mcp-daemon)
13. [Self-Test Infrastructure](#self-test-infrastructure)
14. [Thread Model](#thread-model)
15. [Key Constants](#key-constants)

---

## Design Philosophy

koma's daemon is built on several deliberate architectural decisions:

1. **Bind is the liveness oracle.** Who currently holds the bound Unix socket proves liveness. PID files are never used for liveness checks — PIDs get reused, making them unreliable.

2. **Daemon-per-session.** Each daemon owns exactly one session. Multiple sessions = multiple daemons. This eliminates cross-session contention and simplifies locking.

3. **Single-writer.** The first client to `Attach` becomes the controller with write access. Subsequent clients are read-only observers. This prevents concurrent mutation races.

4. **Sync loop, async bridge.** The daemon loop is synchronous (`thread::sleep`, `try_recv`). Per-client I/O is async (tokio tasks). They communicate over `std::sync::mpsc` channels — no `async` in the hot loop.

5. **Correctness-first streaming.** When in doubt, send a full snapshot. A full snapshot is always a valid update. Incremental deltas are an optimization, not a correctness requirement.

6. **Tombstone, never remove.** Sessions are closed by setting a `closed` flag, not by `Vec::remove`. This preserves index stability for the ~40x/tick positional indexing.

---

## Daemon-per-Session Model

```
koma invocation #1 ──→ mints UUID A ──→ spawns daemon A ──→ binds ~/.koma/run/<A>.sock
koma invocation #2 ──→ mints UUID B ──→ spawns daemon B ──→ binds ~/.koma/run/<B>.sock
koma invocation #3 ──→ mints UUID C ──→ connects to C's daemon (already running)
```

Each daemon:
- Owns exactly ONE session (keyed by its UUID)
- Binds to `~/.koma/run/<session_id>.sock`
- Runs independently (no cross-daemon communication)
- Manages its own agent runtime, tool execution, and async work

The spawn-or-attach decision:
1. Client mints a UUID
2. Client tries `UnixStream::connect(~/.koma/run/<uuid>.sock)`
3. Success → daemon is alive → attach as client
4. `ConnectionRefused` → no daemon → spawn one → become the daemon

---

## Startup Sequence

The entry point is `run_daemon()` in `src-agent/src/app/runtime/lifecycle.rs:445`.

### Phase 1: Process Setup

```
1. Ignore SIGPIPE (libc::signal(SIGPIPE, SIG_IGN))
   - Writes to dead clients return EPIPE, never kill the daemon

2. Require --session <id>
   - Daemon-per-session: session ID is mandatory

3. build_startup() — shared with TUI path
   ├── store::ensure_dirs()          — create ~/.koma/ tree
   ├── tokio::runtime::Runtime::new() — multi-thread runtime
   ├── AppConfig::load()             — load ~/.koma/config.json
   ├── prefill_creds()               — inherit last-used api_key/model/provider
   ├── AppState::new(Mode::Chat)     — placeholder state
   ├── build OpenRouterClient        — if Main route resolves
   └── warm_session()                — workspace reindex + awareness
```

### Phase 2: Session Installation

```
4. install_daemon_session(state, client, handle, session_id)
   ├── session_registry::get(id)     — check if session exists
   │   ├── Some → Session::load()    — load from disk
   │   └── None → create_session_in_with_id() — mint new
   ├── write_lock(&session.path)     — acquire PID-based lock
   ├── build SessionRuntime          — wrap in runtime state
   ├── install as single foreground
   ├── restore bg-persist records    — file_changes, plan_todos, bash/subagent
   └── warm_session() if configured  — workspace reindex + awareness
```

### Phase 3: MCP Setup

```
5. MCP proxy configuration
   ├── No mcp_servers configured → mcp_manager = None (no overhead)
   └── Servers configured:
       ├── ensure_mcp_daemon_running()     — spawn global MCP daemon if needed
       ├── McpManager::connect_proxy()     — connect to ~/.koma/mcp.sock
       │   └── OK → use proxy (N daemons share 1 MCP server copy)
       └── ERR → McpManager::connect_all() — fallback to local connections
```

### Phase 4: Infrastructure

```
6. install_daemon_signals(&handle)
   └── Returns Arc<AtomicBool> (shutting_down flag)

7. write_daemon_pid(&session_id)     — advisory pidfile (diagnostics only)

8. DaemonHub::new()
   └── Returns (hub, req_tx)         — sync↔async bridge

9. ipc::server::bind(&sock_path)     — THIS PROCESS IS NOW THE DAEMON
   └── handle.spawn(accept_loop(listener, req_tx))
```

### Phase 5: Enter Loop

```
10. daemon_loop(state, client, handle, hub, shutting_down)
    └── Returns on: QuitDaemon / signal / self-exit
```

### Phase 6: Teardown

```
11. shutdown_runtime(state, rt)       — release all locks, drop runtime
12. remove_file(&sock_path)          — unlink daemon socket
13. remove_file(&pid_path)           — unlink pidfile
```

---

## The Daemon Loop

Located at `src-agent/src/app/runtime/event_loop/daemon/mod.rs:283`.

The loop is **synchronous** — it uses `try_recv()` and `thread::sleep()`, never `.await`. Per-client async tasks run on the tokio runtime separately.

### Tick Structure

```rust
loop {
    // ── 0. Viewed-sessions refresh ───────────────────────────
    hub.refresh_viewed_sessions(state);
    // C2: compute which sessions ANY client is currently viewing.
    // Used by service_all_sessions for per-session gates
    // (background-finish toast, harness toast, stream-start status).

    // ── 1. Service all sessions ──────────────────────────────
    service_all_sessions(state, client, handle);
    // Per session (skip closed):
    //   drain_stream      — tokens/usage/tool-calls/done/error/compacted
    //   drain_subagents   — collect-then-apply, terminal delivery, queued starts
    //   drain_deferred    — tool-task + shell lanes, fire resume gate
    //   nudge_background  — was_working→ready transition fires toast
    //   poll_memory_sync  — cross-instance MEMORY.md mtime check

    // ── 2. Service global concerns ───────────────────────────
    service_global(state, client, handle);
    // endpoints_rx, version_rx, sec_health_rx, oauth_rx,
    // awareness_rx, warm_rx, clipboard_rx, debounced catalogue fetch,
    // loading splash state machine, deferred compaction, missing-root warning,
    // comet-shimmer reconcile, toast tick

    // ── 3. Hub: drain inbound ────────────────────────────────
    hub.drain_inbound(state, client, handle);
    // Process Register/Request/Disconnect from per-client tasks.
    // Controller's mutating requests dispatched here.

    // ── 3-bis/ter. Async reply drains ────────────────────────
    hub.drain_list_models();    // async GET replies → ModelList frames
    hub.drain_list_routes();    // async GET replies → ModelRoutes frames

    // ── 3a-pre: Control-frame drains ─────────────────────────
    hub.drain_select_pending(state);    // /select → EnterSelect
    hub.drain_resume_pending(state);    // /resume → OpenSwapper
    hub.drain_new_pending(state);       // /new → NewSession

    // ── 3a: Should-quit sweep ────────────────────────────────
    if state.rest.should_quit {
        hub.request_shutdown();
    }

    // ── 3a-bis: Approval park timeout ────────────────────────
    service_approval_park_timeouts(state, hub.client_count() > 0);
    // Auto-deny tool approvals parked >30 min while detached.

    // ── 3a-todo: Passive todo refresh ────────────────────────
    // Refresh Todo-mode sessions' TODO.md every 500ms.

    // ── 3b: Stream deltas ────────────────────────────────────
    hub.stream_deltas(state);
    // Diff each client's baseline → emit StateDelta or full Snapshot.

    // ── 3c: Shutdown check ───────────────────────────────────
    if hub.should_shutdown() || shutting_down.load(Relaxed) {
        break;
    }

    // ── 3d: Self-exit grace ──────────────────────────────────
    if all_sessions_closed(state) && hub.client_count() == 0 {
        quiesce_ticks += 1;
        if quiesce_ticks >= 10 {
            hub.drain_inbound_only(state, client, handle); // re-check
            if hub.client_count() == 0 { break; }
            quiesce_ticks = 0;
        }
    } else {
        quiesce_ticks = 0;
    }

    // ── 4. Adaptive sleep ────────────────────────────────────
    sleep(if all_idle_or_parked_detached() { 100ms } else { 8ms });
}
```

### Adaptive Sleep

The daemon alternates between two cadences:

| Condition | Sleep | Purpose |
|---|---|---|
| Live work (streaming, tools, sub-agents) | 8 ms | >=60fps token flushing |
| Idle or parked-detached | 100 ms | Minimal CPU |

`all_idle_or_parked_detached()` returns `true` when:
- No `catalogue_pending`, no Loading-mode sessions, no running sub-agents, no OAuth flow
- No live session with `is_working() && !awaiting_approval`
- If a client IS attached AND a session is parked on approval → **false** (keep fast for responsive approve)

---

## DaemonHub: Client Coordination

Located at `src-agent/src/app/runtime/event_loop/daemon/hub/`.

### HubClient Registry

Each connected client is tracked as:

```rust
struct HubClient {
    id: u64,                          // monotonic connection ID
    frame_tx: Sender<DaemonFrame>,    // per-client frame channel
    is_controller: bool,              // first enrolled = single writer
    attached: bool,                   // true after Attach snapshot sent
    last_seq: u64,                    // per-client monotonic frame seq
    last_snapshot: Option<StateSnapshot>,  // diff baseline
    foreground: Option<String>,       // per-client foreground UUID
    mode_snapshot_cache: Option<(Discriminant<Mode>, Instant, ModeSnapshot)>,
}
```

### HubInbound Messages

Three message types flow from per-client tasks to the hub:

```rust
enum HubInbound {
    Register { client_id: u64, frame_tx: Sender<DaemonFrame> },
    Request { client_id: u64, req: ClientRequest },
    Disconnect { client_id: u64 },
}
```

### Client Lifecycle

1. **Register**: Per-client task sends `Register` with its frame channel. Client is added to `clients` but NOT attached (no snapshot yet).

2. **Attach**: Client sends `ClientRequest::Attach`. Hub:
   - Sends `Hello` frame (build fingerprint for skew detection)
   - Builds full `StateSnapshot` and sends it
   - Marks `attached = true`
   - Seeds `last_snapshot` baseline
   - Client is now delta-eligible

3. **Request dispatch**: Each `Request` goes through:
   - **Load** this client's `foreground` UUID → resolve to Vec index → set transient cursor
   - **Dispatch** the `ClientRequest`
   - **Store** any foreground move back to client's pointer

4. **Detach/Disconnect**: Client removed from registry. If it was the controller, the seat promotes to the first remaining client.

### Single-Writer Enforcement

The **first enrolled client** is the controller. Mutating requests from non-controller clients are rejected with `DaemonEvent::Error("read-only observer")`.

Read-only requests are honored for everyone: `Attach`, `Resync`, `ListSessions`, `Status`, `Detach`, `FileSearch`, `ListModels`, `ListRoutes`.

### Frame Sequence Numbering

Every `DaemonFrame` carries a monotonic `seq` (bumped per frame per client). If a client detects a gap:

1. Client sends `ClientRequest::Resync`
2. Hub answers with a fresh `DaemonEvent::Snapshot`
3. Client rebuilds its shadow state from scratch

### Atomic Attach

A client's frame channel is enrolled NOT-yet-attached. It becomes delta-eligible ONLY in the same tick its `Attach` is handled:

```
1. Build snapshot (captures current state)
2. Send snapshot to client
3. Flip client to attached
```

This eliminates any window where a delta could be born between building the snapshot and the client going live.

### Mode Snapshot Caching

Per-client mode projections are cached with a 100ms TTL (`MODE_SNAPSHOT_TTL`). This bounds intra-variant payload staleness at ~10Hz while avoiding redundant projections when multiple clients view the same mode.

---

## IPC Protocol

### Wire Format

```
┌──────────────────────────┬───────────────────────────────────┐
│ 4 bytes, big-endian u32   │ UTF-8 JSON payload (N bytes)       │
│ length prefix = N          │                                   │
└──────────────────────────┴───────────────────────────────────┘
```

- Max frame: 64 MiB (`MAX_FRAME_BYTES`)
- Both directions use the same framing
- `FrameReader` handles partial reads, coalesced frames, and cap enforcement

### Client → Daemon (`ClientRequest`)

36 variants organized by function:

**Session lifecycle:**
| Variant | Description |
|---|---|
| `Attach { foreground_id, cwd }` | Register as client, receive full state |
| `Detach` | Disconnect (daemon keeps running) |
| `Status` | One-shot metadata probe (no attach) |
| `Resync` | Request fresh full snapshot |
| `ListSessions` | List all sessions |
| `NewSession { name, cwd }` | Create new session |
| `QuitSession { session_id }` | Close (tombstone) a session |
| `QuitDaemon` | Shut down entire daemon (controller-only) |

**Chat and interaction:**
| Variant | Description |
|---|---|
| `SubmitInput { text }` | Send chat message |
| `Shell { cmd }` | Execute shell command |
| `SendKey(KeyWire)` | Forward keystroke |
| `Paste { text }` | Paste text or file path |
| `RemoveAttachment { marker_n }` | Drop staged attachment |
| `Interrupt` | Stop current turn |
| `RewindTo { index }` | Rewind conversation |
| `Compact` | Summarize + trim history |
| `SwitchForeground { session_id }` | Switch foreground session |

**Tool approval:**
| Variant | Description |
|---|---|
| `ApproveTool { approve }` | Answer paused risky call |
| `PlanDecision { decision }` | Answer paused plan (`approve`/`compact`/`deny`) |

**Sub-agent and bash:**
| Variant | Description |
|---|---|
| `KillSubagent { id }` | Kill running sub-agent |
| `BashKill { id }` | Kill background bash job |

**Config (GUI-gated):**
| Variant | Description |
|---|---|
| `SetMcpServer { ... }` | Upsert MCP server |
| `DeleteMcpServer { uuid }` | Remove MCP server |
| `EnableMcpServer { uuid, enabled }` | Toggle MCP server |
| `SetProvider { ... }` | Upsert provider |
| `DeleteProvider { uuid }` | Remove provider |
| `SetModel { ... }` | Upsert model |
| `DeleteModel { uuid, scope }` | Remove model |
| `ListModels { provider }` | Fetch model catalogue |
| `ListRoutes { provider, model_id }` | Fetch provider routes |
| `SetTheme { name }` | Change theme |
| `SetupKomaFree` | Enable keyless tier |

**UI state:**
| Variant | Description |
|---|---|
| `SetMode { mode }` | Change agent mode |
| `SetSessionMain { model_uuid }` | Set session-local model |
| `OpenSessionHub` | Open session picker |
| `FileSearch { query, limit }` | Fuzzy-search workspace |
| `EditorWrapW(usize)` | Set editor wrap width |
| `RenameSession { name }` | Rename foreground session |

### Daemon → Client (`DaemonFrame`)

```rust
struct DaemonFrame {
    seq: u64,        // monotonic per-client
    event: DaemonEvent,
}
```

**`DaemonEvent`** (12 variants):

| Variant | Description |
|---|---|
| `Hello { version }` | Build fingerprint handshake |
| `Snapshot(Box<StateSnapshot>)` | Full state (on attach/resync) |
| `Delta(StateDelta)` | Incremental state change |
| `Ack` | Request acknowledged |
| `Error(String)` | Error response |
| `EnterSelect` | Signal: enter select-copy mode |
| `OpenSwapper` | Signal: open session picker |
| `NewSession { kill }` | Signal: spawn new session daemon |
| `Status(SessionStatus)` | One-shot discovery probe reply |
| `FileSearchResults` | Fuzzy search results |
| `ModelList` | Model catalogue for GUI |
| `ModelRoutes` | Provider route list for GUI |

**`StateDelta`** (9 variants):

| Variant | Description |
|---|---|
| `TokenAppended(String)` | New token appended to streaming content |
| `ReasoningAppended(String)` | New reasoning token |
| `StatusChanged(String)` | Status bar text changed |
| `InputChanged(String)` | Input field changed |
| `ScrollChanged(u16, bool)` | Scroll position changed |
| `SessionStatusChanged(bool)` | Working/idle state changed |
| `ForegroundChanged(String)` | Foreground session pointer changed |
| `SessionAdded` | New session appeared |
| `Toast(String, ToastKind)` | Notification toast |

### Key Wire Format

```rust
struct KeyWire {
    code: KeyCodeWire,  // 15 mapped variants + Other(String) catch-all
    mods: u8,           // SHIFT=0x01, CONTROL=0x02, ALT=0x04
}
```

Round-trips crossterm `KeyEvent` ↔ `KeyWire` for TUI clients. GUI clients translate HTML keyboard events into the same format.

---

## Snapshot and Delta System

### Snapshot Building

`build_snapshot(state)` in `src-agent/src/ipc/snapshot/projection.rs`:

1. Calls `mode_snapshot(state)` to get the per-client mode projection
2. Projects each `SessionRuntime` → `SessionSnapshot` (25+ fields)
3. Captures `foreground_id`
4. Builds `GlobalSnapshot` (input, cursor, scroll, status, mode, toast, providers, config, etc.)

### Diffing

`diff(prev, next)` in `src-agent/src/ipc/snapshot/diff.rs`:

**Structural changes** (triggers full `StateSnapshot`):
- Mode change
- Theme/accent/palette/agent_mode change
- Session set change (add/remove)
- Per-session: messages, committed_reasoning, tokens, approval, subagents, name, cwd, bash_jobs
- Agent viewer/panel state, staged attachments, models cache

**Incremental deltas** (triggers `StateDelta` frames):
- `TokenAppended` — pure suffix append on streaming buffer
- `ReasoningAppended` — pure suffix append on reasoning buffer
- `StatusChanged`, `ScrollChanged`, `InputChanged`, `ForegroundChanged`, `Toast`

**Optimization**: While the sub-agent viewer is closed AND a sub-agent is detached, all content churn is suppressed. Only structural changes (`id`, `name`, `status`, `detached`) fire deltas.

### Diff Protocol

For each attached client, per tick:

```
1. Swap transient foreground to this client's UUID pointer
2. Build mode_snapshot → build_snapshot_with_mode
3. diff(prev_baseline, &next)
   ├── DiffResult::Full(snapshot)  → send full StateSnapshot, reseed baseline
   └── DiffResult::Deltas(vec)    → send each StateDelta
4. force_resync → unconditional full snapshot (set by Interrupt)
```

---

## Session Lifecycle

### Creation

```
1. Mint UUID v4
2. Store::create_session_in_with_id(workdir, id)
   ├── Create directory: ~/.koma/sessions/<pwd_hash>/<uuid>/
   ├── Write settings.json
   ├── Write messages.json (empty)
   └── Register in session.sqlite
3. Acquire lock (PID-based)
4. Build SessionRuntime
5. Install as foreground
```

### Locking

PID-based lock files in the session directory:
- `write_lock(path)` — writes PID, returns guard
- `remove_lock(path)` — removes lock file
- `is_locked(path)` — checks PID, detects stale (process dead)

Stale locks (PID exists but process is dead) are automatically cleaned on acquisition.

### Closing (Tombstone)

`SessionRuntime::close()`:
1. Abort running stream (if any)
2. Kill all sub-agents
3. Drop receivers
4. Release lock
5. Set `closed = true`

The slot stays in `sessions` Vec — never removed (index stability for ~40x/tick positional indexing).

### Foreground Repointing

`repoint_foreground_off_closed(state)`: After any session close, all foreground pointers (global + per-client) are moved to the first non-closed session.

### pwd Hashing

Working directories are hashed via UUID v5 over the OID namespace → stable hex string. This:
- Avoids filesystem path encoding issues
- Ensures same directory always maps to same bucket
- Allows multiple sessions in the same directory to share memory

---

## Signal Handling

Installed by `install_daemon_signals()` in `src-agent/src/app/runtime/signals.rs`:

| Signal | Behavior |
|---|---|
| SIGHUP | Ignored (survive lost terminal) |
| SIGTERM/SIGINT (1st) | Sets `shutting_down` AtomicBool to `true` → graceful shutdown on next tick |
| SIGTERM/SIGINT (2nd) | `std::process::exit(0)` — hard exit, skip teardown |

Registration runs on the tokio runtime. Best-effort: if signal registration fails, the daemon proceeds without it (relying on `QuitDaemon` from a client).

The `shutting_down` flag is polled by the daemon loop each tick via `shutting_down.load(Ordering::Relaxed)`. `Relaxed` is sufficient — this is a single boolean with no other memory dependencies.

---

## Self-Exit and Grace

### Conditions

The daemon self-exits when:
1. Every session is CLOSED (tombstoned)
2. AND no client is enrolled
3. Sustained for `SELF_EXIT_GRACE_TICKS` (10) consecutive ticks

### Grace Mechanism

```
quiesce_ticks = 0;

loop {
    if all_sessions_closed && hub.client_count() == 0 {
        quiesce_ticks += 1;
        if quiesce_ticks >= 10 {
            // ACCEPT-DRAIN RE-CHECK
            hub.drain_inbound_only(state, client, handle);
            if hub.client_count() == 0 {
                break;  // commit to self-exit
            }
            quiesce_ticks = 0;  // client connected during grace → abort exit
        }
    } else {
        quiesce_ticks = 0;  // live session or client → restart clock
    }
}
```

### Accept-Drain Re-Check

Before committing to exit, the daemon drains pending connections one more time. This catches a client that connected DURING the grace window (its `Register` sitting on the channel). If found, the exit is aborted and the counter resets.

This prevents the daemon from exiting while leaving a client with a half-open socket.

---

## Daemon Management CLI

Located at `src-agent/src/app/runtime/manage.rs`.

### Discovery

All discovery uses the bind-as-oracle approach:
```rust
fn daemon_alive(session_id: &str) -> bool {
    UnixStream::connect(&sock_path).is_ok()
}
```

### Key Functions

| Function | Description |
|---|---|
| `ensure_daemon_running(id, resume, workdir)` | Probe → spawn if not live → poll until accepting |
| `spawn_daemon(id, resume, workdir)` | Re-exec current binary with `--daemon --session <id>`, `setsid()`, stdin/stdout/stderr → `/dev/null` |
| `restart_daemon(id, quiet)` | Stop → spawn → confirm accepting |
| `stop_session_daemon(id, quiet)` | QuitDaemon → SIGTERM → SIGKILL escalation |
| `nuke_session_daemon(id)` | Fire-and-forget QuitDaemon (for Ctrl+X in swapper) |
| `probe_status(sock_path)` | Status probe without attaching (500ms timeout) |
| `list_live_sessions()` | Enumerate sockets → probe each → sweep stale |
| `kill_orphan_daemon_processes()` | `/proc` scan for orphaned daemon processes (Linux-only) |

### Constants

| Constant | Value | Purpose |
|---|---|---|
| `SPAWN_CONNECT_TIMEOUT` | 3s | Max wait for spawned daemon to accept |
| `SPAWN_POLL_INTERVAL` | 50ms | Poll interval during spawn wait |
| `SOCKET_IO_TIMEOUT` | 3s | Socket operation timeout |
| `KILL_GRACE` | 3s | Grace period after SIGTERM before SIGKILL |
| `SIGNAL_GRACE` | 2s | Grace period for signal delivery |

### Process Re-exec

`spawn_daemon` re-execs the current binary (not fork+exec):
```
setsid()                        — new session, detach from terminal
--daemon --session <id>         — daemon mode with session key
stdin/stdout/stderr → /dev/null — fully detached
```

This ensures the daemon runs independently of any terminal.

---

## Global MCP Daemon

Located at `src-agent/src/app/runtime/mcp_daemon.rs`.

### Purpose

A singleton process that owns all MCP server connections. Session daemons proxy to it via `~/.koma/mcp.sock`, so N daemons share ONE copy of every MCP server.

### Architecture

```
Session Daemon A ─┐
Session Daemon B ──┤──(McpRequest/McpResponse)──→ Global MCP Daemon ──→ MCP Servers
Session Daemon C ─┘
```

### Request/Response Protocol

| Request | Response |
|---|---|
| `List` | `Tools { defs, names }` — all advertised tool definitions |
| `Call { tool, args }` | `CallResult(output)` or `CallResult(error)` |
| `Reconnect { servers }` | `Ack` — reconnect to specified servers |
| `Status` | `Status(server_status)` — health of each server |

### Reaper

The global MCP daemon self-reaps when no session daemons need it:

| Constant | Value | Purpose |
|---|---|---|
| `REAPER_POLL` | 15s | How often to check for orphaned daemons |
| `REAPER_INITIAL_GRACE` | 15s | Grace period before first reaper check |
| `REAPER_EMPTY_STREAK_TO_EXIT` | 2 | Consecutive empty scans before exit |

### Lifecycle

1. Session daemon needs MCP → `ensure_mcp_daemon_running()`
2. If global daemon not running → spawn it
3. Connect proxy to `~/.koma/mcp.sock`
4. If proxy fails → fallback to local `connect_all()`

---

## Self-Test Infrastructure

### Daemon Self-Test

`koma --daemon-selftest` (`run_daemon_selftest()` in `lifecycle.rs:569`):

1. Creates a private tokio runtime + DaemonHub
2. Binds a test socket (`daemon-selftest.sock`)
3. Spawns the real `daemon_loop` on a std thread with empty state + never-set shutdown flag
4. Client connects → Attach → reads Hello + Snapshot
5. Client sends `SubmitInput` → daemon applies through `Action::Submit` → lands as "no active session" status
6. Client reads `StatusChanged` delta with that status
7. Client sends `QuitDaemon` → expects `Ack` → daemon loop exits
8. Driver thread joins (10s timeout) → cleanup → print OK/FAIL → exit 0/1

### IPC Self-Test

`koma --ipc-selftest` (`ipc/selftest.rs`):

1. Binds test socket
2. Client/server round-trips a `ListSessions` request + `Ack` reply
3. Byte-equality assertion on serialized frames
4. Tests the frame codec in isolation

---

## Thread Model

### Daemon Process

| Thread | Runtime | Role |
|---|---|---|
| Main thread | None (sync) | `daemon_loop`: services sessions, drives hub, streams deltas |
| Accept loop | tokio (multi-thread) | Binds socket, accepts connections, spawns per-client tasks |
| Per-client read | tokio (multi-thread) | `read_loop`: read frames → `HubInbound::Request` |
| Per-client write | tokio (multi-thread) | `write_loop`: poll channel every 4ms → write frames |
| Tool execution | `std::thread` | Inline tool runs (read, write, bash, grep, etc.) |
| Signal handler | tokio task | Sets `shutting_down` on SIGTERM/SIGINT |
| Classifier (TAC) | tokio task | Off-thread tool-call risk classification |
| Sub-agent engine | tokio task | Autonomous LLM-tool loop |

### Sync↔Async Bridge

The hub uses `std::sync::mpsc` (NOT `tokio::sync::mpsc`) for the per-client frame channels:

```rust
// Per-client task creation:
let (frame_tx, frame_rx) = std::sync::mpsc::channel::<DaemonFrame>();
// frame_tx → registered in hub
// frame_rx → held by write_loop task

// Write loop:
loop {
    match frame_rx.recv_timeout(Duration::from_millis(4)) {
        Ok(frame) => { batch.push(frame); }
        Err(Timeout) => { flush_batch(&mut batch); }
        Err(Disconnected) => break,
    }
}
```

This design avoids `std::sync::mpsc::Receiver` being `!Sync` — using `tokio::sync::mpsc` would require the receiver to be held across `.await`, making the future non-`Send`.

### Per-Client Connection Lifecycle

```
1. accept() returns UnixStream
2. spawn(stream, client_id, hub_tx):
   a. Create mpsc::channel<DaemonFrame>()
   b. Send HubInbound::Register { id, frame_tx } to hub
   c. Split stream into OwnedReadHalf / OwnedWriteHalf
   d. Spawn read_task (read_loop):
      - read_frame_from → decode ClientRequest
      - Send HubInbound::Request { id, req }
      - On EOF/error → HubInbound::Disconnect { id }
   e. Spawn write_task (write_loop):
      - Poll frame_rx every 4ms (FRAME_POLL)
      - Collect-then-write batch
      - On Disconnected or write error → exit
```

Two separate tasks (not `select!`) because `std::sync::mpsc::Receiver` is `!Sync` — held across `.await` would make the future non-`Send`.

---

## Key Constants

| Constant | Value | Location | Description |
|---|---|---|---|
| `MAX_FRAME_BYTES` | 64 MiB | `ipc/proto/mod.rs:24` | Max single IPC frame payload |
| `SELF_EXIT_GRACE_TICKS` | 10 (~1s) | `daemon/mod.rs:85` | Consecutive quiescent ticks before self-exit |
| `APPROVAL_PARK_TIMEOUT` | 30 min | `daemon/mod.rs:98` | Auto-deny if detached too long |
| `MAX_SUBAGENTS` | 5 | `subagent/mod.rs:54` | Max concurrent sub-agents |
| `FRAME_POLL` | 4 ms | `ipc/conn.rs` | Per-client write channel poll interval |
| `MODE_SNAPSHOT_TTL` | 100 ms | `hub/core.rs` | Mode projection cache TTL |
| `DEFAULT_MODEL` | `openai/gpt-4o-mini` | `config.rs` | Default model via OpenRouter |
| `SPAWN_CONNECT_TIMEOUT` | 3 s | `manage.rs` | Max wait for spawned daemon |
| `SPAWN_POLL_INTERVAL` | 50 ms | `manage.rs` | Poll interval during spawn wait |
| `SOCKET_IO_TIMEOUT` | 3 s | `manage.rs` | Socket operation timeout |
| `KILL_GRACE` | 3 s | `manage.rs` | Grace after SIGTERM before SIGKILL |
| `REAPER_POLL` | 15 s | `mcp_daemon.rs` | MCP daemon reaper check interval |
| `REAPER_INITIAL_GRACE` | 15 s | `mcp_daemon.rs` | Grace before first reaper check |
| `REAPER_EMPTY_STREAK_TO_EXIT` | 2 | `mcp_daemon.rs` | Empty scans before MCP daemon exits |

---

## Daemon Sub-Commands

The daemon exposes several sub-commands via `koma daemon <verb>`:

| Command | Description |
|---|---|
| `koma daemon status` | List all live daemon sessions with metadata |
| `koma daemon restart <id>` | Stop + respawn a session daemon |
| `koma daemon stop <id>` | Gracefully stop a session daemon |
| `koma daemon nuke <id>` | Force-stop a session daemon (fire-and-forget) |
| `koma daemon orphans` | Kill orphaned daemon processes (Linux `/proc` scan) |

---

## Storage Layout

```
~/.koma/
  config.json                         ← global AppConfig
  session.sqlite                      ← session registry (uuid → pwd_hash, name, timestamps)
  run/
    <session_id>.sock                 ← daemon socket (ephemeral)
    <session_id>.pid                  ← advisory pidfile
  mcp.sock                            ← global MCP daemon socket
  sessions/
    <pwd_hash>/                       ← bucket per working directory
      settings.json                   ← shared LocalConfig
      memory/
        MEMORY.md                     ← memory index
        <slug>.md                     ← individual memories
      <uuid>/                         ← one directory per session
        settings.json                 ← per-session settings
        messages.json                 ← conversation history
        messages.sqlite               ← message log
        plan.md                       ← approved plan
        plan_todos.md                 ← plan-mode todo checklist
        images/                       ← attached images
```

### Daemon Socket Lifecycle

1. **Bind**: `ipc::server::bind()` removes stale socket, then binds fresh
2. **Active**: Listening for client connections
3. **Teardown**: `remove_file(&sock_path)` unlinks after loop exits

The socket file exists only while the daemon is alive. A missing socket means no daemon.

### Pidfile

Advisory only — the bound socket is the real liveness oracle. Written at startup, removed at teardown. Used for diagnostics and `kill` commands.

---

## Edge Cases and Failure Modes

### Stale Socket

A daemon that crashed left a socket file with no listener. `bind()` removes stale sockets before binding. The crash scenario:
1. Daemon crashes → socket file remains, no listener
2. New `koma` invocation → `connect()` → `ConnectionRefused` → tries to become daemon
3. `bind()` → `remove_file(stale)` → bind fresh → success

### Client Disconnect During Frame Send

If a per-client write task gets a write error (client closed), it exits. The hub detects the dead channel on next `send_to()` — the `seq` is NOT advanced for dead sockets (preventing seq inflation).

### Concurrent `/new` and Self-Exit

A momentary lull (closing a session while a new `/new` is in flight) could trip the self-exit grace. The accept-drain re-check catches this: if a client connected during the grace window, the exit is aborted.

### Daemon During Onboarding

A fresh, unconfigured daemon sitting in `Mode::Onboard` can be quit via `Esc`/`q` (which sets `should_quit`). The daemon loop's should-quit sweep latches `hub.request_shutdown()`, ensuring the process exits cleanly instead of lingering.

### Double SIGTERM

First SIGTERM → graceful shutdown (breaks loop, runs teardown). Second SIGTERM → `std::process::exit(0)` (hard exit, skips teardown). This prevents a stuck teardown from making the daemon unkillable.
