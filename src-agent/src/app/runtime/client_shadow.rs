//! Shadow mode reconstruction for the thin attach client.
//!
//! Each `shadow_*` function rebuilds a REAL mode-state / runtime value from its
//! wire projection so the unmodified `view::draw` renders it. The client never
//! mutates these (input is forwarded to the daemon); they only need to be
//! faithful enough to DRAW.

use std::time::{Duration, Instant};

use crate::app::mode::agents::{
    AgentEditField, AgentScope, AgentSubMode, AgentsState, ModelPickerState, ToolPickerState,
};
use crate::app::mode::editor::TextEditorState;
use crate::app::mode::settings::{
    ModelDraft, ModelModal, PathPicker, PickerMode, ProviderDraft, ProviderModal, RolePickerState,
    SettingsState,
};
use crate::app::mode::{
    CookingEntry, EffortPickerState, HistoryEntry, HubPane, KeyInputForm, LoadingState, PickerState, RewindEntry, RewindState, SessionHub, SessionKind, UsageMetric,
    UsageNavState, UsageRange, UsageView, WarmStatus,
};
use crate::app::state::{SessionRuntime, ToastKind};
use crate::app::subagent::{PendingSubagent, SubAgent, SubAgentStatus};
use crate::dto::openrouter::{ModelEndpoint, ModelPricing};
use crate::ipc::proto::{
    AgentEntry, AgentModelPickerSnapshot, AgentsSnapshot, EffortSnapshot, KeyInputSnapshot,
    LoadingSnapshot, ModelModalSnapshot, PathPickerSnapshot, PickerSnapshot, RewindSnapshot,
    SessionHubSnapshot, SessionSnapshot, SettingsSnapshot, SubAgentSnapshot,
    TextEditorSnapshot, ToolPickerSnapshot, WarmStatusWire,
};
use crate::model::app_config::{ApiType, ModelRole, ThemeMode};
use crate::model::conversation::Conversation;
use crate::model::session::Session;
use crate::model::settings::{InternetMode, Settings};
use crate::model::store::SessionMeta;

/// Map a wire toast `kind` string ("error" / "info") to the local [`ToastKind`].
/// Anything unexpected degrades to `Info` (a neutral box, never a false error).
pub(crate) fn toast_kind(kind: &str) -> ToastKind {
    match kind {
        "error" => ToastKind::Error,
        _ => ToastKind::Info,
    }
}

/// Build a shadow [`SessionRuntime`] from one [`SessionSnapshot`].
///
/// Carries the stable id + the render-relevant fields (streaming buffers, token /
/// cost counters, approval flags, working/finished-unseen). The committed messages +
/// name + model are reconstructed into a minimal [`Session`] so the unmodified chat
/// transcript/header/input renderers consume it exactly as they do a live session.
/// Every NON-render field stays at `Default` — the client never advances a turn, so
/// the tool/sub-agent state machines and channels are never read.
///
/// Both the QUEUED [`PendingSubagent`]s and the RUNNING/finished `subagents` are
/// reconstructed from their plain-data projections, so the `$` panel rows AND the
/// full-screen sub-agent viewer (both rendered FROM Chat mode) draw off real shadow
/// data. A live [`crate::app::subagent::SubAgent`] needs a tokio `AbortHandle` +
/// receiver, which require a runtime in scope — `client_run` enters the runtime
/// context for the render loop so [`shadow_subagent`] can mint inert ones (the
/// client never drives a sub-agent; the handle/rx exist only to satisfy the type).
pub(crate) fn shadow_session_runtime(s: &SessionSnapshot) -> SessionRuntime {
    let mut rt = SessionRuntime::new();
    rt.id = s.id.clone();
    rt.session = Some(shadow_session(s));
    // Mirror the daemon's effective cwd onto the shadow as the live override, so
    // the reconstructed runtime's `effective_cwd()` matches (the shadow session's
    // own `settings.workdir` isn't projected, so this is the only cwd source).
    // Empty only when the daemon had no session; leave the default `None` then.
    if !s.cwd.is_empty() {
        rt.active_cwd = Some(std::path::PathBuf::from(&s.cwd));
    }
    rt.streaming = s.streaming.clone();
    rt.stream_reasoning = s.stream_reasoning.clone();
    rt.tokens_in = s.tokens_in;
    rt.tokens_out = s.tokens_out;
    rt.cost = s.cost;
    rt.tokens_cached = s.tokens_cached;
    // `waiting` drives the local input-poll cadence + the comet; mirror the snapshot's
    // composite `working` so a parked/streaming background session keeps the shadow
    // ticking fast and shimmering, matching the daemon.
    rt.waiting = s.working;
    rt.awaiting_approval = s.awaiting_approval;
    rt.approval_reason = s.approval_reason.clone();
    // The pending tool-call round + cursor so the Chat-mode approval overlay (gated
    // on `awaiting_approval`) renders the real paused call — name, args, payload
    // preview — exactly as the local TUI does. `ToolCall` is plain data; the client
    // never executes it (the y/n is forwarded as `ApproveTool`).
    rt.pending_tool_calls = s.pending_tool_calls.clone();
    rt.tool_idx = s.tool_idx;
    rt.finished_unseen = s.finished_unseen;
    // Reconstruct the queued delegations (plain data) so the remote `$` panel can
    // list the same "pending" rows the local TUI shows. FIFO order is preserved.
    rt.pending_subagents = s
        .pending_subagents
        .iter()
        .map(|p| PendingSubagent {
            id: p.id,
            agent_name: p.agent_name.clone(),
            prompt: p.prompt.clone(),
            // The turn-bookkeeping call id is daemon-internal + never rendered, and the
            // client never advances a turn, so a shadow pending entry carries `None`.
            tool_call_id: None,
        })
        .collect();
    // Reconstruct the running/finished sub-agents (plain data + an inert handle/rx)
    // so the `$` panel list AND the full-screen viewer render off real shadow data.
    rt.subagents = s.subagents.iter().map(shadow_subagent).collect();
    rt
}

/// Rebuild a render-only [`SubAgent`] from its projection.
///
/// Carries every field the `$` panel + the full-screen viewer read (id, agent name,
/// label, status, transcript, structured messages). The two runtime-bound fields a
/// live `SubAgent` requires — the `abort` [`tokio::task::AbortHandle`] and the `rx`
/// event receiver — are minted INERT here: the client never drains `rx` and never
/// aborts (it forwards every key to the daemon, which owns the real sub-agent), so a
/// no-op aborted task's handle + a fresh unused channel satisfy the type without ever
/// being driven. `tokio::spawn` needs a runtime in scope, which `client_run` enters
/// for the render loop. The non-rendered bookkeeping (`model_id`, `tool_call_id`,
/// usage counters) is left empty/zero — the viewer and panel never read it.
pub(crate) fn shadow_subagent(sa: &SubAgentSnapshot) -> SubAgent {
    // Inert abort handle: a task that completes immediately; its handle is never used
    // to abort anything (the daemon owns the real task). Cheap + completes at once.
    let abort = tokio::spawn(std::future::ready(())).abort_handle();
    // Fresh receiver the client never drains (the daemon folds real events; a shadow
    // sub-agent's content arrives wholesale via the next snapshot's `messages`).
    let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
    SubAgent {
        id: sa.id,
        agent_name: sa.name.clone(),
        label: sa.label.clone(),
        // Not rendered by the panel/viewer; left blank (the wire omits it).
        model_id: String::new(),
        status: shadow_subagent_status(&sa.status),
        abort,
        rx,
        transcript: sa.transcript.clone(),
        messages: sa.messages.clone(),
        tool_call_id: None,
        usage_tokens_in: 0,
        usage_tokens_out: 0,
        usage_cost: 0.0,
    }
}

/// Map a wire sub-agent status string back to a [`SubAgentStatus`].
///
/// The daemon flattens the lifecycle enum to a short string (see
/// `ipc::snapshot::subagent_snapshot`): `"running"` / `"done"` / `"killed"` /
/// `"error: <detail>"`. The `Done` final-answer payload is NOT carried (the viewer —
/// the projection's target — renders the transcript + the short status tag, neither of
/// which uses it), so a reconstructed `Done` carries an empty answer; an `Error`
/// keeps its detail (the panel's status line shows it). Unknown → `Running` (the
/// safe "still going" default, never lost).
fn shadow_subagent_status(status: &str) -> SubAgentStatus {
    match status {
        "done" => SubAgentStatus::Done(String::new()),
        "killed" => SubAgentStatus::Killed,
        s if s.starts_with("error:") => {
            SubAgentStatus::Error(s.trim_start_matches("error:").trim().to_string())
        }
        _ => SubAgentStatus::Running,
    }
}

/// Reconstruct a minimal [`Session`] from a [`SessionSnapshot`] for rendering.
///
/// Only the fields the chat view reads are meaningful: `name` (the input-tab label),
/// `conversation` (the transcript), and `settings.model` (the model-name row). The
/// path / pwd_hash / api_key are render-irrelevant on the client and left empty —
/// the client never saves, sends, or locks anything.
pub(crate) fn shadow_session(s: &SessionSnapshot) -> Session {
    // Seed `settings.model` with the daemon-side resolved model id projected in the
    // snapshot. The client's shadow config is keyless + catalogue-cleared, so
    // resolve_role on the client would return empty; using the projected id means
    // the chat header always shows the same model name the daemon resolved.
    let settings = Settings {
        name: s.name.clone(),
        model: s.resolved_model_id.clone(),
        ..Default::default()
    };

    // Re-attach the display-only reasoning the wire carried out-of-band. The
    // `ChatMessage::reasoning` field is `#[serde(skip)]`, so every deserialised
    // message arrives with `reasoning: None`; without this fold-back a committed
    // turn's thinking block would never render on the client (it would only show
    // while the live `stream_reasoning` buffer streamed, then vanish on finalize).
    // The side-channel is index-aligned with `messages`; a missing/short entry
    // (the common no-reasoning case ships an empty vec) leaves `reasoning` at None.
    let mut messages = s.messages.clone();
    for (i, msg) in messages.iter_mut().enumerate() {
        if let Some(Some(reasoning)) = s.committed_reasoning.get(i) {
            msg.reasoning = Some(reasoning.clone());
        }
    }

    Session::new(
        s.id.clone(),
        std::path::PathBuf::new(),
        String::new(),
        settings,
        Conversation::from_messages(messages),
    )
}

// ─── mode reconstruction (stage 2: core interactive modes) ───────────────────
//
// Each rebuilds a REAL mode-state value from its wire projection so the unmodified
// `view::draw` renders it. The client never mutates these (input is forwarded to
// the daemon); they only need to be faithful enough to DRAW. None hold a channel /
// `Instant`-clock that must keep ticking except `Loading::started`, which is
// re-anchored from the projected elapsed-ms so its footer counter matches.

/// Rebuild the first-run wizard form ([`KeyInputForm`]) from its projection.
pub(crate) fn shadow_key_input(f: KeyInputSnapshot) -> KeyInputForm {
    KeyInputForm {
        step: f.step,
        field: f.field,
        endpoint: f.endpoint,
        api_key: f.api_key,
        model: f.model,
        query: f.query,
        result_sel: f.result_sel,
        first_run: f.first_run,
        from_picker: f.from_picker,
    }
}

/// Rebuild the loading splash ([`LoadingState`]) from its projection. The footer's
/// elapsed clock is re-anchored (`now - elapsed`) so it continues from the daemon's
/// phase rather than resetting to 0 on each snapshot.
pub(crate) fn shadow_loading(s: LoadingSnapshot) -> LoadingState {
    LoadingState {
        started: Instant::now() - Duration::from_millis(s.elapsed_ms),
        frame: s.frame,
        workspace: shadow_warm_status(s.workspace),
        awareness: shadow_warm_status(s.awareness),
    }
}

/// Map a [`WarmStatusWire`] back to a [`WarmStatus`].
fn shadow_warm_status(w: WarmStatusWire) -> WarmStatus {
    match w {
        WarmStatusWire::Pending => WarmStatus::Pending,
        WarmStatusWire::Running => WarmStatus::Running,
        WarmStatusWire::Done(d) => WarmStatus::Done(d),
        WarmStatusWire::Skipped => WarmStatus::Skipped,
        WarmStatusWire::Failed => WarmStatus::Failed,
    }
}

/// Rebuild the two-pane session hub ([`SessionHub`]) from its projection.
///
/// The COOKING rows' live `idx` (the daemon's `sessions` index, used on Enter) is
/// not projected and not rendered, so reconstructed rows carry `0` for it; the
/// HISTORY rows' live `path` is likewise daemon-only, rebuilt as an empty path. The
/// client never acts on these — Enter is forwarded for the daemon to resolve.
pub(crate) fn shadow_session_hub(h: SessionHubSnapshot) -> SessionHub {
    SessionHub {
        cooking: h
            .cooking
            .into_iter()
            .map(|c| CookingEntry {
                idx: 0, // daemon-side index; not rendered, resolved on the daemon
                kind: match c.kind.as_str() {
                    "new_session" => SessionKind::NewSession,
                    _ => SessionKind::Session,
                },
                name: c.name,
                working: c.working,
                is_foreground: c.is_foreground,
            })
            .collect(),
        history: h
            .history
            .into_iter()
            .map(|e| HistoryEntry {
                path: std::path::PathBuf::new(), // daemon-side load target; not rendered
                name: e.name,
                last_active: std::time::UNIX_EPOCH
                    + Duration::from_secs(e.last_active_secs),
            })
            .collect(),
        focus: if h.focus_cooking {
            HubPane::Cooking
        } else {
            HubPane::History
        },
        cooking_selected: h.cooking_selected,
        history_selected: h.history_selected,
    }
}

/// Rebuild the `/settings` dashboard ([`SettingsState`]) from its projection — the
/// largest reconstruction. Every draft + list + modal + picker is restored so the
/// settings view (and its pure helper methods, which recompute from these same
/// fields) renders exactly as the daemon's would.
pub(crate) fn shadow_settings(s: SettingsSnapshot) -> SettingsState {
    SettingsState {
        cat: s.cat,
        field: s.field,
        in_detail: s.in_detail,
        editing: s.editing,
        api_key: s.api_key,
        model: s.model,
        provider: s.provider,
        name: s.name,
        theme: shadow_theme(&s.theme),
        accent: s.accent,
        workdir: s.workdir,
        awareness_enabled: s.awareness_enabled,
        awareness_inherit: s.awareness_inherit,
        awareness_model: s.awareness_model,
        awareness_provider: s.awareness_provider,
        classifier_enabled: s.classifier_enabled,
        classifier_model: s.classifier_model,
        classifier_provider: s.classifier_provider,
        allowed_folders: s.allowed_folders,
        short_send_enabled: s.short_send_enabled,
        sliding_cache: s.sliding_cache,
        internet_mode: shadow_internet_mode(&s.internet_mode),
        cwd: std::path::PathBuf::from(s.cwd),
        list_editing: s.list_editing,
        list_sel: s.list_sel,
        picker: s.picker.map(shadow_path_picker),
        providers: s
            .providers
            .into_iter()
            .map(|p| ProviderDraft {
                uuid: p.uuid,
                name: p.name,
                endpoint: p.endpoint,
                api_type: shadow_api_type(&p.api_type),
                api_key: p.api_key,
            })
            .collect(),
        prov_sel: s.prov_sel,
        prov_delete_armed: s.prov_delete_armed,
        prov_modal: s.prov_modal.map(|m| ProviderModal {
            name: m.name,
            endpoint: m.endpoint,
            api_type: shadow_api_type(&m.api_type),
            api_key: m.api_key,
            field: m.field,
        }),
        models: s
            .models
            .into_iter()
            .map(|m| ModelDraft {
                uuid: m.uuid,
                name: m.name,
                model_id: m.model_id,
                provider_idx: m.provider_idx,
                roles: m.roles.iter().map(|r| shadow_role(r)).collect(),
                route: m.route,
                session_only: m.session_only,
            })
            .collect(),
        model_sel: s.model_sel,
        model_delete_armed: s.model_delete_armed,
        model_modal: s.model_modal.map(shadow_model_modal),
    }
}

/// Rebuild the add/edit-model modal ([`ModelModal`]) from its projection. The
/// endpoints are reconstructed from the serde mirror back into [`ModelEndpoint`]
/// (a `Default`-padded copy carrying just the rendered fields).
fn shadow_model_modal(m: ModelModalSnapshot) -> ModelModal {
    ModelModal {
        editing_idx: m.editing_idx,
        uuid: m.uuid,
        name: m.name,
        provider_idx: m.provider_idx,
        model_id: m.model_id,
        field: m.field,
        roles: m.roles.iter().map(|r| shadow_role(r)).collect(),
        role_picker: m.role_picker.map(|rp| RolePickerState {
            checked: rp.checked,
            cursor: rp.cursor,
        }),
        query: m.query,
        result_sel: m.result_sel,
        route: m.route,
        route_sel: m.route_sel,
        endpoints: m.endpoints.map(|eps| {
            eps.into_iter()
                .map(|ep| ModelEndpoint {
                    name: ep.name,
                    provider_name: ep.provider_name,
                    pricing: Some(ModelPricing {
                        prompt: ep.price_prompt,
                        completion: ep.price_completion,
                    }),
                    context_length: None,
                    quantization: None,
                    max_completion_tokens: None,
                    uptime_last_30m: ep.uptime_last_30m,
                    status: None,
                })
                .collect()
        }),
        endpoints_loading: m.endpoints_loading,
        endpoints_for: m.endpoints_for,
    }
}

/// Rebuild the FS directory picker overlay ([`PathPicker`]) from its projection.
///
/// The matches are the daemon's already-computed `read_dir` results, used VERBATIM
/// (the client never walks its own filesystem — its cwd is unrelated to the
/// daemon's session). Constructed as a struct literal rather than via
/// `PathPicker::new`, which would re-run `list_dirs` against the local FS.
pub(crate) fn shadow_path_picker(p: PathPickerSnapshot) -> PathPicker {
    PathPicker {
        query: p.query,
        matches: p.matches,
        sel: p.sel,
        mode: match p.replace_idx {
            None => PickerMode::Add,
            Some(i) => PickerMode::Replace(i),
        },
    }
}

// ─── mode reconstruction (stage 3: secondary full-screen views) ──────────────

/// Rebuild the `--resume` session picker ([`PickerState`]) from its projection.
///
/// Constructed as a struct literal (NOT `PickerState::new`, which would re-run the
/// filter against a freshly-discovered local session list): the daemon's `all`
/// metadata + the `filtered_idx` it computed are carried verbatim so the SAME rows
/// render. Each row's `PathBuf` (the daemon-side load target) is rebuilt empty — the
/// client never loads it (Enter is forwarded), and the picker view doesn't render it.
pub(crate) fn shadow_picker(p: PickerSnapshot) -> PickerState {
    PickerState {
        query: p.query,
        all: p
            .all
            .into_iter()
            .map(|m| SessionMeta {
                id: m.id,
                name: m.name,
                path: std::path::PathBuf::new(), // daemon-side load target; not rendered
                modified: std::time::UNIX_EPOCH + Duration::from_secs(m.modified_secs),
                message_count: m.message_count,
                locked: m.locked,
            })
            .collect(),
        filtered_idx: p.filtered_idx,
        selected: p.selected,
    }
}

/// Rebuild the `/effort` reasoning-effort picker ([`EffortPickerState`]) from its
/// projection (all plain data the overlay reads).
pub(crate) fn shadow_effort(e: EffortSnapshot) -> EffortPickerState {
    EffortPickerState {
        options: e.options,
        selected: e.selected,
        note: e.note,
    }
}

/// Rebuild the `/usage` dashboard nav state ([`UsageNavState`]) from its wire tokens.
/// The dashboard's DATA is seeded separately into `rest.usage_data` (it crosses on the
/// same `UsageSnapshot`), so this only restores the view/range/metric selections.
pub(crate) fn shadow_usage_nav(view: &str, range: &str, metric: &str) -> UsageNavState {
    UsageNavState {
        view: match view {
            "session" => UsageView::Session,
            _ => UsageView::Global,
        },
        range: match range {
            "week" => UsageRange::Week,
            "year" => UsageRange::Year,
            _ => UsageRange::Today,
        },
        metric: match metric {
            "tokens" => UsageMetric::Tokens,
            _ => UsageMetric::Cost,
        },
    }
}

/// Rebuild the message-rewind picker ([`RewindState`]) from its projection — the
/// newest-first entry list + the cursor.
pub(crate) fn shadow_rewind(rw: RewindSnapshot) -> RewindState {
    RewindState {
        entries: rw
            .entries
            .into_iter()
            .map(|e| RewindEntry {
                vec_index: e.vec_index,
                content: e.content,
            })
            .collect(),
        selected: rw.selected,
    }
}

/// Rebuild the `/agents` dashboard ([`AgentsState`]) from its projection.
///
/// Restores the agent list, the working drafts + sub-mode + field cursor (from wire
/// tokens), the three overlays, and a minimal `session_dir` (empty — the client never
/// saves). The KEYLESS model+provider catalogue is folded into `rest.config` by the
/// caller's `shadow_settings`-style path? No — it is reconstructed HERE into a private
/// `AppConfig` the agents view resolves the model label against, so the client renders
/// `name @ provider` exactly as the daemon would WITHOUT any API key. The reconstructed
/// state is render-only; key handling is forwarded to the daemon.
pub(crate) fn shadow_agents(a: AgentsSnapshot) -> AgentsState {
    AgentsState {
        agents: a.agents.into_iter().map(|e: AgentEntry| {
            crate::model::agent_def::AgentDef {
                name: e.name,
                description: e.description,
                conditions: e.conditions,
                source: match e.source.as_str() {
                    "global"  => crate::model::agent_def::AgentSource::Global,
                    "builtin" => crate::model::agent_def::AgentSource::Builtin,
                    _         => crate::model::agent_def::AgentSource::Session,
                },
                model_uuid: e.model_uuid,
                model: e.model,
                tools: e.tools,
                prompt: e.prompt,
                file_path: None,
                ..crate::model::agent_def::AgentDef::default()
            }
        }).collect(),
        list_sel: a.list_sel,
        in_detail: a.in_detail,
        mode: match a.mode.as_str() {
            "edit" => AgentSubMode::Edit,
            "create" => AgentSubMode::Create,
            "delete_confirm" => AgentSubMode::DeleteConfirm,
            _ => AgentSubMode::Browse,
        },
        field: shadow_agent_field(&a.field),
        editing: a.editing,
        create_scope: match a.create_scope.as_str() {
            "global" => AgentScope::Global,
            _ => AgentScope::Session,
        },
        draft_name: a.draft_name,
        draft_description: a.draft_description,
        draft_conditions: a.draft_conditions,
        draft_model_uuid: a.draft_model_uuid,
        draft_model_legacy: a.draft_model_legacy,
        draft_tools: a.draft_tools,
        draft_body: a.draft_body,
        // The session dir is the daemon-side save target; the client never saves, and
        // the view doesn't render it, so an empty path is fine.
        session_dir: std::path::PathBuf::new(),
        tool_picker: a.tool_picker.map(shadow_tool_picker),
        model_picker: a.model_picker.map(shadow_agent_model_picker),
        editor: a
            .editor
            .map(|(field, ed)| (shadow_agent_field(&field), shadow_text_editor(ed))),
        editor_clear_confirm: a.editor_clear_confirm,
    }
}

/// Rebuild the `/agents` tool multi-select picker ([`ToolPickerState`]).
fn shadow_tool_picker(p: ToolPickerSnapshot) -> ToolPickerState {
    ToolPickerState {
        options: p.options,
        checked: p.checked,
        cursor: p.cursor,
        filter: p.filter,
    }
}

/// Rebuild the `/agents` single-select model picker ([`ModelPickerState`]).
fn shadow_agent_model_picker(p: AgentModelPickerSnapshot) -> ModelPickerState {
    ModelPickerState {
        options: p.options,
        cursor: p.cursor,
    }
}

/// Rebuild the full-screen nano editor ([`TextEditorState`]) from its projection. The
/// render-published `wrap_w` cell is re-seeded to `usize::MAX` (its `from_text`
/// default), so before the first client frame every line is one segment — exactly the
/// editor's own safe fallback; the next draw publishes the real width.
fn shadow_text_editor(ed: TextEditorSnapshot) -> TextEditorState {
    TextEditorState {
        lines: ed.lines,
        row: ed.row,
        col: ed.col,
        scroll: ed.scroll,
        wrap_w: std::cell::Cell::new(usize::MAX),
    }
}

/// Map an `/agents` field wire token back to an [`AgentEditField`] (unknown →
/// Description, the editor's default focus — never lost).
fn shadow_agent_field(f: &str) -> AgentEditField {
    match f {
        "name" => AgentEditField::Name,
        "conditions" => AgentEditField::Conditions,
        "model" => AgentEditField::Model,
        "tools" => AgentEditField::Tools,
        "prompt" => AgentEditField::Body,
        _ => AgentEditField::Description,
    }
}

/// Map a theme wire token back to a [`ThemeMode`] (unknown → Dark).
pub(crate) fn shadow_theme(t: &str) -> ThemeMode {
    match t {
        "light" => ThemeMode::Light,
        _ => ThemeMode::Dark,
    }
}

/// Map an internet-mode wire token back to an [`InternetMode`] (unknown → Simple).
pub(crate) fn shadow_internet_mode(t: &str) -> InternetMode {
    match t {
        "full" => InternetMode::Full,
        _ => InternetMode::Simple,
    }
}

/// Map an api-type wire token back to an [`ApiType`] (unknown → OpenAiCompatible).
pub(crate) fn shadow_api_type(t: &str) -> ApiType {
    match t {
        "anthropic" => ApiType::AnthropicCompatible,
        _ => ApiType::OpenAiCompatible,
    }
}

/// Map a role wire token back to a [`ModelRole`] (unknown → Main, never lost).
fn shadow_role(r: &str) -> ModelRole {
    match r {
        "awareness" => ModelRole::Awareness,
        "safeguard" => ModelRole::Safeguard,
        "compactor" => ModelRole::Compactor,
        _ => ModelRole::Main,
    }
}
