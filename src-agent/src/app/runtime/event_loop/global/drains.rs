//! Channel/network GLOBAL drains extracted from [`super::service_global`] for
//! file size — each function is exactly one of the independent channel-drain
//! blocks the driver used to inline, in the SAME order, with the same locals
//! threaded as parameters. `pub(super)` so they cross the `global::drains` ->
//! `global` module boundary without leaking further; no behaviour change.
//!
//! The redraw-facing drains (clipboard, loading splash, deferred compact,
//! workspace warning, shimmer, toast tick) live in the sibling [`super::ui`]
//! module instead — this file keeps the ones that drain a background
//! channel/network result (endpoints, version, security health, OAuth,
//! awareness, startup-warming, the debounced catalogue fetch) plus their two
//! spinner-advance twins and the shared de-globalization helpers.

use std::sync::Arc;

use crate::app::mode::settings::OAuthFlowState;
use crate::app::mode::{Mode, OnboardProviderState, OnboardProviderStep, WarmStatus};
use crate::app::state::AppState;
use crate::service::oauth::OAuthEvent;
use crate::service::{openrouter::OpenRouterClient, StreamEvent, WarmEvent};

/// Drain the per-model provider-endpoints channel. Fully independent of
/// streaming and the harness channel: the background fetch sends exactly one
/// EndpointsLoaded / EndpointsError, folded into the open model modal — but
/// ONLY when its `model_id` still matches the modal's `endpoints_for` (the
/// stale-guard, so a rapid re-selection can't show a previous model's
/// providers). Take() the receiver so the match can mutate the mode; put it
/// back unless the fetch resolved (or the channel closed).
pub(super) fn drain_endpoints(state: &mut AppState) -> bool {
    let mut dirty = false;
    if let Some(mut erx) = state.rest.endpoints_rx.take() {
        let mut keep = true;
        while let Ok(ev) = erx.try_recv() {
            match ev {
                StreamEvent::EndpointsLoaded { model_id, endpoints } => {
                    // De-globalized (C3): mode is per-session and `service_global` runs
                    // OUTSIDE any client bracket, so the foreground cursor is stale scratch
                    // here. Fold the result into WHICHEVER session(s) have a Settings model-
                    // modal awaiting THIS model's endpoints (matched by `endpoints_for`),
                    // not the (stale) foreground. A single fetch is in flight at a time, so
                    // in practice one session matches; iterating keeps it index-correct.
                    apply_to_settings_modal_for(state, &model_id, |m| {
                        m.endpoints = Some(endpoints.clone());
                        m.endpoints_loading = false;
                    });
                    dirty = true;
                    keep = false;
                }
                StreamEvent::EndpointsError { model_id, .. } => {
                    apply_to_settings_modal_for(state, &model_id, |m| {
                        // Empty list => "no providers found" display.
                        m.endpoints = Some(Vec::new());
                        m.endpoints_loading = false;
                    });
                    dirty = true;
                    keep = false;
                }
                _ => {}
            }
        }
        if keep {
            state.rest.endpoints_rx = Some(erx);
        }
    }
    dirty
}

/// Drain the background version-check channel. Each session spawn fires a
/// non-blocking `spawn_check` thread that, on success, sends one `VersionInfo`;
/// a failed/unreachable check sends nothing (graceful degrade). Fold the LATEST
/// received result into `latest_version` for the UI to read. Take() the receiver
/// to mutate `rest`, then ALWAYS put it back: the matching sender lives in
/// `version_tx` for the app's lifetime, so the channel never closes — there is no
/// `Disconnected` terminal state to drop the receiver on. Non-blocking (try_recv).
pub(super) fn drain_version(state: &mut AppState) -> bool {
    let mut dirty = false;
    if let Some(mut vrx) = state.rest.version_rx.take() {
        while let Ok(info) = vrx.try_recv() {
            state.rest.latest_version = Some(info);
            dirty = true;
        }
        state.rest.version_rx = Some(vrx);
    }
    dirty
}

/// Drain the NON-BLOCKING security health probe (mirrors the `version_rx` drain). A
/// `SecDaemonManager::health_async` fetch sends exactly one result: Ok(entries) on a
/// successful probe, Ok(Err(msg)) when the daemon reported/timed-out an error. Fold a
/// success into the OPEN `/security` panel's `install_health` and clear the spinner;
/// toast an error. Take() the receiver so the arms can mutate `state.mode`; put it back
/// only while still Empty (a delivered result OR a closed channel ends the probe). On
/// any terminal outcome the spinner flag is cleared so a panel that is open stops
/// animating. Non-blocking (try_recv).
pub(super) fn drain_sec_health(state: &mut AppState) -> bool {
    let mut dirty = false;
    if let Some(mut hrx) = state.rest.sec_health_rx.take() {
        match hrx.try_recv() {
            Ok(Ok(health)) => {
                // De-globalized (C3): apply to whichever session(s) have the `/security`
                // panel open, not the (stale outside a client bracket) foreground.
                for s in security_states(state) {
                    s.install_health = health.clone();
                    s.health_fetching = false;
                }
                // Receiver consumed (one-shot result delivered) → don't put it back.
                dirty = true;
            }
            Ok(Err(e)) => {
                // Toast is per-session (C6); a health-probe failure is a global notice —
                // surface it on the foreground session (single-window: the only session).
                state.rest.fg_mut().set_toast(format!("security health probe failed: {e}"));
                for s in security_states(state) {
                    s.health_fetching = false;
                }
                // Receiver consumed → don't put it back.
                dirty = true;
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                // Still in flight — keep the receiver for the next tick.
                state.rest.sec_health_rx = Some(hrx);
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                // Sender dropped without sending (shouldn't happen — the spawn always
                // sends — but stay clean): end the probe, clear the spinner.
                for s in security_states(state) {
                    s.health_fetching = false;
                }
                dirty = true;
            }
        }
    }
    dirty
}

/// Drain the `/settings` OAuth submenu's connect-flow channel (mirrors
/// `sec_health_rx`): a spawned Codex/Kilo Code flow sends `CodexUrl`/`KiloCode`
/// once it has something to show, then exactly one terminal event (`Success` or
/// `Failed`). Non-terminal events swap `oauth_flow` to the matching wait screen
/// and put the receiver back; a terminal event applies the result and ends the
/// flow (task handle cleared). De-globalized (C3): fold into whichever
/// session(s) actually have the OAuth submenu open, not the (stale outside a
/// client bracket) foreground.
pub(super) fn drain_oauth(state: &mut AppState, handle: &tokio::runtime::Handle) -> bool {
    let mut dirty = false;
    if let Some(mut orx) = state.rest.oauth_rx.take() {
        match orx.try_recv() {
            Ok(OAuthEvent::CodexUrl { url }) => {
                for flow in oauth_flow_states(state) {
                    *flow = OAuthFlowState::CodexWait { url: url.clone(), frame: 0, copied: false };
                }
                state.rest.oauth_rx = Some(orx);
                dirty = true;
            }
            Ok(OAuthEvent::KiloCode { user_code, verification_url }) => {
                for flow in oauth_flow_states(state) {
                    *flow = OAuthFlowState::KiloWait {
                        user_code: user_code.clone(),
                        verification_url: verification_url.clone(),
                        frame: 0,
                        copied: false,
                    };
                }
                state.rest.oauth_rx = Some(orx);
                dirty = true;
            }
            Ok(OAuthEvent::Success { conn }) => {
                // Immediate persist: seed the refresh-token cache, append to the
                // catalogue, save `config.json`. A save failure surfaces as `Failed`
                // instead of silently losing the just-completed login. Capture the new
                // conn's identity BEFORE the move so the wizard fold below can bind to it.
                let seeded = conn.clone();
                handle.spawn(async move {
                    crate::service::oauth::manager::seed(&seeded).await;
                });
                let conn_uuid = conn.uuid.clone();
                let conn_provider = conn.provider;
                let conn_token = conn.access_token.clone();
                state.rest.config.oauth_conns.push(conn);
                let save_err = state.rest.config.save().err().map(|e| e.to_string());
                let drafts = crate::app::mode::settings::OAuthDraft::from_config(&state.rest.config);
                // Settings sessions: rebuild `oauth_drafts` + return to the list (or
                // surface a save error). Unchanged behaviour.
                for st in settings_states(state) {
                    match &save_err {
                        Some(e) => {
                            st.oauth_flow = OAuthFlowState::Failed(format!(
                                "login ok but config write failed: {e}"
                            ));
                        }
                        None => {
                            st.oauth_drafts = drafts.clone();
                            st.oauth_flow = OAuthFlowState::Idle;
                        }
                    }
                }
                // Guided-wizard sessions: advance to the model picker bound to the new
                // conn (or surface a save error).
                let mut advanced = false;
                for op in onboard_provider_states(state) {
                    match &save_err {
                        Some(e) => {
                            op.oauth_flow = OAuthFlowState::Failed(format!(
                                "login ok but config write failed: {e}"
                            ));
                        }
                        None => {
                            op.new_conn_uuid = conn_uuid.clone();
                            op.provider = Some(conn_provider);
                            op.step = OnboardProviderStep::ModelSelect;
                            op.oauth_flow = OAuthFlowState::Idle;
                            op.query.clear();
                            op.result_sel = 0;
                            advanced = true;
                        }
                    }
                }
                // Prime the network catalogue so ModelSelect can filter immediately
                // (Codex serves its static list, so it needs no fetch). Done AFTER the
                // `onboard_provider_states` borrow ends.
                if advanced && save_err.is_none() && !crate::service::oauth::registry::meta(conn_provider).catalogue_endpoint.is_empty() {
                    let ep = crate::service::oauth::registry::meta(conn_provider).catalogue_endpoint;
                    state.rest.request_catalogue(ep, &conn_token, &conn_uuid);
                }
                state.rest.oauth_task = None;
                dirty = true;
                // Receiver consumed (terminal event) — don't put it back.
            }
            Ok(OAuthEvent::Failed { error }) => {
                for flow in oauth_flow_states(state) {
                    *flow = OAuthFlowState::Failed(error.clone());
                }
                state.rest.oauth_task = None;
                dirty = true;
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                // Still in flight — keep the receiver for the next tick.
                state.rest.oauth_rx = Some(orx);
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                // Sender dropped without a terminal event (task panicked/aborted
                // mid-flight) — surface it rather than leaving the wait screen
                // spinning forever.
                for flow in oauth_flow_states(state) {
                    if !matches!(flow, OAuthFlowState::Idle) {
                        *flow =
                            OAuthFlowState::Failed("login flow ended unexpectedly".to_string());
                    }
                }
                state.rest.oauth_task = None;
                dirty = true;
            }
        }
    }
    dirty
}

/// Drain the dedicated awareness-recompute channel (`cd` / post-`/compact`),
/// mirroring the `sec_health_rx` drain just above. Distinct from `warm_rx`:
/// that channel is REPLACED per warm, so a recompute in flight when a new warm
/// starts would be stranded — this pair is created once and kept for the app's
/// lifetime. Route each `(session_id, summary)` by id (same C4 pattern as
/// `WarmEvent::WarmAwareness` below) since `service_global` runs outside any
/// client bracket and the foreground cursor is stale scratch here. Loop (not a
/// single `try_recv`) so a burst of recomputes (e.g. several quick `cd`s)
/// doesn't lag a tick behind. Non-blocking (try_recv).
pub(super) fn drain_awareness(state: &mut AppState) -> bool {
    let mut dirty = false;
    if let Some(mut arx) = state.rest.awareness_rx.take() {
        let mut keep = true;
        loop {
            match arx.try_recv() {
                Ok((session_id, summary)) => {
                    if let Some(s) = state.rest.sessions.iter_mut().find(|s| s.id == session_id) {
                        s.awareness_summary = summary;
                    }
                    // Session gone (closed since the recompute was spawned) → the
                    // result is simply dropped, same contract as `WarmAwareness`.
                    dirty = true;
                }
                // Channel drained for now: keep listening on later ticks.
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                // Sender dropped without sending — shouldn't happen (the spawn
                // always sends), but stay clean and stop polling.
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    keep = false;
                    break;
                }
            }
        }
        if keep {
            state.rest.awareness_rx = Some(arx);
        } else {
            // Also drop the paired sender so a future recompute reopens a fresh pair.
            state.rest.awareness_tx = None;
        }
    }
    dirty
}

/// Drain the startup-warming channel. Fully independent of streaming: the
/// background catalogue + awareness tasks each send one [`WarmEvent`]. ALWAYS
/// fold the result into `state.rest.*` (the cache / summary) regardless of the
/// current mode — a result that lands AFTER an Esc-to-chat must still populate
/// them — and update the live `LoadingState` step marker only while still in
/// `Mode::Loading`. Take() the receiver so the arms can mutate the mode + rest;
/// put it back unless the channel has closed (both warm tasks finished and
/// dropped their senders → `Disconnected`).
pub(super) fn drain_warm(state: &mut AppState) -> bool {
    let mut dirty = false;
    if let Some(mut wrx) = state.rest.warm_rx.take() {
        let mut keep = true;
        loop {
            match wrx.try_recv() {
                Ok(WarmEvent::WarmCatalogue { endpoint, models }) => {
                    // Key the on-demand cache to the endpoint it was fetched for;
                    // the omnisearch filters locally only while
                    // `models_cache_endpoint` matches the active endpoint.
                    state.rest.models_cache = Some(models);
                    state.rest.models_cache_endpoint = Some(endpoint.clone());
                    // Clear any previous failure for this endpoint so a later
                    // `request_catalogue` call can re-fetch if needed.
                    if state.rest.models_cache_failed.as_deref() == Some(endpoint.as_str()) {
                        state.rest.models_cache_failed = None;
                    }
                    // Clear the in-flight guard for this endpoint so a later
                    // endpoint change can fetch again.
                    if state.rest.catalogue_fetching.as_deref() == Some(endpoint.as_str()) {
                        state.rest.catalogue_fetching = None;
                    }
                    dirty = true;
                }
                Ok(WarmEvent::WarmCatalogueFailed { endpoint }) => {
                    // Record a FAILED fetch for this endpoint without poisoning
                    // the cache: leave `models_cache` / `models_cache_endpoint`
                    // untouched so the image-capability tri-state helper returns
                    // `Unknown` (fail-open) instead of `DoesNotSupport` on an
                    // empty cache. The `models_cache_failed` marker prevents
                    // `request_catalogue` from re-fetching in a rapid loop; the
                    // next user-driven re-trigger (keystroke / provider change)
                    // clears the stale failure and retries.
                    state.rest.models_cache_failed = Some(endpoint.clone());
                    if state.rest.catalogue_fetching.as_deref() == Some(endpoint.as_str()) {
                        state.rest.catalogue_fetching = None;
                    }
                    dirty = true;
                }
                Ok(WarmEvent::WarmAwareness { session_id, summary }) => {
                    let had = summary.is_some();
                    // Route by SESSION ID (C4): the warm result belongs to exactly the
                    // session that was warming, identified by its stable UUID tagged into
                    // the event. The shared `warm_rx` is REPLACED per warm, so without the
                    // tag a result could land on whatever OTHER session happens to still be
                    // in `Mode::Loading` (two near-simultaneous `/new`s) — that was the
                    // cross-session corruption C3 exposed. `service_global` runs OUTSIDE a
                    // client bracket, so the foreground cursor is stale scratch here. Find
                    // the tagged session by id and set its summary (appended to the system
                    // message on every request); advance ITS splash step if it is still
                    // Loading (it may have been Esc'd to Chat — the summary must land
                    // regardless, preserving "summary populates even after skip").
                    if let Some(s) = state.rest.sessions.iter_mut().find(|s| s.id == session_id) {
                        if let Mode::Loading(ls) = &mut s.mode {
                            // Some → ready; None → "no docs" (a benign terminal Done detail,
                            // not a hard failure).
                            ls.awareness = if had {
                                WarmStatus::Done("ready".into())
                            } else {
                                WarmStatus::Done("no docs".into())
                            };
                        }
                        s.awareness_summary = summary;
                    }
                    // If the tagged session is gone (closed/never found) the result is
                    // simply dropped — there is no live session to carry it.
                    dirty = true;
                }
                // Channel drained for now: keep listening on later ticks.
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                // Both warm tasks finished and dropped their senders: done.
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    keep = false;
                    break;
                }
            }
        }
        if keep {
            state.rest.warm_rx = Some(wrx);
        }
    }
    dirty
}

/// Fire a DEBOUNCED, on-demand model-catalogue fetch. The model omnisearch
/// arms `catalogue_pending` (via `request_catalogue`) on each keystroke /
/// provider change, pushing `due` ~300ms forward so a typing burst collapses
/// into one request. Fire here — where `handle` + `client` are in scope — once
/// `due` passes and nothing is already in flight. Reuse the shared `warm_rx`
/// channel (no new channel): [`drain_warm`] folds the result into the
/// per-endpoint cache. On failure the drain records a `models_cache_failed`
/// marker (no rapid re-fetch); the next user-driven re-trigger retries.
pub(super) fn fetch_catalogue_debounced(
    state: &mut AppState,
    client: &mut Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) -> bool {
    let mut dirty = false;
    if let Some(pending) = state.rest.catalogue_pending.as_ref() {
        if state.rest.catalogue_fetching.is_none() && std::time::Instant::now() >= pending.due {
            // Take the pending request and mark its endpoint in-flight.
            let pending = state.rest.catalogue_pending.take().unwrap();
            let endpoint = pending.endpoint;
            let api_key = pending.api_key;
            let oauth_uuid = pending.oauth_uuid;
            state.rest.catalogue_fetching = Some(endpoint.clone());
            // Open a fresh warm channel for this fetch and stash its receiver.
            // Senders aren't stored in state (only the receiver), so this is the
            // only way to obtain one. This is safe wrt the awareness warm task:
            // the omnisearch (the sole `request_catalogue` caller) only runs in
            // Chat-mode modals / the first-run wizard, by which point the startup
            // awareness task has already resolved + closed its channel — so no
            // live awareness send can be stranded on a replaced receiver.
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            state.rest.warm_rx = Some(rx);
            // Reuse the pinned client, or build a keyless one (the first-run
            // wizard fetches before any client is pinned — `Conn` carries the
            // endpoint+key, so a keyless client is enough). The fetch is just
            // `GET {endpoint}/models`; on error the drain records a failure marker
            // (no rapid re-fetch).
            let c = match client.as_ref() {
                Some(c) => Arc::clone(c),
                None => crate::app::runtime::build_client(),
            };
            handle.spawn(async move {
                let conn = crate::service::openrouter::Conn {
                    endpoint: &endpoint,
                    api_key: &api_key,
                    // Catalogue fetch is OpenAI-compatible. `oauth_uuid` is threaded
                    // through so an OAuth-backed catalogue (e.g. Kilo Code) refreshes
                    // its bearer via `fresh_key`; empty for a static-key provider.
                    api_type: crate::model::app_config::ApiType::OpenAiCompatible,
                    account_id: "",
                    oauth_uuid: &oauth_uuid,
                    // Catalogue GET, never koma-free — no X-Koma header needed.
                    install_id: "",
                };
                let ev = match c.list_models(conn).await {
                    Ok(models) => WarmEvent::WarmCatalogue { endpoint, models },
                    Err(_) => WarmEvent::WarmCatalogueFailed { endpoint },
                };
                // A dropped receiver (app closing) makes this a no-op.
                let _ = tx.send(ev);
            });
            dirty = true;
        }
    }
    dirty
}

/// ADVANCE the security health-probe spinner while a probe is in flight. Mirrors the
/// loading-splash frame advance: bump the frame counter each tick on the OPEN panel so
/// the braille frames actually cycle, paired with the force-dirty check so the loop
/// redraws even though no events arrive during the cold IPC round-trip. De-globalized
/// (C3): bump it on whichever session(s) have `/security` open, not the stale foreground.
pub(super) fn advance_security_spinner(state: &mut AppState) -> bool {
    let mut dirty = false;
    if state.rest.sec_health_rx.is_some() {
        for s in security_states(state) {
            s.health_frame = s.health_frame.wrapping_add(1);
            dirty = true;
        }
    }
    dirty
}

/// ADVANCE the OAuth submenu's connect-flow spinner while a Codex/Kilo Code
/// flow is waiting. Mirrors the security health-probe advance above: only the
/// WAIT screens carry a `frame` counter (the picker/paste/failed screens don't
/// animate), so bump it only for sessions currently showing one of those two.
pub(super) fn advance_oauth_spinner(state: &mut AppState) -> bool {
    let mut dirty = false;
    if state.rest.oauth_rx.is_some() {
        for flow in oauth_flow_states(state) {
            match flow {
                OAuthFlowState::CodexWait { frame, .. } | OAuthFlowState::KiloWait { frame, .. } => {
                    *frame = frame.wrapping_add(1);
                    dirty = true;
                }
                _ => {}
            }
        }
    }
    dirty
}

/// De-globalization helper (C3): mutably borrow the [`SecurityState`] of EVERY session
/// currently showing the `/security` panel.
///
/// `service_global` runs OUTSIDE any client bracket, so the transient foreground cursor is
/// stale scratch — a drain that targets "the open `/security` panel" must reach the
/// session(s) actually in `Mode::Security`, not the foreground. In the single-window case
/// at most one session is in that mode, so the iterator yields one element and behaviour is
/// identical to the old `if let Mode::Security(s) = &mut state.mode`.
fn security_states(
    state: &mut AppState,
) -> impl Iterator<Item = &mut crate::app::mode::SecurityState> {
    state.rest.sessions.iter_mut().filter_map(|s| match &mut s.mode {
        Mode::Security(sec) => Some(sec.as_mut()),
        _ => None,
    })
}

/// De-globalization helper (C3): mutably borrow the [`crate::app::mode::SettingsState`]
/// of EVERY session currently showing the `/settings` dashboard.
///
/// The OAuth submenu's connect-flow drain (above) lands here outside any client
/// bracket, so it must reach whichever session(s) are actually in `Mode::Settings` —
/// never the (stale) foreground. Mirrors [`security_states`]; in the single-window
/// case at most one session matches.
fn settings_states(
    state: &mut AppState,
) -> impl Iterator<Item = &mut crate::app::mode::SettingsState> {
    state.rest.sessions.iter_mut().filter_map(|s| match &mut s.mode {
        Mode::Settings(set) => Some(set.as_mut()),
        _ => None,
    })
}

/// De-globalization helper (C3): mutably borrow the [`OAuthFlowState`] of EVERY session
/// whose mode carries an OAuth connect-flow — the `/settings` dashboard OR the guided
/// provider onboarding wizard.
///
/// The OAuth drain's shared-flow mutations (URL/code arrival, failure, spinner advance)
/// are identical for both modes, so they fold through this one helper; only the terminal
/// `Success` event diverges (Settings rebuilds its drafts; the wizard advances to model
/// select), handled by iterating [`settings_states`] and [`onboard_provider_states`]
/// separately. `service_global` runs outside any client bracket, so it must reach the
/// session(s) actually in one of those modes, never the (stale) foreground.
fn oauth_flow_states(state: &mut AppState) -> impl Iterator<Item = &mut OAuthFlowState> {
    state.rest.sessions.iter_mut().filter_map(|s| match &mut s.mode {
        Mode::Settings(set) => Some(&mut set.oauth_flow),
        Mode::OnboardProvider(op) => Some(&mut op.oauth_flow),
        _ => None,
    })
}

/// De-globalization helper (C3): mutably borrow the [`OnboardProviderState`] of EVERY
/// session currently in the guided provider onboarding wizard. Mirrors [`settings_states`];
/// in the single-window case at most one session matches.
fn onboard_provider_states(
    state: &mut AppState,
) -> impl Iterator<Item = &mut OnboardProviderState> {
    state.rest.sessions.iter_mut().filter_map(|s| match &mut s.mode {
        Mode::OnboardProvider(op) => Some(op.as_mut()),
        _ => None,
    })
}

/// De-globalization helper (C3): apply `f` to the Settings model-modal of every session
/// whose modal is awaiting endpoints for `model_id` (matched by `ModelModal::endpoints_for`).
///
/// The per-model provider-endpoints fetch (a single rest-global receiver) lands in
/// `service_global` outside any client bracket, so the result is folded into WHICHEVER
/// session(s) have a Settings model-modal open on THIS model — never the (stale) foreground.
/// One fetch is in flight at a time, so in practice one session matches; iterating keeps it
/// index-correct and matches the old foreground-only fold for the single-window case.
fn apply_to_settings_modal_for(
    state: &mut AppState,
    model_id: &str,
    mut f: impl FnMut(&mut crate::app::mode::settings::ModelModal),
) {
    for s in state.rest.sessions.iter_mut() {
        if let Mode::Settings(set) = &mut s.mode {
            if let Some(m) = set.model_modal.as_mut() {
                if m.endpoints_for.as_deref() == Some(model_id) {
                    f(m);
                }
            }
        }
    }
}
