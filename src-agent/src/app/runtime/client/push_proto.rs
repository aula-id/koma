//! [`PushEnvelope`] + the six one-shot `push_*` emit helpers for the GUI
//! push-envelope bridge — the Rust half of the `window.__komaClient.push` JSON
//! contract (`#[serde(tag = "k")]` names each envelope, matching the JS `push`
//! dispatcher's `k` switch EXACTLY). Split out of `render.rs` originally (pure
//! code motion, no behaviour change), then split AGAIN into `push_rows.rs`
//! (same reason): every `Push*` ROW/DTO struct `PushEnvelope`'s variants carry
//! now lives there, re-exported below so importers keep using
//! `super::push_proto::PushX` unchanged.
//!
//! `PushEnvelope` stays `pub(super)` (struct-crossing-a-sibling-module reach,
//! same as every row struct). `emit` stays in `render.rs` (its callers span
//! both this file and `project.rs`), referenced here as `super::render::emit`.

pub(super) use super::push_rows::{
    PushAttachment, PushBashJob, PushCooking, PushFileChange, PushHistory, PushMcpServer,
    PushModel, PushMsg, PushPalette, PushPaletteInfo, PushPendingCall, PushPlanTodo, PushProvider,
    PushRoute, PushSubAgent, PushToolCall, PushUsageDay, PushUsageModel,
};


/// The daemon->JS envelope, tagged on `k`. One variant per bridge message; every
/// field name matches the contract verbatim (camelCase where the contract uses it).
#[derive(serde::Serialize)]
#[serde(tag = "k")]
pub(super) enum PushEnvelope {
    /// Structural / commit tick (the catch-all): the full committed transcript +
    /// title + palette for `session`. `state` is always `"attached"`.
    Snapshot {
        session: String,
        state: &'static str,
        messages: Vec<PushMsg>,
        title: String,
        palette: PushPalette,
        /// Foreground session's sub-agents (list + status). Authoritative full array —
        /// React REPLACES on each Snapshot, never accumulates.
        subagents: Vec<PushSubAgent>,
        /// Foreground session's background-bash jobs (list + status). Authoritative
        /// full array — React REPLACES on each Snapshot, never accumulates.
        bash: Vec<PushBashJob>,
        /// Foreground session's cumulative file-change log (#24). Authoritative full
        /// array — React REPLACES on each Snapshot. Empty when nothing was touched.
        #[serde(rename = "fileChanges")]
        file_changes: Vec<PushFileChange>,
        /// Foreground session's Plan-mode todo checklist (Explore "PLAN" section).
        /// Authoritative full array — React REPLACES on each Snapshot; empty when
        /// not in Plan mode or no plan is in progress (the section hides/dims).
        #[serde(rename = "planTodos")]
        plan_todos: Vec<PushPlanTodo>,
        /// Foreground session's STAGED composer attachments (chips). Authoritative full
        /// array — React REPLACES on each Snapshot; empty once the message is sent.
        attachments: Vec<PushAttachment>,
        /// The current GLOBAL agent mode label (`"auto"`/`"normal"`/`"plan"`/`"yolo"`),
        /// decoded from the snapshot into the shadow. Rides the Snapshot (folded into its
        /// fingerprint) so the composer mode selector shows + reflects the live mode; a
        /// `SetMode` round-trip flips this on the next snapshot.
        mode: String,
        /// Foreground session's QUEUED mid-turn steer previews (koma's `pending_steer`).
        /// Authoritative full array — React REPLACES on each Snapshot; the composer
        /// renders it as the "Queued N/5" preview list while a turn is in flight.
        /// Folded into the fingerprint so queuing/consuming a steer re-emits the Snapshot.
        #[serde(rename = "pendingSteer")]
        pending_steer: Vec<String>,
        /// Foreground session's tool-approval GATE (wave-7): `true` when a risky/classifier
        /// call OR a `plan_ready` plan digest has PARKED and the daemon is blocked waiting on
        /// a decision the GUI must surface. React raises the approval overlay when set; a
        /// `GuiReq::ApproveTool` (tool card) or `GuiReq::PlanDecision` (plan card) answers it.
        /// Folded into the fingerprint so a park/resume re-emits the Snapshot on its own.
        #[serde(rename = "awaitingApproval")]
        awaiting_approval: bool,
        /// The classifier's "why" for a paused risky call — `None` for a `plan_ready` pause
        /// or a non-classifier gate. Shown as the reason line on the approval card.
        #[serde(rename = "approvalReason")]
        approval_reason: Option<String>,
        /// The actual paused call (name + args); `Some` only while `awaitingApproval`. React
        /// branches on `pendingCall.name == "plan_ready"` (plan controls) vs any other name
        /// (tool approve/deny card).
        #[serde(rename = "pendingCall")]
        pending_call: Option<PushPendingCall>,
    },
    /// Swap-START signal: the host is about to tear down the current attach and connect
    /// a different (or freshly minted) session. `to` is the target session id/uuid — the
    /// authoritative identifier React maps to a friendly hub label (an unknown id, e.g. a
    /// brand-new session, has no row yet, so the overlay falls back to a generic label).
    /// Pushed the instant a `Select`/`New` is acted on, BEFORE teardown, so React can
    /// raise a deterministic full-screen loader across the (uninterruptible, possibly
    /// multi-second build-skew) attach gap during which NOTHING else is pushed. Cleared
    /// naturally by the next `Snapshot { state:"attached" }` — whose `session` equals `to`
    /// on a resolved swap.
    Switching { to: String },
    /// The FULL live streaming buffer (React REPLACES the live bubble). Emitted every
    /// frame the buffer changes; an empty `text` clears the bubble on commit.
    StreamMsg { session: String, text: String },
    /// The FULL live reasoning buffer (React REPLACES). Empty `text` clears it.
    Reasoning { session: String, text: String },
    /// Working flag + optional toast. React animates the spinner locally; the host
    /// only says whether the session is working and what toast (if any) to show.
    /// `toastKind` carries the toast SEVERITY (`"error"` / `"info"`) so React can colour
    /// a safeguard/harness block red vs an informational notice neutrally — the daemon's
    /// `ToastKind` (the third field of `fg.toast`) is otherwise dropped on the wire.
    Status {
        session: String,
        working: bool,
        toast: Option<String>,
        #[serde(rename = "toastKind")]
        toast_kind: Option<&'static str>,
        /// Foreground session's cumulative token/cost counters (mirrors the daemon's
        /// `SessionRuntime` totals — see `client_shadow/session.rs`'s rehydration),
        /// for the GUI status footer. `tokensIn` is the CURRENT context size (not a
        /// running sum — mirrors `SessionRuntime::tokens_in`'s own semantics);
        /// `tokensOut`/`tokensCached`/`cost` accumulate across the session.
        #[serde(rename = "tokensIn")]
        tokens_in: u64,
        #[serde(rename = "tokensCached")]
        tokens_cached: u64,
        #[serde(rename = "tokensOut")]
        tokens_out: u64,
        cost: f64,
        /// The EFFECTIVE agent mode label (`"auto"`/`"normal"`/`"plan"`/`"yolo"`) —
        /// identical source to the Snapshot's `mode` (`shadow.rest.agent_mode`), so
        /// e.g. a model-driven `plan_enter` while the selector reads "auto" still
        /// reports `"plan"` here the instant the round-trip lands (Status re-emits
        /// independently of the Snapshot fingerprint). Rides the status footer so it
        /// updates even on ticks where the transcript itself doesn't change.
        mode: String,
    },
    /// The detached session swapper: the `[+ new session]` row + live cooking rows +
    /// on-disk history. `state` is always `"swapper"`.
    Hub {
        state: &'static str,
        cooking: Vec<PushCooking>,
        history: Vec<PushHistory>,
    },
    /// One-shot omnisearch results for `query` — the GUI overlay REPLACES its list with
    /// `items` (each `{ path, label }`; an empty `path` marks a non-attachable dir row).
    /// Pushed out-of-band (not fingerprinted) whenever the daemon answers a `FileSearch`;
    /// `query` is echoed so the overlay can drop a stale/out-of-order reply.
    SearchResults {
        query: String,
        items: Vec<crate::ipc::proto::FileSearchItem>,
    },
    /// The authoritative GLOBAL config catalogue for the Connector + MCP panels. React
    /// REPLACES its config slices on each push. Emitted whenever the projected config
    /// changes (a full snapshot carries it) and re-emitted on `Ready` (page reload) so a
    /// fresh webview always has the current catalogue. `models` folds the global scope
    /// and the foreground session's local-override scope into one list, each row tagged
    /// with its `scope`.
    Config {
        providers: Vec<PushProvider>,
        models: Vec<PushModel>,
        mcp: Vec<PushMcpServer>,
        /// The active palette (theme), so the EMPTY/swapper state — which never receives a
        /// `Snapshot` (the only other palette carrier) — can repaint to `config.json`'s
        /// theme instead of the hardcoded dark default. Rides Config because Config is
        /// pushed in BOTH host states (swapper via `push_swapper_config`, attached via
        /// `push_config`), so the theme is always available.
        palette: PushPalette,
        /// The uuid of the foreground session's LOCAL Main-role model override, or `null`
        /// when there is none (the session inherits the global Main). The model
        /// quick-picker uses this as its current selection: `null` = the `(inherit)` row,
        /// else the matching local-override option. `None` is serialised as JSON `null`.
        #[serde(rename = "sessionMainUuid")]
        session_main_uuid: Option<String>,
        /// The available theme registry names (`view::theme::PALETTES` keys), for the GUI
        /// onboarding theme step + the future Settings gear picker. Static + identical in
        /// both host states, but rides `Config` so the picker always has the list without a
        /// dedicated envelope. React renders one option per name; a pick round-trips as
        /// `SetTheme { name }`.
        themes: Vec<&'static str>,
        /// The ACTIVE palette (theme) registry key (`config.palette`), so the GUI can
        /// highlight the current card in the Settings Appearance grid + the onboarding
        /// theme picker. Re-pushed on every theme change (Config rides the palette diff), so
        /// the highlight tracks live. Serialised as `theme` to match the React store's
        /// `ConfigSlice.theme`.
        theme: String,
        /// The FULL palette catalogue with resolved colours (one [`PushPaletteInfo`] per
        /// `themes` entry, same order), for the GUI Settings tab's Appearance grid — each a
        /// name + its 11 role colours as `#rrggbb`. Kept ALONGSIDE `themes` (names only) so
        /// the onboarding theme step stays backward-compatible; the Settings grid consumes
        /// this richer list. Static (theme registry is compile-time), but rides `Config` so
        /// the picker always has it. A pick round-trips as `SetTheme { name }`.
        palettes: Vec<PushPaletteInfo>,
        /// FIRST-RUN flag (wave-3+4 A/B): `true` when no usable Main route is configured
        /// (no providers, or no global/local Main-role model bound to a known provider) —
        /// i.e. an empty/first-run `~/.koma/config.json`. React shows the full-screen
        /// onboarding flow (theme → connection) instead of the normal start screen when
        /// this is set; it clears the instant a provider + Main model land.
        ///
        /// Serialised as `firstRun` to match the React store's authoritative Config
        /// envelope contract (`ConfigSlice.firstRun`) — same boolean semantics
        /// (`true` = show onboarding).
        #[serde(rename = "firstRun")]
        needs_onboarding: bool,
    },
    /// One-shot live model-id catalogue for `provider` (uuid), answering a
    /// `ListModels` — the Connector ModelForm REPLACES its model-id picker options with
    /// `models`. Pushed out-of-band (not fingerprinted) whenever the daemon answers a
    /// `ListModels`; `provider` is echoed so the form can drop a stale/out-of-order reply.
    ModelList { provider: String, models: Vec<String> },
    /// One-shot live provider-route list for `modelId` under `provider` (uuid), answering a
    /// `ListRoutes` — the Connector ModelForm REPLACES its ROUTE picker options with the
    /// model's REAL routes (`routes`, each a `PushRoute`) instead of the hardcoded demo
    /// list. Pushed out-of-band (not fingerprinted) whenever the daemon answers a
    /// `ListRoutes`; `provider`/`modelId` are echoed so the form can drop a stale reply (a
    /// provider/model-id change refetches). An EMPTY `routes` means a non-OpenRouter
    /// provider or a failed fetch — the form shows only its synthetic "Auto" default.
    #[serde(rename_all = "camelCase")]
    RouteList {
        provider: String,
        model_id: String,
        routes: Vec<PushRoute>,
    },
    /// One-shot host-computed FILE DIFF answering a `FileDiff` request from the Explore
    /// "FILE CHANGED" panel (opening a Monaco diff tab): `original` is `git show
    /// HEAD:<path>` (empty for a new/untracked file — a valid all-added diff);
    /// `modified` is the current on-disk contents (empty when the file was deleted).
    /// `error` is set (both strings then empty) when the diff couldn't be computed at
    /// all (no git repository, or either side over the size cap); `binary` is set
    /// (both strings then empty, no `error`) when either side isn't valid UTF-8 text.
    /// Computed ENTIRELY host-side (see `compute_file_diff` — never forwarded to the
    /// daemon), so this is pushed the SAME way regardless of attach state, and — like
    /// `ModelList`/`RouteList` — is ALWAYS a reply so the diff tab never hangs.
    /// `origin` says where the ORIGINAL side came from: `"git"` (`git show HEAD:`) or
    /// `"baseline"` (the session's "virtual git" first-touch pre-image — non-git
    /// directories); the GUI shows a dim "session baseline" badge for the latter.
    FileDiff {
        path: String,
        original: String,
        modified: String,
        error: Option<String>,
        binary: bool,
        origin: String,
    },
    /// One-shot host-computed LAST-7-DAYS usage preview answering a `UsagePreview`
    /// request from the activity-bar Usage panel: aggregate totals, a 7-entry daily cost
    /// series (oldest first, today last — zero-filled for days with no ledger rows), and
    /// the top 3 models by spend in the window. Computed ENTIRELY host-side straight off
    /// the global `~/.koma/usage.sqlite` ledger (see `compute_usage_preview` — never
    /// forwarded to the daemon), so this is pushed the SAME way regardless of attach
    /// state, and — like `FileDiff` — is ALWAYS a reply so the panel never hangs loading.
    /// `scope` echoes the request's `"all"`/`"session"` token, and `session_id`
    /// (serialised `sessionId`) echoes the session uuid ACTUALLY queried (`None` for an
    /// "all" scope) — together they let React drop a reply that no longer matches
    /// what's currently selected/attached: a rapid all/session toggle racing an
    /// in-flight request (scope mismatch), OR the foreground session switching mid-flight
    /// while "session" scope stayed selected (session id mismatch) — the latter is what
    /// would otherwise render session A's numbers under session B's attach.
    #[serde(rename_all = "camelCase")]
    UsagePreview {
        cost: f64,
        tokens_in: u64,
        tokens_cached: u64,
        tokens_out: u64,
        calls: u64,
        days: Vec<PushUsageDay>,
        top_models: Vec<PushUsageModel>,
        scope: String,
        session_id: Option<String>,
    },
    /// One-shot reply to a `GetSettings` (and the re-push after a `SetSessionPrefs`),
    /// carrying the foreground session's GUI-editable prefs + the active palette for the
    /// GUI Settings tab's Session section. Pushed out-of-band (not fingerprinted) whenever
    /// the daemon answers a `GetSettings` — or, un-attached, straight from the swapper's
    /// global-config fallback. `internetMode` is `"simple"`/`"full"`. ALWAYS a reply so the
    /// tab's loading state can never hang.
    #[serde(rename_all = "camelCase")]
    SettingsValues {
        name: String,
        workdir: Vec<String>,
        short_send: bool,
        sliding_cache: bool,
        bash_saving: bool,
        internet_mode: String,
        palette: String,
        /// The foreground session's stored `/effort` value (`""` = model
        /// default), for the composer's effort-picker trigger-pill label.
        effort: String,
    },
    /// One-shot reply to a `GetEffortOptions`: the derived `/effort` menu for the
    /// foreground session's current model. `state` is `"loading"` (a catalogue
    /// fetch was just armed or is already in flight — `options` empty),
    /// `"unsupported"` (the model has no reasoning control, or there's no active
    /// session — `options` empty), or `"ready"` (`options`/`selected` populated).
    /// `note` carries the human-readable reason/hint in every state. Pushed
    /// out-of-band (not fingerprinted) whenever the daemon answers a
    /// `GetEffortOptions` — ALWAYS a reply so the picker never hangs.
    #[serde(rename_all = "camelCase")]
    EffortOptions {
        options: Vec<String>,
        selected: usize,
        note: String,
        state: String,
    },
    /// The TUI's animated startup splash ([`crate::app::mode::Mode::Loading`]),
    /// projected so the GUI can render its own loading overlay while a returning
    /// session warms asynchronously (workspace reindex + project-docs awareness
    /// summary). `workspace`/`awareness` are one of `"pending"`/`"running"`/
    /// `"done"`/`"skipped"`/`"failed"` (mirrors [`crate::app::mode::WarmStatus`],
    /// dropping `Done`'s carried detail string — the webview shows a generic
    /// "ready" glyph, not the TUI's dim detail text). `active` is `true` while the
    /// foreground session's mode is `Loading`; pushed exactly ONCE more with
    /// `active: false` (`workspace`/`awareness` both `"done"`) the frame the mode
    /// leaves `Loading`, then never again until the next warm cycle — see
    /// `serialize_and_push`'s dedup comment for why the webview can rely on that
    /// single terminal `false` frame to dismiss its overlay.
    Loading {
        active: bool,
        workspace: String,
        awareness: String,
    },
}

/// Push a swap-START [`PushEnvelope::Switching`] for target session `to`. Called at every
/// swap seam (a hub `Select`/`New` in either host state, a daemon `NewSession` hand-off)
/// the instant BEFORE teardown, so React raises a full-screen loader across the attach
/// gap; the next attached `Snapshot` clears it. Fire-and-forget: a serialise failure is
/// swallowed (the loader just never rises — no worse than before this signal existed).
pub(super) fn push_switching(push: &dyn Fn(String), to: &str) {
    let env = PushEnvelope::Switching { to: to.to_string() };
    if let Ok(json) = serde_json::to_string(&env) {
        push(json);
    }
}

/// Emit a one-shot `ModelList` envelope (out-of-band, un-fingerprinted) for the GUI
/// Connector model-id picker. Shared by the UN-ATTACHED swapper fallback
/// ([`super::host_swapper`]) so a detached `ListModels` lands the SAME envelope the attached
/// `push_loop` re-push produces — byte-compatible (echoed `provider` + `models`).
pub(super) fn push_model_list(push: &dyn Fn(String), provider: String, models: Vec<String>) {
    super::render::emit(push, &PushEnvelope::ModelList { provider, models });
}

/// Emit a one-shot `RouteList` envelope for the GUI Connector route picker, flattening each
/// daemon `ModelEndpointWire` to the camelCase `PushRoute` JS contract — the SAME mapping the
/// attached `push_loop` does. Shared by the UN-ATTACHED swapper fallback so a detached
/// `ListRoutes` lands the SAME envelope (echoed `provider` + `modelId`; an EMPTY `routes`
/// leaves the form showing only its synthetic "Auto").
pub(super) fn push_route_list(
    push: &dyn Fn(String),
    provider: String,
    model_id: String,
    routes: Vec<crate::ipc::proto::ModelEndpointWire>,
) {
    let env = PushEnvelope::RouteList {
        provider,
        model_id,
        routes: routes
            .iter()
            .map(|r| PushRoute {
                name: r.name.clone(),
                provider_name: r.provider_name.clone(),
                price_prompt: r.price_prompt.clone(),
                price_completion: r.price_completion.clone(),
                uptime_last_30m: r.uptime_last_30m,
            })
            .collect(),
    };
    super::render::emit(push, &env);
}

/// Emit a one-shot `FileDiff` envelope for the GUI Explore panel's Monaco diff tab,
/// carrying a host-computed [`super::diff::FileDiffResult`] verbatim. Shared by the
/// UN-ATTACHED swapper fallback and the attached `push_loop`'s off-thread worker,
/// since a `FileDiff` is serviced entirely host-side regardless of attach state.
pub(super) fn push_file_diff(push: &dyn Fn(String), result: super::diff::FileDiffResult) {
    let env = PushEnvelope::FileDiff {
        path: result.path,
        original: result.original,
        modified: result.modified,
        error: result.error,
        binary: result.binary,
        origin: result.origin.to_string(),
    };
    super::render::emit(push, &env);
}

/// Emit a one-shot `UsagePreview` envelope for the GUI activity-bar Usage panel, carrying
/// a host-computed [`super::diff::UsagePreviewResult`] plus the `scope` ("all"/"session") AND
/// `session_id` (the session uuid actually queried, `None` for "all") the request was
/// made under, both echoed back verbatim. Shared by the UN-ATTACHED swapper fallback and
/// the attached `push_loop`'s off-thread worker, since a `UsagePreview` is serviced
/// entirely host-side (the global ledger) regardless of attach state.
pub(super) fn push_usage_preview(
    push: &dyn Fn(String),
    result: super::diff::UsagePreviewResult,
    scope: String,
    session_id: Option<String>,
) {
    let env = PushEnvelope::UsagePreview {
        cost: result.cost,
        tokens_in: result.tokens_in.max(0) as u64,
        tokens_cached: result.tokens_cached.max(0) as u64,
        tokens_out: result.tokens_out.max(0) as u64,
        calls: result.calls.max(0) as u64,
        days: result
            .days
            .into_iter()
            .map(|(epoch, cost)| PushUsageDay { epoch, cost })
            .collect(),
        top_models: result
            .top_models
            .into_iter()
            .map(|m| PushUsageModel {
                model_id: m.model_id,
                cost: m.total_cost,
                calls: m.call_count.max(0) as u64,
            })
            .collect(),
        scope,
        session_id,
    };
    super::render::emit(push, &env);
}

/// Emit a one-shot `SettingsValues` envelope for the GUI Settings tab. Shared by the
/// attached `push_loop` intercept (which unpacks the daemon's `DaemonEvent::SettingsValues`
/// reply) and the UN-ATTACHED swapper fallback ([`super::host_swapper`]), so a detached
/// `GetSettings` lands the SAME envelope the attached path produces.
#[allow(clippy::too_many_arguments)]
pub(super) fn push_settings_values(
    push: &dyn Fn(String),
    name: String,
    workdir: Vec<String>,
    short_send: bool,
    sliding_cache: bool,
    bash_saving: bool,
    internet_mode: String,
    palette: String,
    effort: String,
) {
    super::render::emit(
        push,
        &PushEnvelope::SettingsValues {
            name,
            workdir,
            short_send,
            sliding_cache,
            bash_saving,
            internet_mode,
            palette,
            effort,
        },
    );
}
