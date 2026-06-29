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
    pub foreground: usize,
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
    pub input: String,
    /// Caret position within `input`, as a CHAR index (0..=char_count). Edits
    /// (insert / backspace) and the Left/Right/Home/End keys move it; the view
    /// paints the block cursor here instead of always at the end. Kept in char
    /// units so multibyte input never splits a code point; converted to a byte
    /// offset only at the `String::insert`/`remove` call site. Reset to the end
    /// on any bulk replace (submit/clear, history recall, completion).
    pub cursor: usize,
    /// Image attachments staged by the composer (path-paste / `@`-picker) that
    /// have NOT yet been submitted. Each was produced by the ingest core (its
    /// bytes are already on disk under `<session>/images/`) and matches an
    /// `[Image #N]` marker inserted into `input`. On submit, these are MOVED onto
    /// the user `ChatMessage` and this is cleared; a `/clear` or take_input that
    /// drops the text also clears them so a stray marker can't outlive its image.
    pub pending_attachments: Vec<crate::dto::chat::Attachment>,
    /// Bash-style input history: index into the sent-user-message list while
    /// recalling (None = editing live input).
    pub hist_idx: Option<usize>,
    /// Live input stashed when history recall starts; restored on recall past
    /// the newest entry.
    pub input_stash: String,
    /// Selected row in the `/` command palette (index into the filtered list).
    pub palette_sel: usize,
    pub status: String,
    /// Transient toast: (message, expiry instant, kind). Shown at the top of the
    /// transcript and auto-dismissed once the instant passes. `kind` selects the
    /// box style (red "error" vs neutral "info").
    pub toast: Option<(String, std::time::Instant, ToastKind)>,
    /// True while the `/help` overlay is shown. Any key closes it.
    pub help_open: bool,
    pub should_quit: bool,
    pub scroll: u16,
    /// When true, the transcript stays pinned to the bottom (auto-follows new
    /// content). Cleared when the user scrolls up; re-set on reaching bottom.
    pub follow: bool,
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
}

impl Default for AppStateRest {
    fn default() -> Self {
        Self::new()
    }
}

impl AppStateRest {
    pub fn new() -> Self {
        Self {
            sessions: vec![SessionRuntime::new()],
            foreground: 0,
            prev_session: None,
            spawn_pending: false,
            spawn_prev_fg: 0,
            input: String::new(),
            cursor: 0,
            pending_attachments: Vec::new(),
            hist_idx: None,
            input_stash: String::new(),
            palette_sel: 0,
            status: "ready".into(),
            toast: None,
            help_open: false,
            should_quit: false,
            scroll: 0,
            follow: true,
            last_max_scroll: std::cell::Cell::new(0),
            last_key: None,
            last_esc: None,
            last_model: None,
            last_provider: None,
            config: AppConfig::default(),
            mcp_manager: None,
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
