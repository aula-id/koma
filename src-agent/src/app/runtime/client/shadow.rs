use std::time::{Duration, Instant};

use crate::app::state::{AppState, SessionRuntime};
use crate::ipc::proto::{ClientRequest, DaemonEvent, DaemonFrame, StateDelta, StateSnapshot};

use crate::app::runtime::client_shadow::*;

use super::render::TOAST_TTL;

/// Apply one incoming [`DaemonFrame`] to the shadow, handling seq-gap recovery.
///
/// Returns `true` if the shadow changed and a redraw is needed. On a detected gap
/// it sends [`ClientRequest::Resync`], sets `awaiting_resync`, and drops the frame;
/// while `awaiting_resync` only a fresh `Snapshot` is applied (it reseeds the seq +
/// clears the flag). `Ack` / `Error` frames advance the seq but are non-visual
/// (an `Error` could surface as a toast in a later refinement).
pub(super) fn apply_frame(
    frame: DaemonFrame,
    shadow: &mut AppState,
    expected: &mut u64,
    seeded: &mut bool,
    awaiting_resync: &mut bool,
    select_requested: &mut bool,
    req_tx: &std::sync::mpsc::Sender<ClientRequest>,
) -> bool {
    // --- seq-gap detection (critique #1) ---
    if !*seeded {
        // First frame ever: seed the expectation from it (whatever it is) so we
        // don't false-positive a gap on the initial Snapshot's seq.
        *seeded = true;
        *expected = frame.seq;
    } else if frame.seq != *expected {
        // A frame was dropped (or reordered). Ask for a full rebuild and ignore
        // everything until the fresh Snapshot arrives — UNLESS this very frame is a
        // Snapshot, which is itself a valid full rebuild we can take right now.
        if matches!(frame.event, DaemonEvent::Snapshot(_)) {
            // Fall through to apply it; it reseeds the seq below.
        } else {
            if !*awaiting_resync {
                *awaiting_resync = true;
                let _ = req_tx.send(ClientRequest::Resync);
            }
            // Resync the expectation forward so we don't spam Resync on every
            // subsequent gapped frame; the awaited Snapshot will reseed precisely.
            *expected = frame.seq.wrapping_add(1);
            return false;
        }
    }
    // Next frame should be exactly one past this one.
    *expected = frame.seq.wrapping_add(1);

    match frame.event {
        DaemonEvent::Snapshot(snap) => {
            // A full Snapshot is always a valid rebuild — it clears any pending
            // resync and reseeds the shadow wholesale. (`snap` is boxed on the wire
            // to keep the `DaemonEvent` enum small; unbox it for `apply_snapshot`.)
            *awaiting_resync = false;
            apply_snapshot(shadow, *snap);
            true
        }
        DaemonEvent::Delta(delta) => {
            // Drop deltas while the shadow is known-stale (waiting on the resync
            // Snapshot) — applying them onto a wrong baseline would corrupt it.
            if *awaiting_resync {
                return false;
            }
            apply_delta(shadow, delta)
        }
        // The controller's `/select` hand-off: the daemon (which owns no TTY) asks
        // THIS client to run the transcript dump on its own terminal. The dump leaves
        // the alt-screen + blocks on a keypress, which can't happen here (no `terminal`
        // handle, mid-frame-drain); just latch the request so the render loop performs
        // it after this drain pass completes. Non-visual to the shadow itself.
        DaemonEvent::EnterSelect => {
            *select_requested = true;
            false
        }
        // The build-skew handshake frame (task #142) is consumed BEFORE the render
        // loop, in the pre-render handshake (see `client_run`). If one still reaches
        // here (a re-attach mid-session re-emits it), it is non-visual: the version was
        // already verified at connect time, so just advance the seq and render nothing.
        DaemonEvent::Hello { .. } => false,
        // Non-visual control replies. (A future refinement could toast an Error.)
        DaemonEvent::Ack | DaemonEvent::Error(_) => false,
    }
}

/// Rebuild the entire shadow [`AppState`] from a full [`StateSnapshot`].
///
/// Replaces `rest.sessions` with one reconstructed [`SessionRuntime`] per snapshot
/// session, points `foreground` at the `foreground_id`, and copies the global
/// fields. The transcript-render cache is cleared because a snapshot can replace the
/// committed history wholesale (e.g. a foreground switch to a different session) and
/// the cache's incremental-append guard only covers a pure shrink, not a divergence.
pub(super) fn apply_snapshot(shadow: &mut AppState, snap: StateSnapshot) {
    use crate::app::mode::{Mode, QuitConfirmState};
    use crate::app::state::AgentMode;
    use crate::ipc::proto::{ModeSnapshot, UsageSnapshot};
    use crate::model::app_config::{ModelEntry, ProviderConn};

    let StateSnapshot {
        foreground_id,
        sessions,
        global,
    } = snap;

    // Rebuild every session runtime from its projection.
    let runtimes: Vec<SessionRuntime> = sessions.iter().map(shadow_session_runtime).collect();

    // Resolve the foreground index by stable id (never trust an index across the
    // wire). Fall back to 0 if the id is somehow absent — `sessions` is always >=1.
    let fg = foreground_id
        .as_deref()
        .and_then(|id| sessions.iter().position(|s| s.id == id))
        .unwrap_or(0);

    shadow.rest.sessions = if runtimes.is_empty() {
        // Defensive: never leave `sessions` empty (fg()/fg_mut() index it). The
        // daemon always projects >=1 session, so this is belt-and-suspenders.
        vec![SessionRuntime::new()]
    } else {
        runtimes
    };
    shadow.rest.foreground = fg.min(shadow.rest.sessions.len() - 1);

    // Composer + transcript-view fields now live on the foreground session
    // (per-session in C1; still the single global foreground), so copy them onto
    // the shadow's foreground runtime via `fg_mut()`. `status` stays rest-global.
    {
        let fg = shadow.rest.fg_mut();
        fg.input = global.input;
        fg.cursor = global.cursor;
        fg.scroll = global.scroll;
        fg.follow = global.follow;
    }
    shadow.rest.status = global.status;
    shadow.rest.toast = global.toast.map(|(kind, text)| {
        (text, Instant::now() + TOAST_TTL, toast_kind(&kind))
    });
    // The on-demand model catalogue + the endpoint it was fetched for. The Settings
    // model modal + KeyInput step-1 search render their omnisearch dropdowns from
    // these; without them a remote client's dropdown would sit on `searching
    // models…` forever (it has no fetch path of its own).
    shadow.rest.models_cache = global.models_cache;
    shadow.rest.models_cache_endpoint = global.models_cache_endpoint;

    // Global theme + accent. `view::draw` frames every screen via
    // `theme::palette(&shadow.rest.config)` BEFORE dispatching to a mode renderer, so
    // without these the shadow's `config` stays at `AppConfig::default()` (Dark/green)
    // and a Light-theme or non-green daemon renders every label/border/highlight in
    // the wrong palette. Theme decodes from its wire token (reusing the Settings
    // helper, unknown → Dark); accent is an opaque palette key copied verbatim.
    shadow.rest.config.theme = shadow_theme(&global.theme);
    shadow.rest.config.accent = global.accent;
    // Agent mode: decode from the wire token so the header reflects the current mode.
    // "yolo" must be decoded explicitly — falling to the `_ => Auto` default would
    // silently drop the loud-red Yolo header on the thin client.
    shadow.rest.agent_mode = match global.agent_mode.as_str() {
        "normal" => AgentMode::Normal,
        "yolo"   => AgentMode::Yolo,
        _        => AgentMode::Auto,
    };
    shadow.rest.latest_version = global
        .latest_version
        .as_ref()
        .map(|version| crate::app::version::VersionInfo { version: version.clone(), message: None });
    // The shadow `AppConfig`'s registered-model + provider catalogue is populated
    // ONLY for the `/agents` screen (which resolves a chosen `model_uuid` to a
    // `name @ provider` label off `rest.config`), from that mode's KEYLESS projection.
    // Reset it here every snapshot so a stale catalogue from a previous Agents view
    // never lingers into another screen; the Agents arm below refills it when active.
    shadow.rest.config.models.clear();
    shadow.rest.config.providers.clear();

    // Full-screen sub-agent viewer + `$` panel state (rendered FROM Chat mode off the
    // foreground session's reconstructed `subagents`). Mirror the daemon's
    // `agent_viewer` index / scroll / follow + the panel open-state + selection so the
    // unmodified chat renderer takes the same full-screen-viewer / overlay branch.
    shadow.rest.agent_viewer = global.agent_viewer;
    shadow.rest.agent_viewer_scroll = global.agent_viewer_scroll;
    shadow.rest.agent_viewer_follow = global.agent_viewer_follow;
    shadow.rest.subagents_open = global.subagents_open;
    shadow.rest.subagent_sel = global.subagent_sel;
    // The `@`-file / `/`-command picker highlighted-row index — mirrored like
    // `subagent_sel` so Up/Down on either picker moves the highlight on the client
    // (without this the shadow `palette_sel` stays at 0 and the row never moves).
    shadow.rest.palette_sel = global.palette_sel;

    // Staged composer attachments (ingested daemon-side via path-paste / clipboard /
    // @-picker). The `[Image #N]` marker text already arrives in `input`; mirror the
    // attachment RECORDS too so the shadow composer matches the daemon's exactly.
    // Lives on the foreground session (alongside `input`) now.
    shadow.rest.fg_mut().pending_attachments = global.pending_attachments;
    // The precomputed `@`-file palette (the daemon ran `dir_cache.search` on its
    // index). The client's reconstructed `dir_cache` is empty, so the unmodified
    // file-palette view renders this projected list instead (see
    // `view::chat::render_file_palette`). `None` when the composer isn't on an
    // `@token`; seeding it every snapshot (including with `None`) means a stale list
    // never lingers after the `@token` is completed/cleared.
    shadow.rest.file_palette = global.file_palette;

    // Re-anchor the comet clock from the projected elapsed-ms (authoritative for
    // this snapshot). `work_since = now - elapsed` makes the status shimmer animate
    // from the SAME phase + elapsed-seconds the daemon is at, rather than restarting
    // at 0 each snapshot. `None` (idle) clears it. This REPLACES the old derive-from-
    // working-flag reconcile on the snapshot path; the delta path still reconciles
    // approximately (a working flip there means work just began, so `now` is right).
    shadow.rest.work_since = global
        .work_elapsed_ms
        .map(|ms| Instant::now() - Duration::from_millis(ms));

    // The committed history may have changed wholesale; drop the rendered-lines
    // cache so the next draw rebuilds it against the new messages.
    shadow.rest.transcript_cache.borrow_mut().blocks.clear();

    // Mode: reconstruct from the pure-data `ModeSnapshot` into REAL mode state so the
    // unmodified `view::draw` renders every screen faithfully — the client never
    // mutates these (input is forwarded), it only needs enough to DRAW. Chat is
    // payload-free. The QuitConfirm overlay is special-cased so the client can ALSO
    // intercept its lifecycle keys ([d] detach / [k] kill-all) locally (see
    // `render_loop`). With stage 3 EVERY variant carries its payload, so nothing falls
    // back to a blank Chat render any more.
    //
    // The `/usage` dashboard is special: its numbers come from the daemon's ledger,
    // which the client cannot read, so the projection carries the pre-fetched data —
    // seed `rest.usage_data` from it so the unmodified dashboard renders DB-free.
    // Clear it first so a stale projection never lingers into the next non-Usage mode.
    shadow.rest.usage_data = None;
    // Mode lives on the shadow's FOREGROUND session now (C3), reached via `set_mode` (which
    // borrows `shadow.rest`). Several arms ALSO write `shadow.rest.*` (the Agents catalogue,
    // the Usage `usage_data`), which would overlap a direct `*shadow.mode_mut() = …` borrow.
    // So build the mode into a local — the arms keep their `shadow.rest` writes — then store
    // it onto the foreground session with `set_mode`.
    let new_mode = match global.mode {
        ModeSnapshot::KeyInput(f) => Mode::KeyInput(shadow_key_input(f)),
        ModeSnapshot::SessionPicker(p) => Mode::SessionPicker(shadow_picker(p)),
        ModeSnapshot::SessionHub(h) => Mode::SessionHub(Box::new(shadow_session_hub(h))),
        ModeSnapshot::Chat => Mode::Chat,
        ModeSnapshot::Loading(s) => Mode::Loading(shadow_loading(s)),
        ModeSnapshot::Settings(s) => Mode::Settings(Box::new(shadow_settings(*s))),
        ModeSnapshot::Agents(a) => {
            // Seed the shadow config's KEYLESS catalogue so the agents view resolves
            // the model label (`name @ provider`) off `rest.config`, exactly as the
            // daemon does — without any API key (the reconstructed providers carry an
            // empty `api_key`; the client only resolves labels, never sends a request).
            shadow.rest.config.models = a
                .catalogue_models
                .iter()
                .map(|m| ModelEntry {
                    uuid: m.uuid.clone(),
                    name: m.name.clone(),
                    model_id: m.model_id.clone(),
                    provider_uuid: m.provider_uuid.clone(),
                    ..ModelEntry::default()
                })
                .collect();
            shadow.rest.config.providers = a
                .catalogue_providers
                .iter()
                .map(|p| ProviderConn {
                    uuid: p.uuid.clone(),
                    name: p.name.clone(),
                    endpoint: p.endpoint.clone(),
                    ..ProviderConn::default()
                })
                .collect();
            Mode::Agents(Box::new(shadow_agents(*a)))
        }
        ModeSnapshot::Mcp(m) => Mode::Mcp(Box::new(shadow_mcp(*m))),
        ModeSnapshot::Security(s) => Mode::Security(Box::new(shadow_security(*s))),
        ModeSnapshot::Bash(b) => Mode::Bash(Box::new(shadow_bash(*b))),
        ModeSnapshot::Help(h) => Mode::Help(Box::new(shadow_help(*h))),
        ModeSnapshot::Effort(e) => Mode::Effort(Box::new(shadow_effort(e))),
        ModeSnapshot::Usage(u) => {
            let UsageSnapshot { view, range, metric, data } = *u;
            shadow.rest.usage_data = Some(data);
            Mode::Usage(Box::new(shadow_usage_nav(&view, &range, &metric)))
        }
        ModeSnapshot::MessageRewind(rw) => Mode::MessageRewind(Box::new(shadow_rewind(rw))),
        ModeSnapshot::QuitConfirm { working, total, selected } => {
            // Rebuild the overlay state and restore the daemon-owned focus index.
            // `new` defaults `selected` to 2 (the safe cancel); overwrite it with the
            // projected value so arrow/Tab navigation — which mutates `selected` on the
            // daemon — is reflected on the next frame. `button_rects` stays default
            // (Rect::ZERO); the client's draw recomputes the hit-boxes every frame.
            let mut st = QuitConfirmState::new(working, total);
            st.selected = selected;
            Mode::QuitConfirm(Box::new(st))
        }
    };
    // Apply the rebuilt mode onto the shadow's foreground session (the snapshot's mode is
    // the daemon's foreground-session mode, projected at `fg().mode`; C3).
    shadow.set_mode(new_mode);

    // NOTE: the comet clock (`work_since`) was already set authoritatively from the
    // snapshot's `work_elapsed_ms` above, so it is deliberately NOT reconciled here
    // (re-deriving would discard the precise daemon-anchored phase).
}

/// Apply one incremental [`StateDelta`] to the shadow in place.
///
/// Returns `true` if the shadow changed. Session-scoped deltas resolve their target
/// by stable id (never index); an unknown id is a harmless no-op (the next Snapshot
/// reconciles). The differ only emits these for high-frequency per-tick changes;
/// anything structural (history, tokens, approval, sub-agents, the session set)
/// arrives as a full Snapshot instead (see `ipc::snapshot::diff`).
pub(super) fn apply_delta(shadow: &mut AppState, delta: StateDelta) -> bool {
    match delta {
        StateDelta::TokenAppended { session_id, text } => {
            if let Some(rt) = session_by_id_mut(shadow, &session_id) {
                // A token before any `streaming` buffer means the daemon went
                // None -> Some("…") this turn (the differ treats None/Some("") alike);
                // initialise the buffer so the append lands.
                rt.streaming.get_or_insert_with(String::new).push_str(&text);
                return true;
            }
            false
        }
        StateDelta::ReasoningAppended { session_id, text } => {
            if let Some(rt) = session_by_id_mut(shadow, &session_id) {
                rt.stream_reasoning.push_str(&text);
                return true;
            }
            false
        }
        StateDelta::StatusChanged { session_id, text } => match session_id {
            // Session-scoped status is not separately rendered today (the status line
            // is global); a `None` (global) status updates the rendered status line.
            None => {
                shadow.rest.status = text;
                true
            }
            Some(_) => false,
        },
        StateDelta::InputChanged { text, cursor } => {
            // The shared composer moved (typed/deleted a char, or the caret moved).
            // Carries the WHOLE input string, so replace wholesale; clamp the caret
            // into bounds defensively (the daemon sends a consistent pair, but the
            // composer renderer indexes by cursor and must never read past the end).
            // Composer now lives on the foreground session (single global fg in C1).
            let fg = shadow.rest.fg_mut();
            fg.input = text;
            fg.cursor = cursor.min(fg.input.chars().count());
            true
        }
        StateDelta::ScrollChanged { scroll, follow } => {
            // Global transcript view state moved on the daemon (a forwarded scroll
            // key, or new content re-pinning follow). Mirror it so the rendered
            // offset tracks the daemon between full snapshots. The renderer clamps
            // `scroll` against the live content height each draw, so an offset that
            // momentarily exceeds the shadow's shorter content is self-correcting.
            // Transcript view state now lives on the foreground session.
            let fg = shadow.rest.fg_mut();
            fg.scroll = scroll;
            fg.follow = follow;
            true
        }
        StateDelta::SessionStatusChanged {
            session_id,
            working,
            finished_unseen,
        } => {
            if let Some(rt) = session_by_id_mut(shadow, &session_id) {
                rt.waiting = working;
                rt.finished_unseen = finished_unseen;
                // The working flag feeds the comet clock; reconcile it (only the
                // foreground session's clock is rendered).
                reconcile_work_clock(shadow);
                return true;
            }
            false
        }
        StateDelta::ForegroundChanged { session_id } => {
            if let Some(idx) = shadow
                .rest
                .sessions
                .iter()
                .position(|s| s.id == session_id)
            {
                shadow.rest.foreground = idx;
                // Switching foreground swaps the visible transcript wholesale; clear
                // the rendered-lines cache so it rebuilds for the new session.
                shadow.rest.transcript_cache.borrow_mut().blocks.clear();
                reconcile_work_clock(shadow);
                return true;
            }
            false
        }
        StateDelta::SessionAdded(snap) => {
            // A new parallel session appeared. Append its runtime; the differ would
            // normally send a full Snapshot for a set change, but accept the delta
            // form too (it is in the vocabulary) so the shadow stays in step either way.
            if !shadow.rest.sessions.iter().any(|s| s.id == snap.id) {
                shadow.rest.sessions.push(shadow_session_runtime(&snap));
                // Clear the transcript cache since a new session may become foreground,
                // and the committed history can change wholesale on a foreground switch.
                shadow.rest.transcript_cache.borrow_mut().blocks.clear();
                // Reconcile the work clock to match the daemon's state with the new session
                // in place, so the comet animation stays in sync.
                reconcile_work_clock(shadow);
                return true;
            }
            // (`snap` is `Box<SessionSnapshot>`; `&snap` derefs to `&SessionSnapshot`.)
            false
        }
        StateDelta::Toast { kind, text } => {
            shadow.rest.toast = Some((text, Instant::now() + TOAST_TTL, toast_kind(&kind)));
            true
        }
    }
}

/// Find a shadow session runtime by its stable id (mutable).
pub(super) fn session_by_id_mut<'a>(
    shadow: &'a mut AppState,
    id: &str,
) -> Option<&'a mut SessionRuntime> {
    shadow.rest.sessions.iter_mut().find(|s| s.id == id)
}

/// Re-derive the local "comet" animation clock from the FOREGROUND session's working
/// state, mirroring the rising/falling-edge reconcile the daemon/TUI loop does.
///
/// The status-line shimmer renders only when `work_since` is set. The daemon's own
/// `work_since` is daemon-local and not projected (it's a pure animation clock), so
/// the client maintains its own: set it the moment the foreground session is working
/// (and not paused for approval) and it isn't already running; clear it the moment
/// work ends or an approval prompt takes over.
pub(super) fn reconcile_work_clock(shadow: &mut AppState) {
    let fg = shadow.rest.fg();
    let shimmer = fg.waiting && !fg.awaiting_approval;
    if shimmer {
        if shadow.rest.work_since.is_none() {
            shadow.rest.work_since = Some(Instant::now());
        }
    } else {
        shadow.rest.work_since = None;
    }
}
