//! Wire protocol vocabulary for the koma daemon <-> client split.
//!
//! These are PURE-DATA, serde-round-trippable types — the only things that ever
//! cross the unix-socket boundary between the headless `koma-daemon` (which owns
//! the agent runtime + session locks) and a thin attach/detach TUI client.

pub mod key;
pub mod snapshot;
pub mod stream;

// Re-export everything so downstream `crate::ipc::proto::*` paths keep working.
#[allow(unused_imports)]
pub use key::{key_mods, KeyCodeWire, KeyWire};
pub use snapshot::*;
#[allow(unused_imports)]
pub use stream::StreamEventWire;

use serde::{Deserialize, Serialize};

// ─── frame constants ─────────────────────────────────────────────────────────

/// Hard upper bound on a single length-prefixed frame's payload size (64 MiB).
#[allow(dead_code)]
pub const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;

// ─── lightweight session metadata ────────────────────────────────────────────

/// A NON-attaching snapshot of one session-daemon's single owned session — the data
/// the hub/swapper collects when DISCOVERING live daemons (it probes each
/// `run/<id>.sock` for this, never opening a full attach/snapshot stream). Carries
/// only the few fields the picker needs: which session it is, its display name, its
/// working dir (to disambiguate two sessions of the same name in the hub), and whether
/// its agent is currently cooking.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SessionStatus {
    pub session_id: String,
    pub name: String,
    pub pwd: String,     // the session's working dir, for disambiguation in the hub
    pub working: bool,   // is the session's agent currently cooking?
}

/// One workspace-file hit from a [`ClientRequest::FileSearch`] (the GUI omnisearch
/// overlay). `label` is the display string exactly as the `@`-palette produces it (the
/// workspace-relative path, `[N]`-prefixed in multi-root mode, trailing `/` for a dir);
/// `path` is the ABSOLUTE on-disk path the daemon resolved from it, ready to hand back
/// as an [`ClientRequest::Paste`]/attach target (empty for a directory row, which is not
/// attachable).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FileSearchItem {
    pub path: String,
    pub label: String,
}

// ─── client -> daemon ────────────────────────────────────────────────────────

/// A request sent from a TUI client to the daemon over the unix socket.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub enum ClientRequest {
    Attach {
        foreground_id: Option<String>,
        cwd: Option<String>,
    },
    Detach,
    ListSessions,
    /// Lightweight metadata probe used by live-session DISCOVERY: ask the daemon for a
    /// one-shot [`DaemonEvent::Status`] describing its single owned session, with NO
    /// attach and NO snapshot stream. The daemon must answer this WITHOUT mutating any
    /// session state (no create/attach, no foreground change, no Hello/Snapshot).
    Status,
    /// FIRE-AND-FORGET cross-daemon sub-agent spawn (extension `sessions.spawn_into`, W7):
    /// one session-daemon's grant broker connects ANOTHER session-daemon's keyed socket and
    /// sends this to spawn a sub-agent INTO that daemon's own foreground/first-live session,
    /// through the SAME `spawn_or_queue` path the model's `task` tool uses (`tool_call_id`
    /// None, `detached` false). No attach, no streaming: the sending side speaks the blocking
    /// management codec (like the `Status` discovery probe), reads a single
    /// [`DaemonEvent::Ack`] (accepted/queued) or [`DaemonEvent::Error`] (failure), and closes
    /// — the connection is NEVER enrolled as an attached client owing snapshots. `agent`
    /// defaults to the built-in general agent when absent; `model`/`effort` are optional
    /// per-spawn route overrides (empty = absent). v1 is intentionally poll-less: the target
    /// daemon owns the resulting sub-agent, and the caller receives no ext-facing agent id
    /// (no cross-daemon `agents.status`/`result` polling yet).
    SpawnAgent {
        agent: Option<String>,
        task: String,
        model: Option<String>,
        effort: Option<String>,
    },
    Resync,
    SwitchForeground { session_id: String },
    SubmitInput { text: String },
    Shell { cmd: String },
    SendKey(KeyWire),
    Paste { text: String },
    /// Drop a single STAGED attachment (a GUI chip) by its `[Image #N]` marker number,
    /// unstaging it from the foreground session's `pending_attachments` before submit.
    /// The TUI has no equivalent request (it drops attachments by deleting the marker in
    /// the composer); this is the GUI's explicit remove path.
    RemoveAttachment { marker_n: usize },
    /// Fuzzy-search the foreground session's workspace file index (the `@`-palette engine)
    /// for the GUI omnisearch overlay. Read-only: the daemon runs `DirCache::search` and
    /// replies with a one-shot [`DaemonEvent::FileSearchResults`] WITHOUT attaching or
    /// mutating state. `limit` caps the result count (defaults to the palette cap).
    FileSearch { query: String, limit: Option<usize> },
    ApproveTool { approve: bool },
    /// Answer a paused `plan_ready` approval. `decision` is one of `"approve"`,
    /// `"compact"` (approve + compact history to the plan), or `"deny"` (keep
    /// discussing). Parity with [`ApproveTool`] for a direct (non-key) path; the
    /// TUI client normally forwards the y/a/n keystroke as `SendKey` instead.
    PlanDecision { decision: String },
    NewSession {
        name: Option<String>,
        working_dir: Option<String>,
    },
    QuitSession { session_id: String },
    QuitDaemon,
    EditorWrapW(usize),
    OpenSessionHub,
    /// Rename the requesting client's FOREGROUND session (the GUI RenameOverlay
    /// submit). Sets the session's display name + persists via
    /// [`crate::model::store::rename_session`] (in-memory `name`/`settings.name` +
    /// the SQLite registry). The TUI has no equivalent request — it renames through
    /// the `/rename` slash-command / the Settings save — so this is the GUI's direct
    /// atomic rename path (fixes the "rename not working" gap where `NewSession.name`
    /// was accept-ignored). An empty/whitespace name is a no-op.
    RenameSession { name: String },

    // ─── GUI config setters (Connector + MCP panels) ─────────────────────────
    // All gui-gated: the TUI drives config through `Mode::Settings`/`Mode::Mcp` and
    // never sends these. Each mutates the daemon's authoritative `AppConfig` (or the
    // foreground session's local model overrides) + persists, reusing the SAME
    // config-layer setters/parsers the TUI editors use. The resulting config change
    // forces a full snapshot, so the GUI host re-derives + re-pushes its `Config`
    // envelope (see `ipc::snapshot::diff`).
    /// Upsert an MCP server. `uuid` is `None` for a brand-new server (the daemon mints
    /// one) or `Some` to edit an existing one by uuid. `args`/`env` are the panel's
    /// single-line STRING forms (space-separated args; `K=V, K2=V2` env); the daemon
    /// parses them into its array/pair forms. `transport` is `"stdio"` or `"http"`.
    SetMcpServer {
        uuid: Option<String>,
        name: String,
        enabled: bool,
        transport: String,
        command: String,
        args: String,
        env: String,
        url: String,
    },
    /// Remove the MCP server with `uuid`, persist, and live-reconnect the manager.
    DeleteMcpServer { uuid: String },
    /// Toggle the `enabled` flag on the MCP server with `uuid` (the list-row switch),
    /// persist, and live-reconnect the manager.
    EnableMcpServer { uuid: String, enabled: bool },
    /// Request live MCP server connection status. Answered with a one-shot
    /// `DaemonEvent::McpStatus` frame (never folded — re-pushed by push_intercept).
    GetMcpStatus { request_id: String },
    /// Upsert a provider (Connector ProviderForm). `uuid` is `None` for a new provider
    /// (minted OpenAI-compatible) or `Some` to edit by uuid (preserving its wire type).
    SetProvider {
        uuid: Option<String>,
        name: String,
        endpoint: String,
        api_key: String,
    },
    /// Remove the provider with `uuid` and persist.
    DeleteProvider { uuid: String },
    /// Upsert a model (Connector ModelForm). `uuid` is `None` for a new model. `roles`
    /// are lowercase role tokens (`"main"`/`"awareness"`/…); `scope` is `"global"`
    /// (persisted to `AppConfig.models`) or `"local"` (the foreground session's
    /// `settings.session_models`). Applied with per-scope role-steal.
    SetModel {
        uuid: Option<String>,
        name: String,
        model_id: String,
        provider_uuid: String,
        route: Option<String>,
        roles: Vec<String>,
        scope: String,
    },
    /// Remove the model with `uuid` from the `scope` catalogue (`"global"`/`"local"`)
    /// and persist.
    DeleteModel { uuid: String, scope: String },
    /// Fetch the live model-id catalogue for the provider with uuid `provider` (a
    /// `GET {endpoint}/models`) to populate the Connector ModelForm's model-id picker.
    /// Read-only + async: the daemon spawns the network GET and replies out-of-band with
    /// a [`DaemonEvent::ModelList`] on a later tick (mirrors the `FileSearch` one-shot).
    ListModels { provider: String },
    /// Fetch the live PROVIDER-ROUTE list for one model (a `GET
    /// {endpoint}/models/{model_id}/endpoints`) to populate the Connector ModelForm's
    /// ROUTE picker with the model's REAL routes (provider name + prompt/completion price
    /// + uptime) instead of the hardcoded demo list. `model_id` is the verbatim
    ///   `author/slug` id. Read-only + async like [`ListModels`]: the daemon gates on the
    ///   provider being an OpenRouter-style routable endpoint (non-OpenRouter → empty), spawns
    ///   the network GET, and replies out-of-band with a [`DaemonEvent::ModelRoutes`] on a
    ///   later tick. gui-gated.
    ListRoutes { provider: String, model_id: String },
    /// Set the active theme (the GUI onboarding theme step + the future Settings gear).
    /// `name` is a [`crate::view::theme::PALETTES`] registry key (an unknown name falls
    /// back to the dark palette at render time). The daemon writes `AppConfig.palette`
    /// (the live theme key), persists via `AppConfig::save`, and the resulting palette
    /// change forces a full snapshot so the GUI host re-derives + re-pushes its `Config`
    /// palette live. gui-gated: the TUI picks the theme in `Mode::Settings`. `theme`/
    /// `accent` are the deprecated legacy fields and are left untouched.
    SetTheme { name: String },
    /// Mint (or reuse) the keyless Koma Free provider + a Main-role "koma free" model in
    /// the GLOBAL config — the GUI onboarding "koma free" choice, the non-key equivalent of
    /// the TUI first-run chooser's `Action::SetupKomaFree`. Idempotent: reuses an existing
    /// `ApiType::KomaFree` provider (and its Main model) rather than duplicating (see
    /// [`crate::service::koma_free::ensure_koma_free_config`]). Persisted like the other
    /// config setters; the resulting config change re-pushes a fresh `Config` (a Main route
    /// now resolves → `firstRun` clears, dismissing the GUI onboarding overlay). Applied on
    /// BOTH the attached-daemon path and the pre-session swapper path, like
    /// `SetProvider`/`SetModel`/`SetTheme`. gui-gated: the TUI drives this via `Mode::Onboard`.
    SetupKomaFree,

    // ─── GUI turn/session controls (Explore sidepanel + composer + picker) ────────
    /// Interrupt the foreground session's in-flight turn (the GUI stop button) — the
    /// non-key equivalent of the TUI's Esc-interrupt. Reuses [`Action::Interrupt`]
    /// daemon-side (abort the stream, commit the partial with `[interrupted]`, halt the
    /// agentic loop + kill running sub-agents). gui-gated: the TUI forwards Esc as a
    /// `SendKey` instead.
    Interrupt,
    /// Resend the foreground session's last user turn (composer Ctrl+R parity) —
    /// the non-key equivalent of the TUI's Ctrl+R. Reuses `Action::Resend`
    /// daemon-side: pops trailing assistant messages and re-streams the last
    /// user turn from scratch. A no-op (busy / no session / nothing to resend)
    /// surfaces via the session's `status` line, exactly like the TUI's Ctrl+R.
    /// gui-gated: the TUI drives this via `SendKey` (Ctrl+R).
    Resend,
    /// Clear every queued mid-turn steer message (the composer's queued-list
    /// clear button) — the non-key equivalent of the TUI's Ctrl+X-with-pending-
    /// steers. Reuses `Action::CancelSteers` daemon-side (clears
    /// `pending_steer` + a status line). A no-op when the queue is already
    /// empty. gui-gated: the TUI drives this via `SendKey` (Ctrl+X).
    CancelSteers,
    /// Kill a single sub-agent of the foreground session by its stable id (the GUI
    /// agent-row kill button). Mirrors the model-callable `task_kill` primitive: abort
    /// the tokio task + flip a still-Running status to Killed (a terminal status is left
    /// untouched). gui-gated.
    KillSubagent { id: usize },
    /// Background a single sub-agent of the foreground session by its stable id (the GUI
    /// agent-row background button). Mirrors the TUI's Ctrl+B-on-selection. Reuses
    /// [`Action::BackgroundSubagent`] daemon-side — the handler re-checks eligibility
    /// itself (must be `Running`, not already detached, and have a `tool_call_id`), so an
    /// ineligible or stale id is a clean no-op.
    BackgroundSubagent { id: usize },
    /// Background EVERY eligible running sub-agent of the foreground session at once (the
    /// GUI's global Ctrl+B). Mirrors the TUI's Ctrl+B-in-composer. Reuses
    /// [`Action::BackgroundAllSubagents`] daemon-side, which is itself a no-op when no
    /// sub-agent is eligible. gui-gated.
    BackgroundAllSubagents,
    /// Kill a single background-bash job of the foreground session by its numeric id (the
    /// GUI bash-row kill button). Reuses [`Action::BashKillJob`] daemon-side (SIGTERM +
    /// flip status→Killed). The GUI addresses the job as `bash-<id>`; only the numeric
    /// `id` crosses here. gui-gated.
    BashKill { id: usize },
    /// Set THIS client's read-only STREAM VIEW — which sub-agent / bash job the GUI is
    /// live-streaming into an Explore stream tab (the non-key equivalent of the TUI's
    /// full-screen sub-agent viewer, generalised to bash). `subagent`/`bash` are the
    /// numeric ids of the currently-viewed sub-agent / bash job; BOTH `None` clears the
    /// view (no stream tab active). Exactly one is ever `Some` in practice (the active
    /// tab), but the shape allows either independently. Stored per-client on the hub
    /// (`stream_subagent`/`stream_bash`); it drives TWO per-client streaming behaviours:
    /// (1) a VIEWED detached sub-agent's per-step content churn is no longer suppressed
    /// (see `ipc::snapshot::diff`), so its transcript streams live; (2) the viewed bash
    /// job's captured OUTPUT TAIL is projected into that client's snapshot (larger tail
    /// than the `/bash` panel). Fire-and-forget per-client state (no always-reply needed);
    /// a view CHANGE forces a one-shot full resync so the fresh content lands immediately.
    /// gui-gated: the TUI drives its sub-agent viewer via the `$` panel + Enter, never this.
    ///
    /// `session` PINS the view to one session by its stable UUID. Sub-agent + bash job ids
    /// are PER-SESSION counters (each `SessionRuntime` starts `next_subagent_id` at 0,
    /// `next_bash_job_id` at 1), so a bare numeric id is ambiguous across a daemon's
    /// sessions — both consumers gate on `session` so viewing agent/job N in one session
    /// never touches the same-numbered agent/job in another. `#[serde(default)]` keeps an
    /// intermediate peer that omits it decoding cleanly (→ `None`, i.e. unpinned).
    SetStreamView {
        subagent: Option<usize>,
        bash: Option<usize>,
        #[serde(default)]
        session: Option<String>,
    },
    /// Set the GLOBAL agent mode (the GUI composer mode selector) to `mode`, one of
    /// `"auto"`/`"normal"`/`"plan"`/`"yolo"`. Routed daemon-side through the SAME
    /// `AppStateRest::set_agent_mode` choke-point the TUI's Shift+Tab / `/mode` funnel
    /// through (so Plan-enter/leave + the plan-boundary system-prompt swap stay correct);
    /// `"yolo"` is honoured ONLY while `yolo_armed` (else a no-op), exactly like `/mode`.
    /// An unknown token is a no-op. The mode change re-projects into the snapshot, so the
    /// GUI reflects it live. gui-gated: the TUI drives the mode via Shift+Tab / `/mode`.
    SetMode { mode: String },
    /// Set (or clear) the foreground session's LOCAL Main-role model override (the GUI
    /// model quick-picker). `model_uuid` is `Some(uuid)` of a GLOBAL `config.models`
    /// entry to CLONE into a session-local Main [`crate::model::app_config::ModelEntry`]
    /// (reusing an existing matching local override instead of duplicating), or `None` to
    /// REMOVE the override (inherit the global Main). Persists to the per-session
    /// `session_models`; the global catalogue is never touched. gui-gated.
    SetSessionMain { model_uuid: Option<String> },
    /// Rewind the foreground session's conversation to JUST BEFORE the message at
    /// `index` (the GUI's hover-edit pencil on a USER chat bubble) — the non-key
    /// equivalent of the TUI's double-Esc `Mode::MessageRewind` + Enter path.
    /// `index` is the vec position into `SessionSnapshot.messages`
    /// (`Conversation::messages()`); it must address a User-role turn (the core
    /// guards a non-user / out-of-range index as a clean no-op). Reuses
    /// [`Action::RewindToMessage`] daemon-side: abort any in-flight turn, truncate
    /// the live conversation + the sqlite archive to before `index`, and REFILL the
    /// composer with that message's text (surfaced to the client via the
    /// projected `GlobalSnapshot.input` / `InputChanged` delta — NOT auto-sent, the
    /// user edits + presses Enter). gui-gated: the TUI drives rewind via double-Esc.
    RewindTo { index: usize },
    /// Compact the foreground session's conversation (the GUI status-footer Compact
    /// action) — the non-key equivalent of the TUI's `/compact`. Reuses
    /// [`crate::app::runtime::commands::compact::handle_compact`] daemon-side with
    /// `preserve_n_override: None` (the session's configured `compaction.preserve_n`),
    /// the SAME entry point `/compact` calls. A no-op (busy / no session) is reported
    /// back via the session's `status` line, exactly like `/compact`. gui-gated: the
    /// TUI drives compaction via the `/compact` slash command.
    Compact,
    /// Fetch the foreground session's GUI-editable prefs (name / workdir / short-send /
    /// sliding-cache / bash-saving / internet-mode) + the global palette, for the GUI
    /// Settings tab. Read-only: the daemon replies with a one-shot
    /// [`DaemonEvent::SettingsValues`] WITHOUT attaching or mutating any state (best-effort
    /// defaults when there is no foreground session, so the tab never hangs). gui-gated: the
    /// TUI drives settings through `Mode::Settings`.
    GetSettings,
    /// Partial-update the foreground session's GUI-editable prefs (the GUI Settings tab's
    /// Session section). Only the `Some` fields are applied, EACH through the SAME per-field
    /// apply logic the TUI settings save uses (`actions::settings::handle_save_settings`):
    /// short-send / sliding-cache / bash-saving are plain field sets, `internet_mode`
    /// (`"simple"`/`"full"`) goes through the shared internet-feedback path, and `workdir` is
    /// normalized (trim + drop empties + cwd fallback) with a dir-cache reindex. The daemon
    /// then persists the session settings and re-pushes a fresh [`DaemonEvent::SettingsValues`]
    /// so the UI reflects reality. gui-gated: the TUI drives these via `Mode::Settings` /
    /// `/internet`.
    SetSessionPrefs {
        short_send: Option<bool>,
        sliding_cache: Option<bool>,
        bash_saving: Option<bool>,
        coding_autosave: Option<bool>,
        internet_mode: Option<String>,
        workdir: Option<Vec<String>>,
    },

    /// GUI composer EFFORT picker opened: derive the `/effort` menu for the
    /// foreground session's current (Main-role) model, reusing
    /// [`crate::app::runtime::commands::effort::effort_menu`] — the SAME
    /// derivation the TUI's `/effort` command uses, including its cold-cache
    /// fetch-arm side effect. ALWAYS replies with a one-shot
    /// [`DaemonEvent::EffortOptions`] (never a bare `Ack`/`Error`) so the picker
    /// never hangs: a cold/mismatched cache replies `state: "loading"` with
    /// empty `options` (the GUI can re-poll), a model with no reasoning control
    /// replies `state: "unsupported"`, and a derived menu replies
    /// `state: "ready"` with `options`/`selected`/`note` populated. gui-gated:
    /// the TUI drives the picker via `Mode::Effort` directly.
    GetEffortOptions,
    /// GUI composer EFFORT picker pick: persist the chosen effort level via the
    /// SAME [`crate::app::runtime::actions::Action::SaveEffort`] the TUI picker's
    /// confirm keystroke runs (`"default"` → empty = model default; no client
    /// rebuild, effort is resolved per-call). The reply is a fresh
    /// [`DaemonEvent::SettingsValues`] push (mirrors [`SetSessionPrefs`]'s
    /// reply-via-re-push framing) so the GUI's effort-picker label updates off
    /// the SAME settings channel it already listens to. gui-gated: the TUI
    /// drives this via `Mode::Effort`'s confirm handler.
    SetEffort { effort: String },

    // ─── GUI /agents dashboard (sub-agent definitions) ───────────────────────
    /// Fetch the merged sub-agent registry (built-in < global < session) + the model /
    /// provider catalogue for the GUI /agents dashboard. Read-only: the daemon replies
    /// with a one-shot [`DaemonEvent::AgentsValues`] WITHOUT attaching or mutating any
    /// state, and ALWAYS (built-in + global only when there is no foreground session) so
    /// the dashboard never hangs. gui-gated: the TUI drives the roster through
    /// `Mode::Agents` + `SendKey`.
    ListAgents,
    /// Upsert one sub-agent definition (the /agents editor's create / save / rename).
    /// `scope` is `"global"` (`~/.koma/agents/`) or `"session"`
    /// (`<session_dir>/agents/`); a `"session"` scope with no foreground session is an
    /// error. `original_name` is the agent's name BEFORE this edit: `Some(x)` with
    /// `x != name` is a RENAME (the old `<scope>/<x>.md` is deleted AFTER the new file
    /// is written, so a save error never orphans the old def), `Some(x)` with `x == name`
    /// (or `None`) is an in-place create / edit. Only the editor-carried fields cross
    /// here; on an EDIT the daemon loads the existing def first and mutates ONLY these,
    /// so non-editor frontmatter (steps / effort / temperature / color) survives the
    /// round-trip, and the legacy `model` / `provider` / `provider_uuid` slots are
    /// cleared (the editor drives `model_uuid` now). Saving over a name whose current
    /// source is a BUILT-IN forces `"session"` scope (a session override — a built-in is
    /// never mutated in place). The daemon persists via the data layer (which re-validates
    /// the name → path-safe), rebuilds the foreground session's system-prompt roster, and
    /// re-pushes a fresh [`DaemonEvent::AgentsValues`] as the reply. gui-gated.
    SetAgent {
        original_name: Option<String>,
        /// Request-sequence number echoed in the reply for stale-reply protection
        /// (GUI agent-save lifecycle). 0 = no correlation.
        #[serde(default)]
        req_seq: u64,
        scope: String,
        name: String,
        description: String,
        conditions: String,
        model_uuid: Option<String>,
        tools: Vec<String>,
        prompt: String,
    },
    /// Delete one file-backed sub-agent definition (`<scope>/<name>.md`) — the /agents
    /// dashboard's delete. `scope` is `"global"` / `"session"`. A BUILT-IN agent has no
    /// file and is NOT deletable: a delete targeting a name whose source is built-in is
    /// rejected with an [`DaemonEvent::Error`]. Deleting a session / global override that
    /// shadowed a built-in simply re-exposes the built-in on the next load. The daemon
    /// rebuilds the foreground session's roster and re-pushes a fresh
    /// [`DaemonEvent::AgentsValues`] as the reply. gui-gated.
    DeleteAgent {
        scope: String,
        name: String,
        /// Request-sequence number echoed in the reply for stale-reply protection.
        /// 0 = no correlation.
        #[serde(default)]
        req_seq: u64,
    },

    // ─── GUI OAuth surface (Codex / Kilo Code / xAI login) ───────────────────
    /// Fetch the current OAuth state (idle): the persisted connections + the available
    /// providers, phase `"idle"`. Read-only + ALWAYS-reply (a one-shot
    /// [`DaemonEvent::OAuthState`]) like [`GetSettings`]/[`ListAgents`]; delivered whether
    /// or not the requesting client is session-attached so the OAuth screen never hangs.
    /// gui-gated: the TUI drives OAuth through `Mode::Settings`/`OnboardProvider`.
    GetOAuthState,
    /// Start an OAuth login flow. `provider` is `"codex"` (browser PKCE loopback),
    /// `"kilocode"` / `"xai"` (device code), or `"codex_paste"` (surface the paste-a-token input).
    /// For the two browser/device flows the daemon reuses the EXISTING
    /// `Action::OAuthStart` machinery (spawns `run_codex_flow`/`run_kilo_flow`, which drain
    /// via `drain_oauth` + persist on success) and streams progress back to THIS client as
    /// a sequence of [`DaemonEvent::OAuthState`] pushes (`starting` → `waiting_url` /
    /// `waiting_code` → `success` / `failed`). `codex_paste` just replies `paste` (the
    /// token then arrives via [`SubmitOAuthPaste`]). Attached-only (the flow runs on the
    /// daemon's runtime). gui-gated.
    StartOAuth { provider: String },
    /// Complete the Codex paste-token flow: build a connection straight from a hand-pasted
    /// raw access token via the EXISTING `Action::OAuthPaste` path (persist + seed cache),
    /// then reply with a `success` [`DaemonEvent::OAuthState`] carrying the fresh
    /// connection list. Attached-only. gui-gated.
    SubmitOAuthPaste { token: String },
    /// Cancel an in-flight OAuth flow via the EXISTING `Action::OAuthCancel` path (abort the
    /// background task + drop its receiver), then reply with an `idle`
    /// [`DaemonEvent::OAuthState`]. Attached-only. gui-gated.
    CancelOAuth,
    /// Delete a persisted OAuth connection by `uuid` via the EXISTING `Action::OAuthDelete`
    /// path (remove from `config.oauth_conns` + persist + evict the token-refresh cache),
    /// then reply with a fresh `idle` [`DaemonEvent::OAuthState`]. Works UN-ATTACHED too (the
    /// connector is reachable pre-session): the GUI host removes + evicts from the on-disk
    /// config and re-pushes host-side. gui-gated.
    DeleteOAuthConn { uuid: String },

    // ─── GUI extension STORE surface (browse / install / uninstall) ──────────
    // The koma.run extension marketplace, wired to koma's install pipeline
    // (`crate::app::ext`). Browse/detail hit the PUBLIC store endpoints (no auth);
    // install needs the KomaRun account bearer + verifies + spawns, so the whole
    // family is DAEMON-owned (install mutates the live MCP/ext managers + config),
    // NOT host-local. Every reply lands out-of-band on a later tick via the hub's
    // per-client seq'd `send_to` (the network fetch is spawned), like `ListModels`.
    /// Browse the store catalogue: `GET https://koma.run/api/v1/extensions` with the
    /// optional `q` (full-text) / `category` filters. PUBLIC (no auth). Read-only + async:
    /// the daemon spawns the GET and replies out-of-band with a [`DaemonEvent::StoreCatalogue`]
    /// (empty items + an `error` string on a network failure — never a hang). gui-gated.
    StoreBrowse { query: Option<String>, category: Option<String> },
    /// Fetch one extension's full detail: `GET .../extensions/{id}`. PUBLIC (no auth).
    /// Read-only + async like [`StoreBrowse`]; replies with a [`DaemonEvent::StoreItemDetail`]
    /// (`detail: None` + an `error` on failure). gui-gated.
    StoreDetail { id: String },
    /// Install `id` (optionally pinning `version`, else latest). The install action: detect
    /// the `<os>-<arch>` platform, resolve the KomaRun account bearer (via
    /// [`crate::service::oauth::manager::fresh_key`]), `GET .../extensions/{id}/download`
    /// following the 302 → signed URI, verify the artifact's sha256 + Ed25519 signature, then
    /// unpack + register + (daemon-kind) spawn it. Replies with a [`DaemonEvent::ExtensionOpResult`]
    /// then a fresh [`DaemonEvent::InstalledExtensions`]. gui-gated.
    InstallExtension { id: String, version: Option<String> },
    /// Uninstall `id`: purge its contributions, stop its process, remove its on-disk dir +
    /// registry entry. Replies with a [`DaemonEvent::ExtensionOpResult`] then a fresh
    /// [`DaemonEvent::InstalledExtensions`]. gui-gated.
    UninstallExtension { id: String },
    /// FIRE-AND-FORGET cross-daemon in-memory unload of an extension — the uninstall
    /// FAN-OUT. The one uninstalling side (a session-daemon OR the detached GUI host) sends
    /// this to EVERY OTHER live session-daemon's keyed socket (over the blocking management
    /// codec, like [`SpawnAgent`]/`Status` — never Attached, never streamed a snapshot) so
    /// each drops the just-removed extension's LIVE footprint: its contributed MCP tools,
    /// running child process, ext-agent containment registry, published context blob, and
    /// buffered chat prompts. It touches NO config/disk (the uninstalling side already
    /// persisted that removal); this is purely the in-memory half other daemons can't learn
    /// about until their next boot. The receiver Acks, but the sender NEVER reads it
    /// (fire-and-forget); a daemon too old to know the verb error-replies or drops the
    /// connection, which the sender ignores (additive variant, like the MCP `Fingerprint`
    /// probe). NOT gui-gated — it is daemon-internal and never sent by a GUI client.
    UnloadExtension { id: String },
    /// Fetch the locally-installed extension registry (read-only): replies with a one-shot
    /// [`DaemonEvent::InstalledExtensions`]. gui-gated.
    ListInstalledExtensions,

    // ─── GUI extension PANEL bridge (W8) ─────────────────────────────────────
    /// A GUI extension PANEL's request to its backing extension daemon. The panel iframe
    /// (`koma://extension/<ext_id>/…`) posts it; the host forwards it here through the attached
    /// daemon, which AUTO-STARTS the (daemon-kind, enabled) extension if it is not already
    /// running — a panel being open implies user intent — then `invoke`s its `panel.msg` method
    /// with `{ panelId, payload }` and answers OUT-OF-BAND with a [`DaemonEvent::ExtPanelReply`]
    /// correlated by `req_id`. `panel_id` names the contributing panel; `payload` is the opaque
    /// request body the extension defines. DAEMON-owned (the daemon holds the ext managers), so
    /// attached-only — un-attached the GUI replies locally (W9). Neither `ensure_started` nor
    /// the invoke runs on the event-loop thread (both block on a sync→async bridge); the handler
    /// offloads them to `spawn_blocking`. gui-gated.
    ExtPanelMsg {
        ext_id: String,
        panel_id: String,
        req_id: Option<String>,
        payload: serde_json::Value,
    },
}

// ─── daemon -> client ────────────────────────────────────────────────────────

/// The daemon -> client envelope. Carries a monotonic `seq`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct DaemonFrame {
    pub seq: u64,
    pub event: DaemonEvent,
}

/// What a [`DaemonFrame`] carries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub enum DaemonEvent {
    /// Build-skew handshake (task #142): sent VERY FIRST on attach.
    Hello { version: String },
    /// A full state projection — sent on attach and on resync. Boxed.
    Snapshot(Box<StateSnapshot>),
    /// An incremental update folded onto the existing shadow.
    Delta(StateDelta),
    /// Acknowledgement of a request that produces no other reply.
    Ack,
    /// A request failed; the `String` is a human-readable reason.
    Error(String),
    /// One-shot: the controller asked for the `/select` transcript dump.
    EnterSelect,
    /// One-shot: signal the foreground client to open its LOCAL daemon swapper
    /// (the `/resume` picker). Mirrors [`EnterSelect`] — a daemon-side `/resume`
    /// (or `OpenSessionHub`) emits this to the requesting client INSTEAD of building
    /// a daemon-side `Mode::SessionHub`, so the daemon never changes its own mode.
    /// Handled entirely client-side in `render_loop` (it detaches + runs the swapper
    /// standalone); the shadow treats it as a non-visual no-op.
    OpenSwapper,
    /// One-shot: signal the foreground client to spawn + attach a BRAND-NEW
    /// session-daemon (the `/new` hand-off). Mirrors [`OpenSwapper`] — a daemon-side
    /// `/new` sets `new_pending` and the hub emits this to the controlling client INSTEAD
    /// of creating a session itself (a daemon owns exactly ONE session). `kill` is the
    /// `/new kill` flag: `true` tears the CURRENT session-daemon down (`QuitDaemon`) before
    /// attaching the new one; `false` leaves it cooking (resumable via the swapper). Handled
    /// entirely client-side in `render_loop`/`client_run` (it detaches — or kills, then
    /// detaches — and attaches a freshly minted id); the shadow treats it as a non-visual
    /// no-op.
    NewSession { kill: bool },
    /// One-shot: instruct the CONTROLLING client to ATTACH to ANOTHER session's daemon (the
    /// extension `sessions.switch` hand-off to a session this daemon does not own, W7). A
    /// broker `sessions.switch` whose target uuid is NOT a live session in THIS daemon sets
    /// `state.rest.ext_switch_pending = Some(uuid)`; the hub drains it next tick and
    /// broadcasts this to attached clients — the EXACT mirror of `new_pending` →
    /// [`NewSession`] / `resume_pending` → [`OpenSwapper`], leaving the daemon's own mode
    /// untouched. The client is expected to detach and re-attach the named session's daemon
    /// (via its keyed socket). Payload-free beyond the target `session_id`. The TUI shadow
    /// treats it as a non-visual no-op (it MAY ignore the hand-off); GUI wiring lands in a
    /// later wave. Zero attached clients → structural no-op.
    AttachSession { session_id: String },
    /// One-shot reply to a [`ClientRequest::Status`] discovery probe: this daemon's
    /// single owned session's metadata. Sent WITHOUT attaching the client or streaming
    /// any snapshot — the connection is expected to close right after.
    Status(SessionStatus),
    /// One-shot reply to a [`ClientRequest::FileSearch`]: the resolved workspace-file
    /// hits for `query` (echoed so the GUI can drop a stale/out-of-order reply). Sent
    /// WITHOUT attaching or snapshotting — a metadata reply like [`Status`].
    FileSearchResults { query: String, items: Vec<FileSearchItem> },
    /// One-shot reply to a [`ClientRequest::ListModels`]: the live model-id catalogue
    /// (`GET {endpoint}/models`) for the provider uuid echoed in `provider` (so the GUI
    /// can drop a stale/out-of-order reply). Delivered on a LATER tick than the request
    /// (the fetch is async) via the hub's per-client seq'd `send_to`, so it never
    /// breaks the seq stream; an empty `models` marks a failed/empty fetch. The GUI host
    /// re-pushes it as a `ModelList` envelope; the TUI shadow treats it as a no-op.
    ModelList { provider: String, models: Vec<String> },
    /// One-shot reply to a [`ClientRequest::ListRoutes`]: the model's live provider-route
    /// list (`GET {endpoint}/models/{model_id}/endpoints`), each route flattened to the
    /// GUI subset ([`ModelEndpointWire`]: provider name + prompt/completion price + uptime).
    /// `provider`/`model_id` are echoed so the ModelForm can drop a stale/out-of-order reply
    /// (a provider/model-id change refetches). Delivered on a LATER tick than the request
    /// (the fetch is async) via the hub's per-client seq'd `send_to`; an EMPTY `routes`
    /// marks a non-OpenRouter provider or a failed/empty fetch (the form shows only "Auto").
    /// The GUI host re-pushes it as a `RouteList` envelope; the TUI shadow treats it as a
    /// no-op.
    ModelRoutes {
        provider: String,
        model_id: String,
        routes: Vec<ModelEndpointWire>,
    },
    /// One-shot reply to a [`ClientRequest::GetSettings`] (and a re-push after a
    /// [`ClientRequest::SetSessionPrefs`]): the foreground session's GUI-editable prefs +
    /// the global config palette, for the GUI Settings tab. Delivered whether or not the
    /// requesting client is session-attached (like [`ModelList`], via the hub's per-client
    /// seq'd `send_to`), and ALWAYS sent — best-effort defaults (`name`/`workdir` empty)
    /// when there is no foreground session — so the Settings tab never hangs. `internet_mode`
    /// is the `"simple"`/`"full"` wire token; `palette` is the active theme registry key. The
    /// GUI host re-pushes it as a `SettingsValues` envelope; the TUI shadow ignores it.
    SettingsValues {
        name: String,
        workdir: Vec<String>,
        short_send: bool,
        sliding_cache: bool,
        bash_saving: bool,
        coding_autosave: bool,
        internet_mode: String,
        palette: String,
        /// The foreground session's stored `/effort` value (`""` = model
        /// default), for the GUI composer's effort-picker label. Mirrors the
        /// TUI's `sess.settings.effort` field verbatim.
        effort: String,
    },
    /// One-shot reply to a [`ClientRequest::GetEffortOptions`]: the derived
    /// `/effort` menu for the foreground session's current model, from
    /// [`crate::app::runtime::commands::effort::effort_menu`]. `state` is
    /// `"loading"` (no options yet — a catalogue fetch was just armed or is
    /// already in flight; `options` empty, `selected` 0), `"unsupported"`
    /// (the model has no reasoning control, or there's no active
    /// session/client; `options` empty, `selected` 0), or `"ready"`
    /// (`options`/`selected` populated exactly like `Mode::Effort`'s
    /// `EffortPickerState`). `note` carries the human-readable reason/hint in
    /// every state (the TUI's status-line text for `"loading"`/`"unsupported"`,
    /// the picker's capability note for `"ready"`). ALWAYS sent — delivered
    /// whether or not the requesting client is session-attached, like
    /// [`DaemonEvent::SettingsValues`] — so the GUI picker never hangs.
    EffortOptions {
        options: Vec<String>,
        selected: usize,
        note: String,
        state: String,
    },
    /// One-shot reply to a [`ClientRequest::ListAgents`] (and the re-push after a
    /// [`ClientRequest::SetAgent`] / [`ClientRequest::DeleteAgent`]): the merged sub-agent
    /// registry + the model / provider catalogue for the GUI /agents dashboard. `agents`
    /// is the FULL roster (built-in + global + session, hidden included, disabled already
    /// dropped by the loader), each entry carrying its `source`
    /// (`"builtin"` / `"global"` / `"session"`), chosen `model_uuid`, tool allow-list, and
    /// system prompt. `catalogue_models` seeds the editor's model picker — the foreground
    /// session's local `session_models` FIRST, then the global `config.models` — and
    /// `catalogue_providers` mirrors `config.providers`. `available_tools` is the
    /// user-selectable tool-name list for the editor's tool picker
    /// ([`crate::tool::agent_selectable_tools`] — every built-in tool minus the internal /
    /// infra ones, in registry order), the SAME source the TUI picker uses. Delivered
    /// whether or not the requesting client is session-attached (like [`SettingsValues`],
    /// via the hub's per-client seq'd `send_to`) and ALWAYS sent (built-in + global only
    /// when there is no foreground session) so the dashboard never hangs. The GUI host
    /// re-pushes it as an `AgentsValues` envelope; the TUI shadow ignores it.
    AgentsValues {
        /// Request-sequence number echoed from [`ClientRequest::SetAgent`] /
        /// [`ClientRequest::DeleteAgent`] for stale-reply protection. 0 = no correlation.
        #[serde(default)]
        req_seq: u64,
        agents: Vec<AgentEntry>,
        catalogue_models: Vec<CatalogueModelSnapshot>,
        catalogue_providers: Vec<CatalogueProviderSnapshot>,
        available_tools: Vec<String>,
    },
    /// One-shot result of a daemon-side SetAgent/DeleteAgent (attached path).
    /// On success the authoritative reply is always a fresh [`AgentsValues`] push,
    /// so this only carries failures. `req_seq` echoes the request sequence for
    /// stale-reply protection; 0 = no correlation.
    AgentOp {
        ok: bool,
        #[serde(default)]
        error: Option<String>,
        #[serde(default)]
        req_seq: u64,
    },
    /// The streaming GUI OAuth surface's authoritative state, for the webview's OAuth
    /// screen. Sent as the one-shot reply to a [`ClientRequest::GetOAuthState`] /
    /// [`ClientRequest::DeleteOAuthConn`] / [`ClientRequest::CancelOAuth`] /
    /// [`ClientRequest::SubmitOAuthPaste`], AND streamed repeatedly as an in-flight
    /// [`ClientRequest::StartOAuth`] progresses (`starting` → `waiting_url` /
    /// `waiting_code` → `success` / `failed`) — the daemon hub delivers each transition to
    /// exactly the initiating client via its per-client seq'd `send_to` (a background flow
    /// task can't advance the seq), so the frames interleave gap-free with the snapshot
    /// stream (mirrors the `ModelList`/`SettingsValues` out-of-band replies).
    ///
    /// `phase` ∈ `"idle"` | `"starting"` | `"waiting_url"` | `"waiting_code"` | `"paste"` |
    /// `"success"` | `"failed"`. `url` is the Codex authorization URL (`waiting_url`);
    /// `user_code` + `verification_url` are the Kilo Code device fields (`waiting_code`);
    /// `error` is the failure reason (`failed`). `conns` is the CURRENT persisted
    /// connection list (TOKENLESS [`OAuthConnWire`] — never an access/refresh/id token) and
    /// `providers` the available-provider catalogue ([`OAuthProviderWire`]); both are
    /// (re)built from the live config + the registry on every push so the webview store
    /// stays authoritative. The GUI host re-pushes it as an `OAuthState` envelope; the TUI
    /// shadow ignores it.
    OAuthState {
        phase: String,
        url: Option<String>,
        user_code: Option<String>,
        verification_url: Option<String>,
        error: Option<String>,
        conns: Vec<OAuthConnWire>,
        providers: Vec<OAuthProviderWire>,
    },
    /// One-shot reply to a [`ClientRequest::StoreBrowse`]: the store catalogue rows for the
    /// GUI Store grid. `error` is `Some` (and `items` empty) on a network/parse failure so
    /// the grid renders an error state rather than hanging. Delivered out-of-band on a later
    /// tick via the hub's per-client seq'd `send_to` (the fetch is async), whether or not the
    /// client is session-attached. The GUI host re-pushes it as a `StoreCatalogue` envelope;
    /// the TUI shadow treats it as a no-op.
    StoreCatalogue {
        items: Vec<StoreItemWire>,
        error: Option<String>,
    },
    /// One-shot reply to a [`ClientRequest::StoreDetail`]: one extension's full detail.
    /// `detail` is `None` (and `error` `Some`) when the fetch failed or the id was unknown.
    /// Same out-of-band delivery as [`StoreCatalogue`].
    StoreItemDetail {
        detail: Box<Option<StoreDetailWire>>,
        error: Option<String>,
    },
    /// The locally-installed extension registry — the reply to
    /// [`ClientRequest::ListInstalledExtensions`] AND the re-push after a successful
    /// [`ClientRequest::InstallExtension`] / [`ClientRequest::UninstallExtension`]. The GUI
    /// host re-pushes it as an `InstalledExtensions` envelope; the TUI shadow ignores it.
    InstalledExtensions { items: Vec<InstalledExtWire> },
    /// One-shot result of an install/uninstall op. On success the authoritative registry
    /// reply is the following [`InstalledExtensions`] push; this carries the ok/error status
    /// (echoing `id` so the GUI can clear that card's pending spinner). `ok: false` +
    /// `error` surfaces the failure (e.g. "sign in to koma.run to install", an entitlement
    /// error, or a signature-verification hard stop).
    ExtensionOpResult {
        id: String,
        ok: bool,
        error: Option<String>,
    },
    /// Out-of-band reply to a [`ClientRequest::ExtPanelMsg`] (W8 panel bridge): the extension's
    /// `panel.msg` invoke outcome, delivered to the REQUESTING client via the hub's per-client
    /// seq'd `send_to` (the auto-start + invoke run off-loop on `spawn_blocking`, so they can't
    /// advance the seq themselves — the same out-of-band pattern as [`StoreCatalogue`]). `req_id`
    /// is echoed from the request so the GUI panel can correlate the reply to its pending call;
    /// `ok`/`payload`/`error` carry the result (an unavailable / disabled / oneshot extension, a
    /// failed auto-start, or a timed-out/failed invoke is `ok:false` + `error`). The GUI host
    /// re-pushes it as an `ExtPanelReply` envelope; the TUI shadow treats it as a no-op.
    ExtPanelReply {
        ext_id: String,
        panel_id: String,
        req_id: Option<String>,
        ok: bool,
        payload: Option<serde_json::Value>,
        error: Option<String>,
    },
    /// Unsolicited daemon→panel push (W8 panel bridge): an extension daemon sent a `panel.push`
    /// notify, broadcast to EVERY attached client (panel pushes are NOT request-correlated — no
    /// initiating client) by the daemon hub's `drain_ext_panel_pushes`.
    /// `panel_id` names the target panel; `payload` is the extension-defined body. The GUI host
    /// re-pushes it as an `ExtPanelPush` envelope; the TUI shadow treats it as a no-op.
    ExtPanelPush {
        ext_id: String,
        panel_id: String,
        payload: serde_json::Value,
    },
    /// One-shot reply to a [`ClientRequest::GetMcpStatus`]: per-server connection state
    /// (tool counts + errors) plus an optional top-level availability error. The GUI
    /// host re-pushes this as a `McpStatus` envelope via `push_intercept`.
    McpStatus {
        /// Echoed from the request for stale-reply protection.
        request_id: String,
        /// Per-server status rows keyed by server uuid. Only connected or errored
        /// servers appear; disabled and still-connecting servers are absent.
        servers: Vec<McpStatusServer>,
        /// Top-level error when the MCP manager is unavailable (no session, proxy
        /// transport failure). `None` when the status was retrieved successfully.
        #[serde(skip_serializing_if = "Option::is_none")]
        global_error: Option<String>,
    },
}

/// One row in a [`DaemonEvent::McpStatus`] reply.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpStatusServer {
    /// Server config uuid.
    pub id: String,
    /// True when the server has a live connection.
    pub connected: bool,
    /// Discovered tool count (0 when connected with no tools, or when not connected).
    pub tool_count: usize,
    /// Human-readable error string when the server failed to connect.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// ─── mode discriminant ───────────────────────────────────────────────────────

/// A PURE-DATA projection of the live [`crate::app::mode::Mode`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub enum ModeSnapshot {
    Onboard(Box<OnboardSnapshot>),
    OnboardProvider(Box<OnboardProviderSnapshot>),
    KeyInput(KeyInputSnapshot),
    SessionPicker(PickerSnapshot),
    SessionHub(SessionHubSnapshot),
    Chat,
    Loading(LoadingSnapshot),
    Settings(Box<SettingsSnapshot>),
    Agents(Box<AgentsSnapshot>),
    Mcp(Box<McpSnapshot>),
    Extensions(Box<ExtensionsSnapshot>),
    ExtScreen(Box<ExtScreenSnapshot>),
    ExtStore(Box<ExtStoreSnapshot>),
    Security(Box<SecuritySnapshot>),
    Bash(Box<BashSnapshot>),
    Todo(Box<TodoSnapshot>),
    Help(Box<HelpSnapshot>),
    Effort(EffortSnapshot),
    Usage(Box<UsageSnapshot>),
    MessageRewind(RewindSnapshot),
    QuitConfirm { working: usize, total: usize, selected: usize },
}

// ─── incremental deltas ──────────────────────────────────────────────────────

/// An incremental state update the daemon emits between full snapshots.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub enum StateDelta {
    TokenAppended { session_id: String, text: String },
    ReasoningAppended { session_id: String, text: String },
    StatusChanged {
        session_id: Option<String>,
        text: String,
    },
    InputChanged { text: String, cursor: usize },
    ScrollChanged { scroll: u16, follow: bool },
    SessionStatusChanged {
        session_id: String,
        working: bool,
        finished_unseen: bool,
    },
    ForegroundChanged { session_id: String },
    SessionAdded(Box<SessionSnapshot>),
    Toast { kind: String, text: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The cross-daemon spawn request (W7 `sessions.spawn_into`) survives a
    /// serde round-trip intact — it crosses the unix socket between two
    /// session-daemons, so its wire shape must be stable (all four fields,
    /// including the `Option` absences).
    #[test]
    fn spawn_agent_serde_roundtrip() {
        let full = ClientRequest::SpawnAgent {
            agent: Some("researcher".into()),
            task: "summarise the diff".into(),
            model: Some("gpt-5".into()),
            effort: Some("high".into()),
        };
        let bytes = serde_json::to_vec(&full).expect("serialise SpawnAgent");
        let back: ClientRequest = serde_json::from_slice(&bytes).expect("deserialise SpawnAgent");
        assert_eq!(back, full);

        // Optional fields absent (the common `sessions.spawn_into { session, task }` shape).
        let minimal = ClientRequest::SpawnAgent {
            agent: None,
            task: "do the thing".into(),
            model: None,
            effort: None,
        };
        let back2: ClientRequest =
            serde_json::from_slice(&serde_json::to_vec(&minimal).unwrap()).unwrap();
        assert_eq!(back2, minimal);
    }

    /// The attach-hand-off signal (W7 `sessions.switch` to a non-local session)
    /// round-trips — it is broadcast to attached clients, so its wire shape must
    /// hold.
    #[test]
    fn attach_session_serde_roundtrip() {
        let ev = DaemonEvent::AttachSession {
            session_id: "abc-123".into(),
        };
        let back: DaemonEvent =
            serde_json::from_slice(&serde_json::to_vec(&ev).unwrap()).unwrap();
        assert_eq!(back, ev);
    }

    /// The panel→daemon request (W8 panel bridge) round-trips intact, INCLUDING the
    /// `req_id` present/absent forms and an arbitrary JSON `payload` — it crosses the
    /// unix socket from the GUI host to the daemon, so its wire shape must be stable.
    #[test]
    fn ext_panel_msg_serde_roundtrip() {
        let with_id = ClientRequest::ExtPanelMsg {
            ext_id: "run.koma.example".into(),
            panel_id: "sidebar".into(),
            req_id: Some("r-7".into()),
            payload: serde_json::json!({ "action": "refresh", "n": 3 }),
        };
        let back: ClientRequest =
            serde_json::from_slice(&serde_json::to_vec(&with_id).unwrap()).unwrap();
        assert_eq!(back, with_id);

        // Fire-and-forget shape (no correlation id).
        let no_id = ClientRequest::ExtPanelMsg {
            ext_id: "run.koma.example".into(),
            panel_id: "sidebar".into(),
            req_id: None,
            payload: serde_json::Value::Null,
        };
        let back2: ClientRequest =
            serde_json::from_slice(&serde_json::to_vec(&no_id).unwrap()).unwrap();
        assert_eq!(back2, no_id);
    }

    /// The panel-reply event (W8) round-trips: both the ok+payload and the
    /// error (`ok:false`, no payload) shapes cross the wire to the requesting client.
    #[test]
    fn ext_panel_reply_serde_roundtrip() {
        let ok = DaemonEvent::ExtPanelReply {
            ext_id: "run.koma.example".into(),
            panel_id: "sidebar".into(),
            req_id: Some("r-7".into()),
            ok: true,
            payload: Some(serde_json::json!({ "rows": [1, 2, 3] })),
            error: None,
        };
        let back: DaemonEvent =
            serde_json::from_slice(&serde_json::to_vec(&ok).unwrap()).unwrap();
        assert_eq!(back, ok);

        let err = DaemonEvent::ExtPanelReply {
            ext_id: "run.koma.example".into(),
            panel_id: "sidebar".into(),
            req_id: None,
            ok: false,
            payload: None,
            error: Some("extension not available".into()),
        };
        let back2: DaemonEvent =
            serde_json::from_slice(&serde_json::to_vec(&err).unwrap()).unwrap();
        assert_eq!(back2, err);
    }

    /// The unsolicited daemon→panel push (W8) round-trips — it is broadcast to every
    /// attached client, so its wire shape must hold.
    #[test]
    fn ext_panel_push_serde_roundtrip() {
        let ev = DaemonEvent::ExtPanelPush {
            ext_id: "run.koma.example".into(),
            panel_id: "sidebar".into(),
            payload: serde_json::json!({ "tick": 42 }),
        };
        let back: DaemonEvent =
            serde_json::from_slice(&serde_json::to_vec(&ev).unwrap()).unwrap();
        assert_eq!(back, ev);
    }
}
