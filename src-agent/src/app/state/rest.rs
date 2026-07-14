//! [`AppStateRest`] struct definition and its constructor/default impl.
//!
//! The mode-independent "rest of the world" state: the model-catalogue cache, the
//! global config/managers, and the foreground session set. The per-session
//! token/cost counters, the status line + toast (C6), and EXECUTION state (the
//! active [`Session`], the streaming machinery, the tool-approval / sub-agent state
//! machines, …) live in [`SessionRuntime`]; `sessions` always holds at least one and
//! `foreground` indexes the active one. Methods are split into sibling submodules
//! (input, scroll, misc); the streaming-lifecycle + toast methods live on
//! `SessionRuntime`.

use std::cell::RefCell;
use std::collections::HashMap;
use crate::model::app_config::AppConfig;
use crate::service::WarmEvent;
use super::runtime::SessionRuntime;
use super::types::{AgentMode, CataloguePending, TranscriptCache};

pub struct AppStateRest {
    /// The foreground session set. Always non-empty; `foreground` is always a
    /// valid index into it. For now there is exactly ONE entry (single-session);
    /// the multi-session machinery is carved but not yet wired.
    pub sessions: Vec<SessionRuntime>,
    /// Index of the active session in `sessions` (always in range).
    ///
    /// In the daemon (C2) this is a TRANSIENT "currently-acting view cursor": it is
    /// only meaningful while bracketed by ONE client's request (load/store in
    /// `handle_request`) or ONE client's snapshot projection (`stream_deltas`). The
    /// persistent per-client foreground lives on `HubClient::foreground` (a UUID). NO
    /// per-tick background code may rely on this index for per-client correctness —
    /// `service_all_sessions` runs OUTSIDE any client bracket, so it reads `viewed_sessions`
    /// instead. In the LOCAL (single-view) TUI this is still the one true foreground.
    pub foreground: usize,
    /// The set of session UUIDs CURRENTLY VIEWED by some client (C2). Refreshed once
    /// per tick BEFORE `service_all_sessions`: in the DAEMON, from every attached
    /// client's resolved foreground UUID; in the LOCAL loop, the single global
    /// foreground session's UUID. Replaces the stale `idx == foreground` gates in the
    /// per-tick session servicing (background-finish toast / finished-unseen clear /
    /// harness-verdict toast / stream-start status), which must reflect "viewed by ANY
    /// client", not the transient `foreground` cursor. A session viewed by NOBODY
    /// behaves as a pure background session.
    pub viewed_sessions: std::collections::HashSet<String>,
    /// Saved (session) before a /new or reconfigure prompt; restored on cancel.
    pub prev_session: Option<crate::model::session::Session>,
    /// True while a `/new`-spawned PARALLEL session (freshly appended to
    /// `sessions`, no creds yet) is waiting in the KeyInput credential prompt.
    /// If the user Escapes that prompt, the cancel handler pops the just-appended
    /// session back off `sessions`, releases its lock, and restores `foreground`
    /// to `spawn_prev_fg` (so a brand-new empty session never lingers half-made).
    /// Cleared once the creds are confirmed or the cancel is handled.
    pub spawn_pending: bool,
    /// The `foreground` index to restore if a `/new`-spawned session's KeyInput is
    /// cancelled (see `spawn_pending`). Set in `handle_new` just before the new
    /// session is appended + made foreground. Only meaningful while `spawn_pending`.
    pub spawn_prev_fg: usize,
    /// Selected row in the `/` command palette (index into the filtered list).
    pub palette_sel: usize,
    pub should_quit: bool,
    /// Max scroll offset (content_lines - viewport) from the LAST render. The
    /// renderer writes it (via interior mutability through a shared ref); the
    /// key/mouse scroll handlers read it to clamp + detect "at bottom". Single-
    /// threaded UI state, never sent across threads, so `Cell` is fine.
    pub last_max_scroll: std::cell::Cell<u16>,
    /// Persisted scroll offset for the `/` command + `@` file palettes (shared,
    /// since only one shows at a time). Render-owned (interior-mutable, never
    /// serialized); drives `scroll_window` so the selection walks within the
    /// visible rows instead of pinning to the bottom.
    pub palette_offset: std::cell::Cell<usize>,
    /// Persisted scrolloff offsets for the other selectable list overlays. Each
    /// mode-state struct is rebuilt fresh from the IPC snapshot every client frame
    /// (see `app/runtime/client_shadow/modes.rs`), so a `Cell` on those structs
    /// would reset each frame — the offset MUST live here on `AppStateRest` (never
    /// reset by snapshot reconciliation). One cell per independent list; two lists
    /// that can coexist visually (the hub panes, the two model-modal dropdowns) get
    /// distinct cells. All drive `crate::view::scroll::scroll_window`.
    pub help_offset: std::cell::Cell<usize>,
    pub session_picker_offset: std::cell::Cell<usize>,
    pub hub_cooking_offset: std::cell::Cell<usize>,
    pub hub_history_offset: std::cell::Cell<usize>,
    pub rewind_offset: std::cell::Cell<usize>,
    pub todo_offset: std::cell::Cell<usize>,
    pub key_input_results_offset: std::cell::Cell<usize>,
    pub settings_dir_picker_offset: std::cell::Cell<usize>,
    /// Persisted scroll offset for the Appearance category's palette box list
    /// (`draw_palette_list`). Boxes are 3 rows tall (border + swatch + border);
    /// this drives `scroll_window` so the window follows `SettingsState::palette_sel`.
    pub settings_palette_offset: std::cell::Cell<usize>,
    pub model_modal_results_offset: std::cell::Cell<usize>,
    pub model_modal_route_offset: std::cell::Cell<usize>,
    pub agents_tool_picker_offset: std::cell::Cell<usize>,
    pub agents_model_picker_offset: std::cell::Cell<usize>,
    pub effort_offset: std::cell::Cell<usize>,
    pub settings_models_offset: std::cell::Cell<usize>,
    pub subagent_list_offset: std::cell::Cell<usize>,
    pub last_key: Option<String>,
    /// Instant of the most-recent IDLE Esc press in Chat, used to detect a
    /// double-Esc (two idle Escs within ~400ms) that opens the message-rewind
    /// picker. Recorded on the first idle Esc, consumed (compared + cleared) on
    /// the second. `None` when no idle Esc is pending.
    pub last_esc: Option<std::time::Instant>,
    pub last_model: Option<String>,
    /// Most-recently used OpenRouter provider slug (empty string = default routing).
    pub last_provider: Option<String>,
    /// Global application config (theme, accent). Loaded once at startup after
    /// `ensure_dirs`; defaults to `AppConfig::default()` until then.
    pub config: AppConfig,
    /// The GLOBAL MCP client manager, built once at startup from
    /// `config.mcp_servers`. Shared (cloned `Arc`) into every [`crate::tool::ToolCtx`]
    /// so `mcp__*` tool calls can be dispatched to their server. `None` until startup
    /// builds it (and stays `None` for a config with no MCP servers — the manager is
    /// still built but inert, so this is `Some` of an empty manager in practice).
    pub mcp_manager: Option<std::sync::Arc<crate::app::mcp::McpManager>>,
    /// The GLOBAL security daemon client manager, built once at startup. Shared
    /// (cloned `Arc`) into every [`crate::tool::ToolCtx`] so `sec_*` tool calls can
    /// be dispatched to the daemon. `None` until startup builds it (and stays inert
    /// when the daemon is not installed — behaviour is byte-identical to a build
    /// without the security daemon).
    pub sec_manager: Option<std::sync::Arc<crate::app::sec::SecDaemonManager>>,
    /// The extension host manager, built once at startup. Owns every running
    /// extension's child process + duplex unix-socket connection, keyed by extension
    /// id. `None` until startup builds it (stays an empty, inert manager when no
    /// extensions are installed). Boot auto-starts each enabled daemon-kind extension.
    pub ext_manager: Option<std::sync::Arc<crate::app::ext::ExtHostManager>>,
    /// Token koma mints and hands the security daemon child at spawn.
    pub sec_token: String,
    /// Runtime flag: `true` when the user has enabled the security daemon from the
    /// `/security` panel. Starts `false` so the daemon stays off by default even when
    /// installed. The panel's toggle key (`t`) flips this and starts/stops the daemon.
    pub security_enabled: bool,
    /// Layer-1 ARM flag for YOLO mode. `false` by default: until the user explicitly
    /// arms YOLO from the `/security` panel (its "Enable YOLO mode" checkbox row, toggled
    /// with Space/Enter), the `Yolo` agent mode is unreachable — Shift+Tab / `/mode` cycle
    /// Auto<->Normal only. While armed, the user may then ENTER `Yolo` (Layer 2) via
    /// `/mode yolo` or the toggle. Disarming it while currently in `Yolo` drops
    /// `agent_mode` back to `Auto` (see `handle_security_toggle_tool`'s YOLO branch).
    /// Mirrors `security_enabled`'s lifecycle; rides to the thin client in the
    /// `/security` panel's snapshot like `sec_inactive`.
    pub yolo_armed: bool,
    /// Tool names the user has explicitly DISABLED from the `/security` panel (the
    /// inactive set). Empty by default = every tool active, so the stream's
    /// advertise-fold behaves byte-identically to before this feature when nothing has
    /// been toggled off. The fold filters any `sec_` tool whose name is in this set out
    /// of the advertised ToolDefs + allow-list + the awareness tool-list injection, so
    /// disabled tools never bleed into the model's view (e.g. hiding PWN/CRYPTO tools
    /// during WEB work). Toggled by the panel's Enter (one tool) / `d` (whole domain).
    pub sec_inactive: std::collections::HashSet<String>,
    /// Set by `/select`; the event loop performs the terminal hand-off next tick.
    pub select_pending: bool,
    /// True while the conversation is dumped to the normal terminal for copying.
    pub select_active: bool,
    /// Set by `/resume` (and the `OpenSessionHub` request) in the DAEMON: the hub
    /// drains this next tick into a one-shot [`crate::ipc::proto::DaemonEvent::OpenSwapper`]
    /// to the controlling client, mirroring `select_pending` → `EnterSelect`. The
    /// daemon never enters a hub mode itself — the swapper is a purely client-side
    /// overlay — so this is a transient signal, not a UI state. Standalone (no-daemon)
    /// builds never set it: `handle_resume` opens `Mode::SessionHub` directly there.
    pub resume_pending: bool,
    /// Set by `/new` in the DAEMON: a transient "spawn a brand-new session-daemon"
    /// request, drained next tick by the hub into a one-shot
    /// [`crate::ipc::proto::DaemonEvent::NewSession`] to the controlling client (which
    /// detaches — or kills, then detaches — the current daemon and attaches a freshly
    /// minted one). The inner `bool` is the KILL flag: `Some(true)` = `/new kill` (the
    /// current session-daemon is torn down first via `QuitDaemon`); `Some(false)` = plain
    /// `/new` (the current daemon is left cooking, resumable via the swapper). Exactly
    /// mirrors `resume_pending` → `OpenSwapper`: a transient signal, never a UI state. In
    /// the DAEMON-PER-SESSION world a daemon owns ONE session, so `/new` no longer appends
    /// a tab — it makes another DAEMON. Standalone (`--local`, no daemon) NEVER routes here
    /// as a signal: the local loop drains it into
    /// [`crate::app::runtime::commands::new_session::apply_new_session_local`] (the legacy
    /// append-a-session path) so `--local` `/new` behaves exactly as before.
    pub new_pending: Option<bool>,
    /// Set by the extension grant broker's `sessions.switch` (W7) when the target session
    /// uuid is NOT a live session in THIS daemon: a transient "tell the client to attach
    /// that session's OTHER daemon" request, drained next tick by the hub into a one-shot
    /// [`crate::ipc::proto::DaemonEvent::AttachSession`] broadcast to attached clients. The
    /// EXACT mirror of `new_pending` → `NewSession` / `resume_pending` → `OpenSwapper`: a
    /// transient signal, never a UI state, the daemon's own mode left untouched. A
    /// `sessions.switch` to a LIVE local session takes the in-daemon `handle_live_switch`
    /// path instead and never sets this. `None` when no cross-daemon switch is pending.
    pub ext_switch_pending: Option<String>,
    /// Cache of each committed message's rendered visual lines, reused across
    /// frames so markdown/syntect highlighting doesn't re-run every redraw.
    /// Borrowed mutably by the chat renderer through a shared `&rest` (the UI is
    /// single-threaded, so `RefCell` is fine).
    pub transcript_cache: RefCell<TranscriptCache>,
    /// Tool-approval policy. `Auto` runs every tool immediately; `Normal` pauses
    /// for `y/n` on risky (write/delete) tools. Toggled with Shift+Tab / `/mode`.
    pub agent_mode: AgentMode,
    /// The mode to restore when leaving `Plan` mode: set to the PREVIOUS mode
    /// the moment `agent_mode` transitions INTO `Plan`, and cleared back to
    /// `None` the moment it transitions back OUT (either manually via `/mode` /
    /// Shift+Tab, or — in a later wave — via plan approval/denial). `None`
    /// whenever `agent_mode != Plan`. See `set_agent_mode`, the single
    /// choke-point that maintains this invariant.
    pub plan_return_mode: Option<AgentMode>,
    /// One-shot signal set by `handle_approve_plan_compact`: after the
    /// plan-approval compaction completes, `apply_compaction_result` reads
    /// `<session>/plan.md` and appends it as the first post-compaction user turn
    /// so the model executes from a clean context that leads with the plan. A
    /// missing plan.md is silently skipped; the flag is cleared either way.
    pub pending_plan_seed: bool,
    /// Process working directory captured at startup. The deterministic
    /// workspace check (WC) always allows this directory regardless of the
    /// allow-list, so running the agent in the folder you want to work in just
    /// works. Set once in `runtime::run`; never mutated afterwards.
    pub launch_dir: std::path::PathBuf,
    /// Receiver for a model's provider-endpoint fetch. Opened (replacing any
    /// previous, which drops an in-flight older fetch's receiver — the desired
    /// stale-cancel) when the model modal selects/opens an OpenRouter model;
    /// the spawned task sends one [`StreamEvent::EndpointsLoaded`] or
    /// [`StreamEvent::EndpointsError`]. Drained in `run_loop` independently of
    /// streaming. `None` when no endpoints fetch is in flight.
    pub endpoints_rx: Option<tokio::sync::mpsc::UnboundedReceiver<crate::service::StreamEvent>>,
    /// Receiver for warming background tasks. Carries TWO kinds of [`WarmEvent`]:
    /// the startup project-awareness summary (opened by `runtime::warm_session` for
    /// a returning-into-Chat session, folded into `awareness_summary` and advancing
    /// the `LoadingState` splash), and the ON-DEMAND, per-endpoint model catalogue
    /// (opened by the debounced omnisearch fetch in the event-loop tick, folded into
    /// `models_cache` + `models_cache_endpoint`). Drained in `run_loop` independently
    /// of streaming, mirroring `endpoints_rx`. `None` when nothing is in flight.
    pub warm_rx: Option<tokio::sync::mpsc::UnboundedReceiver<WarmEvent>>,
    /// Cached model catalogue (`GET {endpoint}/models`) for ONE endpoint at a
    /// time — the endpoint recorded in `models_cache_endpoint`. Fetched ON DEMAND
    /// (debounced) by the model omnisearch for whichever provider is being edited,
    /// not at boot. `Some(vec![])` is a legitimate "endpoint has no models" state
    /// (the endpoint genuinely returned an empty list). `None` = "never fetched".
    /// Re-fetched when the active omnisearch endpoint differs from
    /// `models_cache_endpoint`.
    pub models_cache: Option<Vec<crate::dto::openrouter::ModelInfo>>,
    /// Which endpoint `models_cache` currently holds models for (`None` when the
    /// cache has never been populated). The omnisearch only filters against
    /// `models_cache` while this equals the active provider's endpoint; otherwise
    /// it shows `searching models…` and (re)requests a fetch.
    pub models_cache_endpoint: Option<String>,
    /// The endpoint whose catalogue fetch LAST FAILED (`None` when no failure is
    /// recorded). Set by the `WarmCatalogueFailed` drain so `request_catalogue`
    /// can skip re-fetching an endpoint that just failed (preventing rapid retry
    /// loops), while still allowing retries when the user explicitly re-triggers
    /// (e.g. switching back to the endpoint later). Cleared on successful fetch.
    pub models_cache_failed: Option<String>,
    /// A debounced catalogue fetch waiting to fire (see [`CataloguePending`]).
    /// Set/refreshed by [`AppStateRest::request_catalogue`]; consumed by the
    /// event-loop tick once `due` passes. `None` when no fetch is pending.
    pub catalogue_pending: Option<CataloguePending>,
    /// The endpoint of a catalogue fetch currently IN FLIGHT (in-flight guard so
    /// the same endpoint isn't fetched twice concurrently). Set when the tick
    /// spawns the fetch; cleared by the `warm_rx` drain when the result lands.
    /// `None` when nothing is being fetched.
    pub catalogue_fetching: Option<String>,
    /// Start instant of the current WORKING wait — the moment the app entered a
    /// model/tool/fold wait that should shimmer (i.e. `waiting && !awaiting_approval`).
    /// Drives the status-line "comet" animation's elapsed counter and its travelling
    /// head. Reconciled on the rising/falling edge in the event-loop tick: set to
    /// `Some(now)` when shimmer becomes active and it's `None`; cleared to `None`
    /// the moment work ends or an approval prompt takes over. `None` when idle.
    pub work_since: Option<std::time::Instant>,
    /// The missing-root set we last warned about, so the toast fires only when
    /// the set changes (not on every reindex).
    pub warned_missing_roots: Vec<String>,
    /// True while the sub-agent panel is open (toggled by the sub-agent UI).
    #[allow(dead_code)]
    pub subagents_open: bool,
    /// Selected row in the sub-agent list (index into the foreground session's
    /// `subagents`).
    #[allow(dead_code)]
    pub subagent_sel: usize,
    /// When `Some(i)`, the full-screen sub-agent VIEWER is open showing
    /// `subagents[i]`'s structured conversation (rendered exactly like the main
    /// chat, view-only). `None` = not viewing. Opened with Enter on a spawned row
    /// in the `$` panel; Esc closes it back to the panel. Short-circuits the
    /// normal chat draw while set (mirrors the full-screen prompt editor).
    pub agent_viewer: Option<usize>,
    /// Scroll offset (top visual line) for the sub-agent viewer. Used only when
    /// `agent_viewer_follow` is false (not pinned). Reset to 0 when the viewer opens.
    pub agent_viewer_scroll: u16,
    /// true = pinned to the newest line; cleared when the user scrolls up,
    /// re-set when they scroll back to the bottom.
    pub agent_viewer_follow: bool,
    /// Receiver for a background clipboard-image fetch (Ctrl+V). The fetch thread
    /// shells out to `wl-paste` (Wayland) or `xclip` (X11), reads raw PNG bytes, and
    /// sends `Ok(bytes)` on success or `Err(reason)` on failure (tool absent, empty
    /// clipboard, non-image data). Drained each tick in `run_loop`; on `Ok` the bytes
    /// are ingested as an attachment; on `Err` a toast is shown. `None` when no fetch
    /// is in flight.
    pub clipboard_rx: Option<std::sync::mpsc::Receiver<Result<Vec<u8>, String>>>,
    /// Pre-fetched `/usage` dashboard data, supplied ONLY on the daemon's thin attach
    /// client (which has no sqlite ledger of its own). `None` on a local TUI: the
    /// `/usage` renderer then collects the data live from the ledger every frame
    /// (unchanged behaviour). `Some(_)` on the client: rebuilt from each
    /// [`crate::ipc::proto::ModeSnapshot::Usage`] payload so the renderer draws the
    /// SAME dashboard without DB access (mirrors how `models_cache` feeds the
    /// omnisearch dropdowns remotely). Read only while in `Mode::Usage`; left `None`
    /// otherwise so it never lingers.
    pub usage_data: Option<crate::model::usage::UsageData>,
    /// Pre-computed `@`-file palette matches, supplied ONLY on the daemon's thin
    /// attach client (whose reconstructed session has an empty `dir_cache`, so it
    /// cannot run the `search` the file-palette view normally calls). `None` on a
    /// local TUI: `view::chat::render_file_palette` then computes the matches live
    /// from `fg().dir_cache` every frame (unchanged behaviour). `Some(_)` on the
    /// client: seeded from each [`crate::ipc::proto::GlobalSnapshot::file_palette`]
    /// so the dropdown renders the SAME entries the daemon computed (mirrors how
    /// `usage_data` feeds the DB-less `/usage` dashboard). Read only while the
    /// composer's last token is an `@partial`; the daemon leaves it `None` otherwise
    /// so it never lingers into an unrelated frame.
    pub file_palette: Option<Vec<String>>,
    /// The newest koma version learned from the public version endpoint. `None`
    /// until the first background check SUCCEEDS (a failed/unreachable check leaves
    /// it `None`, so the UI shows only the current version). Updated in place on the
    /// event-loop tick when a fresh [`crate::app::version::VersionInfo`] arrives;
    /// kept as the LATEST received. Read-only for the UI (next stage), which compares
    /// it against [`crate::model::store::current_version`] via
    /// [`crate::app::version::is_newer`] to decide whether to advertise an update.
    pub latest_version: Option<crate::app::version::VersionInfo>,
    /// Clone-per-spawn SENDER for the background version check. Created once in
    /// `new()` and held for the app's lifetime; every session spawn clones it into a
    /// fresh [`crate::app::version::spawn_check`] thread. Because this end is kept
    /// alive here, the channel never observes a premature `Disconnected` in the drain.
    pub version_tx: Option<tokio::sync::mpsc::UnboundedSender<crate::app::version::VersionInfo>>,
    /// RECEIVER for the background version check, drained each tick in the event
    /// loop (alongside `warm_rx`/`endpoints_rx`). Each `try_recv`'d `VersionInfo` is
    /// stored into `latest_version`. Non-blocking: never awaited.
    pub version_rx: Option<tokio::sync::mpsc::UnboundedReceiver<crate::app::version::VersionInfo>>,
    /// Receiver for an in-flight NON-BLOCKING security health probe. `Some` while a
    /// `SecDaemonManager::health_async` fetch is pending; drained each tick in
    /// `service_global` and folded into the open [`crate::app::mode::SecurityState`]
    /// (`install_health`), then cleared. Mirrors `version_rx`. `None` when no probe is
    /// in flight. Kept out of the IPC snapshot — only the daemon owns the manager, so
    /// only the daemon ever drives a probe; the client animates from the projected
    /// `health_fetching` / `health_frame` instead.
    pub sec_health_rx:
        Option<tokio::sync::mpsc::UnboundedReceiver<Result<Vec<crate::app::sec::InstallHealthEntry>, String>>>,
    /// Receiver for the in-flight `/settings` OAuth submenu connect flow (Codex
    /// browser login or Kilo Code device login). Mirrors `sec_health_rx`: opened
    /// by `Action::OAuthStart`'s handler, drained each tick in `service_global`
    /// and folded into the open OAuth submenu's `oauth_flow` (+ `oauth_drafts` on
    /// success), then cleared. `None` when no flow is in flight.
    pub oauth_rx: Option<tokio::sync::mpsc::UnboundedReceiver<crate::service::oauth::OAuthEvent>>,
    /// Abort handle for the background task behind `oauth_rx`, so `Action::OAuthCancel`
    /// (Esc on a waiting flow) and a fresh `Action::OAuthStart` (superseding an older
    /// flow) can actually stop the in-flight browser/poll loop rather than merely
    /// dropping its receiver. `None` when no flow is in flight.
    pub oauth_task: Option<tokio::task::AbortHandle>,
    /// The hub connection id of the GUI/push client that initiated the CURRENT OAuth login
    /// flow (`ClientRequest::StartOAuth`), or `None` for a TUI-driven flow / no flow. While
    /// `Some`, the OAuth drain (`event_loop::global::drains::drain_oauth`) queues each flow
    /// transition onto `oauth_pushes` for the daemon hub to deliver to this client as a
    /// [`crate::ipc::proto::DaemonEvent::OAuthState`]. Cleared on the terminal event
    /// (success / failure, taken in the drain) and by `CancelOAuth`. The drain's mode-state
    /// fold + config persist are UNCHANGED — this is a parallel side-channel, so TUI parity
    /// is preserved (a TUI client renders the flow off its snapshot as before).
    pub oauth_gui_client: Option<u64>,
    /// Outbox of GUI OAuth phase pushes queued by `drain_oauth` when a background flow
    /// transition lands (only while `oauth_gui_client` is `Some`), drained each tick by the
    /// daemon hub's `drain_oauth_pushes` — which turns each into a seq'd
    /// [`crate::ipc::proto::DaemonEvent::OAuthState`] to the initiating client. Empty except
    /// in the ticks a transition lands; never touched by the standalone/TUI loop.
    pub oauth_pushes: Vec<crate::service::oauth::OAuthPushOut>,
    /// Dedicated lane for OFF-THREAD awareness recomputes triggered by `cd`
    /// (`apply_workspace_change`) and post-`/compact` (`apply_compaction_result`).
    /// Carries `(session_id, summary)` pairs. Deliberately SEPARATE from `warm_rx`:
    /// that channel is REPLACED wholesale on every warm (see its doc), so a
    /// recompute in flight when a new warm starts would be stranded — never
    /// delivered, never re-created. This receiver is created lazily on first use
    /// and lives for the app's lifetime. Drained in `service_global` alongside
    /// `sec_health_rx`/`warm_rx`.
    pub awareness_rx: Option<tokio::sync::mpsc::UnboundedReceiver<(String, Option<String>)>>,
    /// SENDER half of `awareness_rx`, cloned into each spawned recompute task.
    /// `None` until the first recompute is spawned (see `session_mgmt::spawn_awareness_recompute`).
    pub awareness_tx: Option<tokio::sync::mpsc::UnboundedSender<(String, Option<String>)>>,
    /// SENDER half of the extension grant-broker lane. A clone is handed to
    /// [`crate::app::ext::ExtHostManager`] at startup (`set_ext_call_tx`); each
    /// extension's socket reader task uses it to forward an `agents.*` `Call` — which
    /// needs `AppState`/session access the reader task lacks — into the event loop.
    /// Created once here and held for the app's lifetime (plus the manager's clone),
    /// so the paired receiver never observes a premature `Disconnected`.
    pub ext_call_tx: tokio::sync::mpsc::UnboundedSender<crate::app::ext::ExtCallRequest>,
    /// RECEIVER half of the extension grant-broker lane, drained each tick in
    /// `service_global` (`drains::drain_ext_calls`): each [`crate::app::ext::ExtCallRequest`]
    /// is dispatched against the ACTIVE session through the grant broker and its
    /// oneshot answered with the broker's JSON. `Option` only so the drain can
    /// take/put-back it (mirroring `oauth_rx`); always `Some` between ticks.
    pub ext_call_rx:
        Option<tokio::sync::mpsc::UnboundedReceiver<crate::app::ext::ExtCallRequest>>,
    /// SENDER half of the extension notify lane. A clone is handed to
    /// [`crate::app::ext::ExtHostManager`] at startup (`set_ext_notify_tx`); each
    /// extension's socket reader task uses it to forward an ext->koma `Notify` — which
    /// needs `AppState` access the reader task lacks — into the event loop. Created
    /// once here and held for the app's lifetime (plus the manager's clone), so the
    /// paired receiver never observes a premature `Disconnected`.
    pub ext_notify_tx: tokio::sync::mpsc::UnboundedSender<crate::app::ext::ExtNotify>,
    /// RECEIVER half of the extension notify lane, drained each tick in `service_global`
    /// alongside `ext_call_rx` (`drains::drain_ext_notifies`, W8 panel bridge): each
    /// [`crate::app::ext::ExtNotify`] is routed by name — a well-formed `panel.push` is queued
    /// onto [`Self::ext_panel_pushes`] for the daemon hub to broadcast to attached panels; a
    /// malformed `panel.push` or any unknown notify name is logged + dropped. `Option` only so
    /// the drain can take/put-back it (mirroring `ext_call_rx`); always `Some` between ticks.
    pub ext_notify_rx: Option<tokio::sync::mpsc::UnboundedReceiver<crate::app::ext::ExtNotify>>,
    /// BOUNDED daemon→panel push outbox (W8 panel bridge): `(ext_id, panel_id, payload)` triples
    /// queued by `drains::drain_ext_notifies` when an extension sends a `panel.push`, drained
    /// each tick by the daemon hub's `drain_ext_panel_pushes` — which broadcasts each as a seq'd
    /// [`crate::ipc::proto::DaemonEvent::ExtPanelPush`] to every ATTACHED client. Capped at 256
    /// with drop-OLDEST (a live panel wants the freshest state, not a backlog); the drain logs a
    /// single overflow line per over-cap tick. Parallels `oauth_pushes`, but with a KEY
    /// difference: `oauth_pushes`' producer is gated on the daemon-only `oauth_gui_client`, so it
    /// stays empty in the standalone/TUI loop for free — a panel push has no such daemon-only
    /// producer gate (an extension pushes regardless of GUI), so the standalone/TUI `run_loop`
    /// CLEARS this each tick (there is no panel to receive it) rather than letting it retain.
    pub ext_panel_pushes: Vec<(String, String, serde_json::Value)>,
    /// Per-extension registry of the sub-agents THAT extension has spawned,
    /// keyed by extension id. This is the containment boundary the grant broker
    /// (`app::ext::broker`) resolves every `agents.status`/`agents.result`/
    /// `agents.kill` call through: an extension can only ever name an ext-facing
    /// agent id it was itself handed by `agents.spawn`, which stays bound to the
    /// STABLE session UUID it was spawned into — never the transient foreground
    /// index, so a foreground switch can never redirect a poll at a different
    /// session's (or another extension's, or the user's own) sub-agent. Entries
    /// accumulate for the app's lifetime; see `ExtAgentRegistry`'s doc for the
    /// full rationale. `// TODO cleanup on uninstall`: an entry should ideally be
    /// dropped when its extension is stopped/uninstalled, but `ExtHostManager::stop`
    /// is called both from a background reader task (frame-too-large kill,
    /// `wire.rs`) and from `ExtHostManager`'s own methods, neither of which has
    /// `AppState` access — only `shutdown_runtime` (whole-app teardown) does, and
    /// it clears every entry there. A per-extension uninstall path with event-loop
    /// access should clear its own entry the same way.
    pub ext_agents: HashMap<String, crate::app::ext::ExtAgentRegistry>,
    /// Per-extension PUBLISHED CONTEXT blobs (`context.set` / `context.clear`),
    /// keyed by the calling extension's id. A `BTreeMap` (not `HashMap`) so
    /// iteration is in deterministic key order → the System-prompt VOLATILE TAIL
    /// these ride in (appended in `stream::run::start_stream_task`, AFTER the
    /// `CACHE_SPLIT_MARK`, so an update never busts the provider-cached head) is
    /// byte-stable across turns. One extension can only ever read/replace its OWN
    /// entry (keyed by caller identity), never another's. An empty map contributes
    /// nothing to the tail (byte-identical to before this feature). Purely
    /// in-memory / transient — `AppStateRest` is never serialised.
    pub ext_context: std::collections::BTreeMap<String, String>,
}

impl Default for AppStateRest {
    fn default() -> Self {
        Self::new()
    }
}

impl AppStateRest {
    pub fn new() -> Self {
        // Version-check channel, created ONCE here: the sender is cloned per session
        // spawn into a background `spawn_check` thread; the receiver is drained each
        // event-loop tick into `latest_version`. Holding the sender for the app's
        // lifetime keeps the drain from ever seeing a premature `Disconnected`.
        let (vtx, vrx) = tokio::sync::mpsc::unbounded_channel();
        // Extension grant-broker lane, created ONCE here: the sender is cloned into
        // `ExtHostManager` at startup and kept here for the app's lifetime (so the
        // drain never sees a premature `Disconnected`); the receiver is drained each
        // tick in `service_global`.
        let (ext_call_tx, ext_call_rx) = tokio::sync::mpsc::unbounded_channel();
        // Extension notify lane, created ONCE here for the same reason as
        // `ext_call_tx`/`ext_call_rx` above.
        let (ext_notify_tx, ext_notify_rx) = tokio::sync::mpsc::unbounded_channel();
        let first = SessionRuntime::new();
        // Seed the viewed set with the sole session's UUID so the per-tick gates treat
        // it as foreground from tick zero (the local loop re-derives this each tick; the
        // daemon refreshes from its attached clients — but a freshly-built state always
        // has its one session "viewed" until a loop overwrites it).
        let viewed_sessions = std::iter::once(first.id.clone()).collect();
        Self {
            sessions: vec![first],
            foreground: 0,
            viewed_sessions,
            prev_session: None,
            spawn_pending: false,
            spawn_prev_fg: 0,
            palette_sel: 0,
            should_quit: false,
            last_max_scroll: std::cell::Cell::new(0),
            palette_offset: std::cell::Cell::new(0),
            help_offset: std::cell::Cell::new(0),
            session_picker_offset: std::cell::Cell::new(0),
            hub_cooking_offset: std::cell::Cell::new(0),
            hub_history_offset: std::cell::Cell::new(0),
            rewind_offset: std::cell::Cell::new(0),
            todo_offset: std::cell::Cell::new(0),
            key_input_results_offset: std::cell::Cell::new(0),
            settings_dir_picker_offset: std::cell::Cell::new(0),
            settings_palette_offset: std::cell::Cell::new(0),
            model_modal_results_offset: std::cell::Cell::new(0),
            model_modal_route_offset: std::cell::Cell::new(0),
            agents_tool_picker_offset: std::cell::Cell::new(0),
            agents_model_picker_offset: std::cell::Cell::new(0),
            effort_offset: std::cell::Cell::new(0),
            settings_models_offset: std::cell::Cell::new(0),
            subagent_list_offset: std::cell::Cell::new(0),
            last_key: None,
            last_esc: None,
            last_model: None,
            last_provider: None,
            config: AppConfig::default(),
            mcp_manager: None,
            sec_manager: None,
            ext_manager: None,
            sec_token: String::new(),
            security_enabled: false,
            yolo_armed: false,
            sec_inactive: std::collections::HashSet::new(),
            select_pending: false,
            select_active: false,
            resume_pending: false,
            new_pending: None,
            ext_switch_pending: None,
            transcript_cache: RefCell::new(TranscriptCache::default()),
            agent_mode: AgentMode::default(),
            plan_return_mode: None,
            pending_plan_seed: false,
            launch_dir: std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
            endpoints_rx: None,
            warm_rx: None,
            models_cache: None,
            models_cache_endpoint: None,
            models_cache_failed: None,
            catalogue_pending: None,
            catalogue_fetching: None,
            work_since: None,
            warned_missing_roots: Vec::new(),
            subagents_open: false,
            subagent_sel: 0,
            agent_viewer: None,
            agent_viewer_scroll: 0,
            agent_viewer_follow: true,
            clipboard_rx: None,
            usage_data: None,
            file_palette: None,
            latest_version: None,
            version_tx: Some(vtx),
            version_rx: Some(vrx),
            sec_health_rx: None,
            oauth_rx: None,
            oauth_task: None,
            oauth_gui_client: None,
            oauth_pushes: Vec::new(),
            awareness_rx: None,
            awareness_tx: None,
            ext_call_tx,
            ext_call_rx: Some(ext_call_rx),
            ext_notify_tx,
            ext_notify_rx: Some(ext_notify_rx),
            ext_panel_pushes: Vec::new(),
            ext_agents: HashMap::new(),
            ext_context: std::collections::BTreeMap::new(),
        }
    }

    /// Borrow the foreground session's runtime (read-only).
    pub fn fg(&self) -> &SessionRuntime {
        &self.sessions[self.foreground]
    }

    /// Borrow the foreground session's runtime (mutable).
    pub fn fg_mut(&mut self) -> &mut SessionRuntime {
        let i = self.foreground;
        &mut self.sessions[i]
    }

    /// Snapshot the foreground session's resolved Main-route identity — call
    /// BEFORE a settings mutation that might reassign the Main role, then pass
    /// the result to [`Self::reset_effort_if_main_changed`] after the mutation
    /// to detect (and correct for) an actual model swap.
    pub fn main_identity_now(&self) -> Option<(String, String, Option<String>)> {
        self.fg()
            .session
            .as_ref()
            .and_then(|s| crate::app::resolve::main_identity(&self.config, &s.settings))
    }

    /// BUG FIX: reset the foreground session's `settings.effort` to the
    /// model-default (unset) value when the resolved Main route CHANGED since
    /// `before` was captured (via [`Self::main_identity_now`]) — i.e. the Main
    /// model this session chats with actually swapped, not merely a re-pick of
    /// the same model or a change to some OTHER role. Different models expose
    /// different effort scales (see
    /// `app::runtime::commands::effort::{build_effort_options, preselect_effort}`,
    /// where an empty string is exactly the value that preselects "default"),
    /// so a stale effort chosen for the OLD model must never silently carry
    /// onto the new one. No-ops (skips the write + extra save) when nothing
    /// changed or `effort` is already the default — safe to call
    /// unconditionally after any settings mutation that might touch Main.
    pub fn reset_effort_if_main_changed(&mut self, before: Option<(String, String, Option<String>)>) {
        let after = self.main_identity_now();
        if before != after {
            if let Some(sess) = self.fg_mut().session.as_mut() {
                if !sess.settings.effort.is_empty() {
                    sess.settings.effort = String::new();
                    let _ = sess.save();
                }
            }
        }
    }

    /// Single choke-point for changing `agent_mode` — used by both the
    /// Shift+Tab cycle and `/mode`, so `plan_return_mode` and the Plan-mode
    /// system-prompt nudge never drift out of sync between the two entry
    /// points.
    ///
    /// - Entering `Plan` from anything else remembers the previous mode in
    ///   `plan_return_mode`.
    /// - Leaving `Plan` (to any other mode) clears `plan_return_mode`.
    /// - Crossing the Plan boundary (either direction) also flips the
    ///   foreground session's `plan_mode_hint` and rebuilds + saves its
    ///   system prompt, so the nudge appears/disappears immediately. A
    ///   same-tier move (Auto↔Normal↔Yolo) leaves the prompt untouched.
    pub fn set_agent_mode(&mut self, new_mode: AgentMode) {
        let old_mode = self.agent_mode;
        if old_mode == new_mode {
            return;
        }
        let entering_plan = new_mode == AgentMode::Plan;
        let leaving_plan = old_mode == AgentMode::Plan;
        if entering_plan {
            self.plan_return_mode = Some(old_mode);
            // Belt-and-suspenders: a fresh plan cycle starts clean, so drop any
            // approved-plan stash left from a prior cycle before the classifier could
            // be fed a stale plan.
            self.fg_mut().approved_plan = None;
        } else if leaving_plan {
            self.plan_return_mode = None;
        }
        self.agent_mode = new_mode;
        // Captured inside the `sess` borrow below and applied to `self.fg_mut()`
        // afterward (can't touch `self` again while `sess` — itself borrowed FROM
        // `self.fg_mut()` — is still alive), so the GUI Explore "PLAN" section's
        // in-memory mirror stays in lockstep with the rail seed/clear on disk.
        let mut plan_todos_after: Option<Vec<crate::app::mode::todo::TodoItem>> = None;
        if entering_plan || leaving_plan {
            if let Some(sess) = self.fg_mut().session.as_mut() {
                sess.plan_mode_hint = entering_plan;
                // Seed the two locked plan rails on entry, UNCONDITIONALLY. This
                // guard-checking `old_mode == new_mode` early-returns above, and the
                // `plan_enter` interception short-circuits when already in Plan, so
                // `entering_plan` only ever fires on a genuine transition INTO plan —
                // a fresh rail seed is always correct there, and overwrites any stale
                // plan_todos.md left behind by an abnormal exit (crash/kill mid-plan,
                // since `agent_mode` is process-local and resets on restart).
                if entering_plan {
                    use crate::app::mode::todo::{
                        self, TodoItem, TodoPriority, TodoStatus, PLAN_RAIL_SAVE, PLAN_RAIL_SERVE,
                    };
                    let path = sess.plan_todos_path();
                    let rails = vec![
                        TodoItem {
                            content: PLAN_RAIL_SERVE.to_string(),
                            status: TodoStatus::Pending,
                            priority: TodoPriority::Low,
                            locked: true,
                        },
                        TodoItem {
                            content: PLAN_RAIL_SAVE.to_string(),
                            status: TodoStatus::Pending,
                            priority: TodoPriority::Low,
                            locked: true,
                        },
                    ];
                    let _ = todo::save_todos_to(&path, &rails);
                    plan_todos_after = Some(rails);
                } else if leaving_plan {
                    // Leaving Plan for any non-plan mode (plan approved, `/mode`,
                    // Shift+Tab) drops the plan-specific `plan_todos.md` artifact so
                    // it never lingers into the next planning session. Best-effort
                    // remove — a missing file (NotFound) is fine. Deny STAYS in
                    // Plan, so this never fires on "chat more".
                    let _ = std::fs::remove_file(sess.plan_todos_path());
                    // The mirror itself does NOT clear to empty here: it mirrors
                    // the session's CURRENT todo list, not Plan-mode membership.
                    // Leaving Plan means the per-directory `memory/TODO.md` (the
                    // regular working list) is now the source of truth — read
                    // it immediately so an approved plan that carries into
                    // execution keeps showing its checklist instead of the GUI
                    // Explore "PLAN" section going blank until the model's next
                    // `checklist`. Empty when that file doesn't exist yet.
                    plan_todos_after = Some(crate::app::mode::todo::load_current_todos(sess, false));
                }
                sess.rebuild_system();
                let _ = sess.save();
            }
            if let Some(todos) = plan_todos_after {
                self.fg_mut().plan_todos = todos;
            }
        }
    }

    /// Resolve a per-client foreground POINTER (a stable session UUID, or `None`) to a
    /// concrete index into `sessions` (C2). Sessions are append+tombstone and addressed
    /// by UUID, so the index is resolved at the point of use. Fallback when the UUID is
    /// `None` or no longer resolvable: the FIRST non-closed session, else `0` (there is
    /// always at least one slot). Used to bracket each client's request / snapshot so the
    /// existing `fg()`-based handlers and the snapshot projection act on THAT client's view.
    pub fn resolve_foreground(&self, id: Option<&str>) -> usize {
        if let Some(id) = id {
            if let Some(i) = self.sessions.iter().position(|s| s.id == id) {
                return i;
            }
        }
        self.sessions
            .iter()
            .position(|s| !s.closed)
            .unwrap_or(0)
    }

    /// Reset the scroll/follow of session `idx` itself (snap it to the bottom), instead of
    /// the foreground session (C2). `scroll`/`follow` are PER-SESSION (C1), so a stream
    /// that starts on session `idx` snaps ITS OWN view to the newest line regardless of
    /// which client is currently the acting cursor — preserving the original visible
    /// effect (the client viewing `idx` sees the snap-to-bottom) while never yanking an
    /// unrelated session's scroll. Mirrors [`reset_scroll`] but targets `sessions[idx]`.
    pub fn reset_scroll_at(&mut self, idx: usize) {
        if let Some(s) = self.sessions.get_mut(idx) {
            s.follow = true;
            s.scroll = 0;
        }
    }

    /// Seed session `sess_idx`'s cumulative token/cost counters from its sqlite
    /// log (0 if absent). Called when that session is loaded/created so its OWN
    /// counters reflect prior usage; never touches any other session's totals.
    pub fn load_token_totals(&mut self, sess_idx: usize, session_dir: &std::path::Path) {
        let (i, o, c) = crate::model::msglog::totals(session_dir).unwrap_or((0, 0, 0.0));
        let rt = &mut self.sessions[sess_idx];
        rt.tokens_in = i;
        rt.tokens_out = o;
        rt.cost = c;
    }
}
