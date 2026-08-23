//! `Push*` row/DTO structs for the GUI push-envelope bridge — split out of
//! `push_proto.rs` for file size (pure code motion, no behaviour change).
//! [`super::push_proto::PushEnvelope`] re-exports every struct here at
//! `pub(super)` (matching each struct's own declared visibility exactly) so
//! `push_proto.rs`, `project.rs`, `project_config.rs`, and `push_loop.rs` keep
//! using their existing `super::push_proto::PushX` import paths unchanged.

// ─── host-relay push envelopes (native-React GUI client) ─────────────────────────
//
// The GUI host is itself the daemon client (see `crate::app::runtime::gui`): instead
// of drawing the shadow `AppState` to a terminal, it SERIALISES it into the JSON
// envelopes the React client consumes and pushes them through
// `window.__komaClient.push(...)`. These structs are the Rust half of the bridge
// contract — `#[serde(tag = "k")]` names each envelope, matching the JS `push`
// dispatcher's `k` switch EXACTLY. The host always pushes AUTHORITATIVE full values
// (React REPLACES on `StreamMsg` / `Reasoning`, never appends); [`PushState`] dedups
// so an unchanged frame emits nothing.

/// One committed conversation turn in a [`PushEnvelope::Snapshot`].
///
/// `content` + `reasoning` are the plain text body + display-only thinking (unchanged
/// from W0-W3). `toolCalls` is the fuller turn projection W4 adds: an assistant turn's
/// requested tool calls, each already JOINED to its paired `Role::Tool` result so React
/// can render the TUI's `● call → inline result box` grammar 1:1 without accumulating
/// (the host pushes the AUTHORITATIVE full array; React REPLACES). Empty for non-tool
/// turns (skipped from the wire) and for user messages.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PushMsg {
    pub(super) role: &'static str,
    /// Special render kind for a USER message, detected daemon-side from its
    /// invisible sentinel prefix and STRIPPED out of `content` so React never
    /// renders a raw sentinel char: `"shell"` (a `!`-shell `$ cmd`+output entry,
    /// `render_shell_block`) or `"bashNudge"` (a bg-bash completion nudge,
    /// `render_bash_nudge_block`). `None`/absent on a plain user or assistant
    /// message → React renders the normal user band / assistant block.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) kind: Option<&'static str>,
    pub(super) content: String,
    pub(super) reasoning: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) tool_calls: Vec<PushToolCall>,
    /// Image attachments carried by this (user) message — each `[Image #N]`
    /// marker's on-disk basename + kind, so React can render the warn attachment
    /// card (mirrors `render_attachment_card`, `transcript.rs:581`). Empty on
    /// messages with no attachments (skipped from the wire).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) attachments: Vec<PushAttachment>,
}

/// One tool CALL on an assistant [`PushMsg`], with its paired result folded in (the
/// TUI resolves call→result live by `tool_call_id`; the projection does the same join
/// so React doesn't need the raw `Role::Tool` messages). Mirrors `render_tool_lines`
/// (`view/chat/transcript.rs:631`):
/// - `signature` = `format_tool_signature(name,args)`, the quote-less `name(args)`
///   header the TUI shows (already flattened + capped at 60 chars).
/// - `label` = the box label (`bash`/`read`/`grep`/…) when this tool's output is BOXED
///   (`tool_box_label`), else `None` → React renders the terse one-liner fallback.
/// - `output` = the paired `Role::Tool` result content (`None` while in-flight).
/// - `status` = `"done"` once a matching `Role::Tool` result exists, else `"pending"`
///   (drives the ⚙→✓ glyph flip; resolved fresh each Snapshot so a late-landing result
///   re-emits — see the folded fingerprint).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PushToolCall {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) args: String,
    pub(super) signature: String,
    pub(super) label: Option<String>,
    pub(super) output: Option<String>,
    pub(super) status: &'static str,
}

/// One STAGED (not-yet-sent) composer attachment chip in a [`PushEnvelope::Snapshot`].
/// Mirrors the daemon's `pending_attachments`: `marker_n` (serialised `markerN`) ties
/// the chip to its `[Image #N]` marker so React can round-trip it back in a
/// `RemoveAttachment`; `name` is the on-disk basename; `kind` is `"image"`/`"file"`
/// derived from the sniffed mime. Authoritative full array — React REPLACES on each
/// Snapshot (a stage/drop re-emits the Snapshot via the folded fingerprint).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PushAttachment {
    pub(super) marker_n: usize,
    pub(super) name: String,
    pub(super) kind: &'static str,
}

/// One sub-agent row in a [`PushEnvelope::Snapshot`]. `name` is the agent definition
/// name, `summary` is the compact one-line label (the truncated task), and `status` is
/// the canonical lifecycle string `running`/`done`/`killed`/`error`. The live
/// `transcript`/`liveText`/`thinking` are folded in ONLY for the sub-agent THIS client is
/// streaming into an Explore stream tab (`GuiReq::SetStreamView`) — every other row stays
/// list+status only, so a non-viewed agent's per-step churn never re-emits this Snapshot.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PushSubAgent {
    /// Stable per-session sub-agent id — the handle the GUI kill button round-trips as
    /// [`crate::app::runtime::gui`]'s `KillSubagent { id }`.
    pub(super) id: usize,
    pub(super) name: String,
    pub(super) status: &'static str,
    pub(super) summary: String,
    /// Whether this sub-agent is already backgrounded (detached). A detached agent's
    /// background button is hidden (it's already backgrounded); React shows a subtle
    /// "bg" hint instead.
    pub(super) detached: bool,
    /// Whether this sub-agent is currently PARKING the main turn — i.e. it still has a
    /// live `tool_call_id` (the model's delegating tool call hasn't been answered yet).
    /// Only a `running && !detached && blocking` agent is eligible for the
    /// background button / Ctrl+B (mirrors the TUI's `Action::BackgroundSubagent`
    /// eligibility gate). Never the raw tool_call_id — just the boolean.
    pub(super) blocking: bool,
    /// The sub-agent's display-ready TRANSCRIPT lines (the SAME source the TUI `$`-panel
    /// preview renders, `SubAgent::transcript`), so the stream tab matches the TUI.
    /// `Some` ONLY for the VIEWED sub-agent; `None` for every other row (kept off the wire
    /// + out of this Snapshot's fingerprint). `Some([])` = viewed but no lines yet (a
    ///   restored agent, or one that just started).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) transcript: Option<Vec<String>>,
    /// The live in-progress report tail for the CURRENT (not-yet-committed) turn
    /// (`SubAgent::live_text`), shown dim under the transcript. Viewed sub-agent only, and
    /// only when non-empty; `None` otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) live_text: Option<String>,
    /// The sub-agent's latest thinking block (the most recent committed message's
    /// reasoning), for a dim collapsible block in the stream tab. Viewed sub-agent only;
    /// `None` when the agent produced no reasoning.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) thinking: Option<String>,
}

/// One background-bash job row in a [`PushEnvelope::Snapshot`]. `id` is the model-facing
/// job id (`bash-<n>`), `cmd` is the shell command, and `status` is the canonical
/// lifecycle string `running`/`done`/`killed`/`error`. `outputTail` is the captured output
/// tail, folded in ONLY for the job THIS client is streaming into a stream tab
/// (`GuiReq::SetStreamView`) — `None` for every other row (so an un-viewed job's per-line
/// output never re-emits this Snapshot).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PushBashJob {
    pub(super) id: String,
    pub(super) cmd: String,
    pub(super) status: &'static str,
    /// Captured OUTPUT TAIL of the VIEWED job (from the shadow's inert job, baked from the
    /// projection's `output_tail`). `None` for every non-viewed row. `Some("")` = viewed
    /// but no output (a restored job, or one with none yet).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) output_tail: Option<String>,
}

/// One cumulative file-change row in a [`PushEnvelope::Snapshot`] (#24): the
/// (workspace-relative when possible) `path` this session's `write`/`edit`/`delete`
/// touched, and its latest `status` (`"added"`/`"modified"`/`"deleted"`, dedup by
/// path). Persisted daemon-side so it survives compaction + close/reopen.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PushFileChange {
    pub(super) path: String,
    pub(super) status: String,
}

/// One Plan-mode todo row in a [`PushEnvelope::Snapshot`], for the Explore "PLAN"
/// section. The two locked workflow rails ride this too now (flagged via
/// `locked`, not dropped), so the section shows TUI-parity rails right after
/// `plan_enter`. `status` is the wire label (`"pending"`/`"in_progress"`/
/// `"completed"`/`"cancelled"`, `TodoStatus::label`).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PushPlanTodo {
    pub(super) content: String,
    pub(super) status: &'static str,
    pub(super) locked: bool,
}

/// The palette roles the React chat paints with (resolved from the shadow's TUI
/// [`crate::view::theme::Palette`], so a themed daemon repaints the chat live).
/// `bg`/`fg` drive the window chrome; `accent`/`dim`/`panel` are the same three
/// roles `view::draw` uses for the chat grammar (accent bullets/rails, dim
/// thinking/tool text, the user-message band = `panel`), each `#rrggbb`.
#[derive(serde::Serialize, PartialEq, Clone)]
pub(super) struct PushPalette {
    pub(super) bg: String,
    pub(super) fg: String,
    pub(super) accent: String,
    pub(super) dim: String,
    pub(super) panel: String,
    pub(super) warn: String,
    pub(super) success: String,
    pub(super) info: String,
    pub(super) error: String,
    /// Whether this palette reads as a dark theme (derived from `bg`'s relative
    /// luminance — see `project_config::palette_is_dark`), so panel iframes and other
    /// theme-aware consumers can pick a dark/light variant without parsing `bg` hex.
    pub(super) dark: bool,
}

/// One named palette in the [`PushEnvelope::Config`] `palettes` catalogue — the GUI
/// Settings tab's Appearance grid renders a movie-strip card per entry. `name` is the
/// `view::theme::PALETTES` registry key (round-trips as `SetTheme { name }`); `colors` is
/// the palette's role colours as `#rrggbb` strings in the FIXED order
/// `[bg, fg, dim, accent, panel, sel_bg, sel_fg, success, warn, error, info]`, resolved
/// through the SAME `color_hex` conversion [`PushPalette`] uses.
#[derive(serde::Serialize)]
pub(super) struct PushPaletteInfo {
    pub(super) name: String,
    pub(super) colors: Vec<String>,
}

/// A COOKING-pane row in a [`PushEnvelope::Hub`]. The synthetic `[+ new session]`
/// row carries only `kind`/`id`/`name`; a real session row fills the rest (the
/// session-only fields are `Option` + skip-if-none so the two shapes match the
/// contract's per-row shape).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PushCooking {
    pub(super) kind: &'static str,
    pub(super) id: Option<String>,
    pub(super) name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) working: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) foreground: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) dir_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) current_dir: Option<bool>,
}

/// A HISTORY-pane row in a [`PushEnvelope::Hub`] (an on-disk session not currently
/// live). `id` is the session UUID (the on-disk dir name); `last_active` is unix ms.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PushHistory {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) last_active: u64,
    pub(super) dir_label: String,
    pub(super) current_dir: bool,
}

/// One provider row in a [`PushEnvelope::Config`] (the Connector panel's ProviderForm
/// model). `id` is the config uuid (stable identity a `SetProvider`/`DeleteProvider`
/// round-trips). The plaintext `api_key` is NEVER sent to the webview (devtools are
/// enabled, and the key would sit readable in the DOM/console) — only `has_key`, a
/// presence flag the form uses to render a "leave blank to keep" placeholder. Saving
/// with a blank key preserves the existing stored key (see `upsert_provider`).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PushProvider {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) endpoint: String,
    pub(super) has_key: bool,
    /// `true` for the auto-provisioned keyless koma-free [`ApiType::KomaFree`] connection
    /// (minted by onboarding / the `/free` toggle, never user-created). React HIDES or
    /// read-only-LOCKS this row in the Connector ProviderForm — it has no editable key /
    /// endpoint (the endpoint + dual-header auth are forced daemon-side) so surfacing it as
    /// an editable provider is a leak. `false` for every real, user-managed provider.
    pub(super) is_koma_free: bool,
}

/// One model row in a [`PushEnvelope::Config`] (the Connector panel's ModelForm model).
/// `id` is the config/session-override uuid; `provider` is the serving provider's uuid
/// (matches the ProviderForm option value); `roles` are the lowercase role tokens; and
/// `scope` is `"global"` (from `AppConfig.models`) or `"local"` (from the foreground
/// session's `settings.session_models`).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PushModel {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) model_id: String,
    pub(super) provider: String,
    pub(super) route: String,
    pub(super) roles: Vec<&'static str>,
    pub(super) scope: &'static str,
    /// The SYNTHETIC "advertised free" flag (wave-3+4 free-pin): `true` ONLY for the
    /// keyless koma-free row the host prepends to the quick-picker list (its `id` is the
    /// opaque `KOMA_FREE_SENTINEL`, not a real config uuid); `false` for every real
    /// global/local model. React sorts a `free` row to the TOP of the picker and badges
    /// it; picking it round-trips the sentinel id back as `SetSessionMain`, which the
    /// daemon routes through the `/free` find-or-create flow.
    pub(super) free: bool,
    /// For a `"local"`-scope override CLONED from a global entry, the `uuid` of that
    /// global (`ModelEntry::source_uuid`) — serialized as `sourceUuid`. The GUI
    /// ModelPicker lights the active session-Main's origin row by matching THIS exact
    /// id, falling back to a name compare only for an override authored before the
    /// field existed. Omitted from the wire (`None`) for every global row, the synthetic
    /// free row, and a directly-authored local entry.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) source_uuid: Option<String>,
    /// W12b: the id of the extension that flagged THIS model its recommended `default` (via
    /// `models.register { default: true }`) — serialized as `recommendedBy`, an ADDITIVE hint
    /// the GUI picker can badge ("recommended by <ext>"). Set only for a model uuid present in
    /// `AppConfig::ext_preferred_models`; omitted (`None`) for every other row. Purely a hint:
    /// when Main was unset the preferred model is auto-assigned (vacuum-fill) and this flag is
    /// moot; when Main was already a real user choice this is the ONLY surfacing of a later
    /// extension's recommendation (it never reassigns Main). GUI picker rendering of the badge
    /// is deferred (wire-only for now).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) recommended_by: Option<String>,
}

/// One MCP-server row in a [`PushEnvelope::Config`] (the McpPanel Server model). `id` is
/// the config uuid. The daemon stores `args` as a `Vec<String>` and `env` as ordered
/// `(key,value)` pairs; both are rendered back into the panel's single-line STRING forms
/// (`args` space-joined, `env` as `K=V, K2=V2`) so the round-trip matches the form
/// exactly (a `SetMcpServer` re-parses them daemon-side).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PushMcpServer {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) enabled: bool,
    pub(super) transport: &'static str,
    pub(super) command: String,
    pub(super) args: String,
    pub(super) env: String,
    pub(super) url: String,
    /// STATIC config-field tool count (from the Config envelope; no longer live).
    /// Runtime status (connected, live tool count, errors) now comes from the
    /// dedicated `McpStatus` push envelope instead.
    pub(super) tool_count: usize,
    /// Human-readable error string when the server's background connect failed.
    /// `None` = connected (or still connecting). Shown as an amber/red indicator.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) error: Option<String>,
}

/// One MCP server row in a [`PushEnvelope::McpStatus`] reply — live connection status
/// from `McpManager::server_status_cached()` and `server_errors()`. `id` is the server
/// config uuid; `connected` is true when a live connection exists; `toolCount` is the
/// discovered tool count (0 when connected with no tools, or when not connected);
/// `error` is the human-readable connection error if any.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PushMcpStatusServer {
    pub(super) id: String,
    pub(super) connected: bool,
    pub(super) tool_count: usize,
    /// Optional — omitted from JSON when None.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) error: Option<String>,
}

/// One provider-route row in a [`PushEnvelope::RouteList`] (the Connector ModelForm's
/// live ROUTE picker). Flattened from a daemon `ModelEndpointWire`: `providerName` is the
/// serving provider's display name; `pricePrompt`/`priceCompletion` are USD-per-token
/// strings ("0" = free, `null` = unknown); `uptimeLast30m` is the rolling uptime percent
/// (`null` = unknown). React prepends a synthetic "Auto" option client-side; picking a
/// route round-trips its provider name as the model's pinned `route` string.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PushRoute {
    pub(super) name: Option<String>,
    pub(super) provider_name: Option<String>,
    pub(super) price_prompt: Option<String>,
    pub(super) price_completion: Option<String>,
    pub(super) uptime_last_30m: Option<f64>,
}

/// The paused tool call surfaced to the GUI approval overlay when the foreground session
/// is `awaitingApproval` (wave-7). Two shapes ride the SAME gate, distinguished by `name`:
///   - `name == "plan_ready"` — a Plan-mode plan digest is parked; the digest itself is
///     already in the transcript as THIS call's rewritten `highlights` args, so React shows
///     the approve / approve&compact / deny controls and answers with `GuiReq::PlanDecision`.
///   - any other `name` — a risky/classifier-flagged tool call is parked; React renders the
///     two-button approve/deny card showing `name` + `args` (+ the reason line), answering
///     with `GuiReq::ApproveTool`.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PushPendingCall {
    pub(super) name: String,
    pub(super) args: String,
}

/// One day's cost in a [`PushEnvelope::UsagePreview`]'s 7-entry daily series. `epoch` is
/// the LOCAL-midnight unix-seconds boundary for that day (see `compute_usage_preview`).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PushUsageDay {
    pub(super) epoch: i64,
    pub(super) cost: f64,
}

/// One model row in a [`PushEnvelope::UsagePreview`]'s top-3 list.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PushUsageModel {
    pub(super) model_id: String,
    pub(super) cost: f64,
    pub(super) calls: u64,
}

/// One time-series point in a [`PushEnvelope::Analytics`] reply.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PushAnalyticsSeriesPoint {
    pub(super) epoch: i64,
    pub(super) cost: f64,
    pub(super) tokens: i64,
}

/// One model row in a [`PushEnvelope::Analytics`] reply (full token breakdown).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PushAnalyticsModel {
    pub(super) model_id: String,
    pub(super) cost: f64,
    pub(super) tokens_in: u64,
    pub(super) tokens_cached: u64,
    pub(super) tokens_out: u64,
    pub(super) calls: u64,
}

/// One entry in a Coding panel directory listing.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct PushFileTreeEntry {
    pub name: String,
    pub path: String,
    /// Whether this entry is a directory.
    #[serde(rename = "isDir")]
    pub is_dir: bool,
}
