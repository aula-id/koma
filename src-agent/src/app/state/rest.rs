//! [`AppStateRest`] struct definition and its constructor/default impl.
//!
//! The mode-independent "rest of the world" state: input buffer, status line,
//! scroll, model-catalogue cache, and the foreground session set. The
//! per-session token/cost counters and EXECUTION state (the active [`Session`], the
//! streaming machinery, the tool-approval / sub-agent state machines, …) lives
//! in [`SessionRuntime`]; `sessions` always holds at least one and `foreground`
//! indexes the active one. Methods are split into sibling submodules (input,
//! scroll, misc); the streaming-lifecycle methods live on `SessionRuntime`.

use std::cell::RefCell;
use crate::model::app_config::AppConfig;
use crate::service::WarmEvent;
use super::runtime::SessionRuntime;
use super::types::{AgentMode, CataloguePending, ToastKind, TranscriptCache};

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
    pub status: String,
    /// Transient toast: (message, expiry instant, kind). Shown at the top of the
    /// transcript and auto-dismissed once the instant passes. `kind` selects the
    /// box style (red "error" vs neutral "info").
    pub toast: Option<(String, std::time::Instant, ToastKind)>,
    pub should_quit: bool,
    /// Max scroll offset (content_lines - viewport) from the LAST render. The
    /// renderer writes it (via interior mutability through a shared ref); the
    /// key/mouse scroll handlers read it to clamp + detect "at bottom". Single-
    /// threaded UI state, never sent across threads, so `Cell` is fine.
    pub last_max_scroll: std::cell::Cell<u16>,
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
    /// Cache of each committed message's rendered visual lines, reused across
    /// frames so markdown/syntect highlighting doesn't re-run every redraw.
    /// Borrowed mutably by the chat renderer through a shared `&rest` (the UI is
    /// single-threaded, so `RefCell` is fine).
    pub transcript_cache: RefCell<TranscriptCache>,
    /// Tool-approval policy. `Auto` runs every tool immediately; `Normal` pauses
    /// for `y/n` on risky (write/delete) tools. Toggled with Shift+Tab / `/mode`.
    pub agent_mode: AgentMode,
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
    /// not at boot. `Some(vec![])` is a TERMINAL "no models / fetch failed" state
    /// for that endpoint (degrade to manual model-id entry), distinct from `None`
    /// = "never fetched". Re-fetched when the active omnisearch endpoint differs
    /// from `models_cache_endpoint`.
    pub models_cache: Option<Vec<crate::dto::openrouter::ModelInfo>>,
    /// Which endpoint `models_cache` currently holds models for (`None` when the
    /// cache has never been populated). The omnisearch only filters against
    /// `models_cache` while this equals the active provider's endpoint; otherwise
    /// it shows `searching models…` and (re)requests a fetch.
    pub models_cache_endpoint: Option<String>,
    /// A debounced catalogue fetch waiting to fire (see [`CataloguePending`]).
    /// Set/refreshed by [`AppStateRest::request_catalogue`]; consumed by the
    /// event-loop tick once `due` passes. `None` when no fetch is pending.
    pub catalogue_pending: Option<CataloguePending>,
    /// The endpoint of a catalogue fetch currently IN FLIGHT (in-flight guard so
    /// the same endpoint isn't fetched twice concurrently). Set when the tick
    /// spawns the fetch; cleared by the `warm_rx` drain when the result lands.
    /// `None` when nothing is being fetched.
    pub catalogue_fetching: Option<String>,
    /// Start instant of the `/compact` animation. `Some` only while a compaction
    /// is in flight (set in `Command::Compact`, cleared once the result is
    /// applied). The renderer uses it to draw the spinner + elapsed + indeterminate
    /// bar, and the event loop uses it both to keep redrawing each tick (so the
    /// animation actually animates) and to enforce the cosmetic minimum duration.
    pub compact_anim_start: Option<std::time::Instant>,
    /// Earliest instant the stashed compaction result may be applied. Set when a
    /// fast `StreamEvent::Compacted` arrives before the minimum animation duration
    /// has elapsed; the event loop applies `compact_pending` once `now >= this`.
    pub compact_apply_at: Option<std::time::Instant>,
    /// Stashed `(summary, kept_tail)` awaiting the minimum-duration gate. Held
    /// only when a compaction finished faster than the minimum so the apply is
    /// deferred (non-blocking) rather than slept on. Applied by the event loop.
    pub compact_pending: Option<(String, Vec<crate::dto::chat::ChatMessage>)>,
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
            status: "ready".into(),
            toast: None,
            should_quit: false,
            last_max_scroll: std::cell::Cell::new(0),
            last_key: None,
            last_esc: None,
            last_model: None,
            last_provider: None,
            config: AppConfig::default(),
            mcp_manager: None,
            sec_manager: None,
            sec_token: String::new(),
            security_enabled: false,
            yolo_armed: false,
            sec_inactive: std::collections::HashSet::new(),
            select_pending: false,
            select_active: false,
            transcript_cache: RefCell::new(TranscriptCache::default()),
            agent_mode: AgentMode::default(),
            launch_dir: std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
            endpoints_rx: None,
            warm_rx: None,
            models_cache: None,
            models_cache_endpoint: None,
            catalogue_pending: None,
            catalogue_fetching: None,
            compact_anim_start: None,
            compact_apply_at: None,
            compact_pending: None,
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
