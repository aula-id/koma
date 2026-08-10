use std::sync::Arc;

use crate::app::mode::{LoadingState, Mode, WarmStatus};
use crate::app::resolve::resolve_role_dispatch;
use crate::app::state::AppState;
use crate::model::app_config::{ApiType, ModelRole};
use crate::service::openrouter::OpenRouterClient;
use crate::service::WarmEvent;

/// Make the on-disk lock match the active session.
///
/// Releases the previously-held lock if the active session changed, then writes
/// a fresh `session.lock` for the current one. A no-op when the active session
/// is unchanged (so calling it on every activation is cheap and idempotent).
/// All lock IO is best-effort, so this never fails or blocks.
pub(crate) fn reconcile_session_lock(state: &mut AppState) {
    let cur = state.rest.fg().session.as_ref().map(|s| s.path.clone());
    if state.rest.fg().held_lock == cur {
        return; // active session unchanged → on-disk lock already correct
    }
    // Drop the stale lock first so switching away from a session unlocks it.
    if let Some(old) = state.rest.fg_mut().held_lock.take() {
        crate::model::store::remove_lock(&old);
    }
    // Acquire the new session's lock (if there is an active session now).
    if let Some(new) = cur {
        crate::model::store::write_lock(&new);
        state.rest.fg_mut().held_lock = Some(new);
    }
}

/// Warm-awareness timeout for the startup splash / background warm. SHORT on
/// purpose: the splash is skippable (Esc), but it must NEVER hang. An unbounded
/// awareness call on a cold/slow route (e.g. keyless koma-free) would strand the
/// session in [`Mode::Loading`], where the key dispatcher routes Esc to the splash-
/// skip and never to Chat's double-Esc composer-clear / rewind. On timeout the drain
/// still receives a terminal [`WarmEvent::WarmAwareness`] (`summary: None`), so a
/// splash flips Loading→Chat instead of hanging.
const WARM_AWARENESS_TIMEOUT_SECS: u64 = 60;

/// Warm a newly-activated session WITH the animated loading splash (the splash
/// variant: startup, /new, picker-select, creds-confirm). Thin wrapper over
/// [`warm_session_impl`] with `show_splash = true`; see it for the full contract.
pub(crate) fn warm_session(
    state: &mut AppState,
    client: &Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) {
    warm_session_impl(state, client, handle, true);
}

/// Warm a newly-activated session in the BACKGROUND — identical warm work to
/// [`warm_session`] (lock reconcile, workspace reindex, awareness task + channel) but
/// WITHOUT the [`Mode::Loading`] splash: the session stays in whatever mode it already
/// is (Chat), so the composer and the double-Esc composer-clear / rewind are usable
/// immediately. Used by the first-run onboarding tails, where a blocking splash on a
/// cold keyless route would otherwise swallow those keys. The awareness summary still
/// folds into `awareness_summary` via the shared `WarmEvent` drain regardless of mode
/// (the drain gates only the splash-STEP marker on `Mode::Loading`, not the summary
/// store). Thin wrapper over [`warm_session_impl`] with `show_splash = false`.
pub(crate) fn warm_session_background(
    state: &mut AppState,
    client: &Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) {
    warm_session_impl(state, client, handle, false);
}

/// Warm a newly-activated session to match a cold terminal launch: kick off a
/// background reindex of its workspace and (best-effort) compute the project
/// awareness summary. Safe to call whenever a session becomes the active one
/// (startup, /new, picker-select, creds-confirm). No-op if no session.
///
/// NON-BLOCKING: the awareness network call used to run via `handle.block_on` on
/// the UI thread BEFORE the event loop started, so a slow network froze the app on
/// a black screen. It is now SPAWNED as a background task (mirroring the endpoints
/// fetch), and — when there is awareness work to do AND `show_splash` is set — this
/// switches the app into [`Mode::Loading`], an animated splash the event loop renders
/// while the task runs. The task sends a [`WarmEvent::WarmAwareness`] on `warm_rx`,
/// drained in `run_loop` to populate the summary and advance the splash; once the
/// awareness step is terminal the loop enters Chat. This function returns immediately
/// (it only spawns), so startup never blocks. The spawned call is bounded by
/// [`WARM_AWARENESS_TIMEOUT_SECS`] so a hung upstream can never wedge the splash.
///
/// The model catalogue is NO LONGER fetched here: it loads ON DEMAND, per
/// endpoint, the first time a model omnisearch needs it (see
/// `AppStateRest::request_catalogue` + the debounced tick in `event_loop`). So
/// when awareness is disabled or unroutable, there is no warm work and the mode is
/// left as-is (Chat) — no splash flash.
fn warm_session_impl(
    state: &mut AppState,
    client: &Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
    show_splash: bool,
) {
    // Claim the lock for the now-active session (and release any prior one).
    // Cheap no-op when the active session is unchanged. Placed first so every
    // activation path that routes through warm_session — startup, /new,
    // picker-select, creds-confirm — acquires the lock.
    reconcile_session_lock(state);
    // Snapshot what we need, dropping the session borrow before mutating
    // `state.mode` / `state.rest`. `config` is cloned so the role resolution
    // below doesn't borrow `state` across the spawn.
    let (workdir, settings, workdirs) = match state.rest.fg().session.as_ref() {
        Some(s) => (s.workdir(), s.settings.clone(), s.workdirs()),
        None => return,
    };
    // Warm channel: shared between the linker poll and the awareness task so
    // both can send WarmEvent variants through a single drain.
    let (warm_tx, warm_rx) = tokio::sync::mpsc::unbounded_channel();
    state.rest.warm_rx = Some(warm_rx);
    // L1: best-effort linker daemon warm — ensure it's running, register the
    // session's workspace roots, then fetch the summary. All linker IPC runs on
    // a dedicated std::thread so it never stalls the daemon event loop (which
    // must respond to hub probes within 500ms).
    #[cfg(feature = "linker")]
    {
        let session_id = state.rest.fg().id.clone();
        let workdirs_clone = workdirs.clone();
        let tx = warm_tx.clone();
        std::thread::spawn(move || {
            // Best-effort: start daemon + register. Failure is silent (graph
            // just won't be available yet; the model turns fine without it).
            let _ = crate::linker::client::ensure_and_register(&workdirs_clone, &session_id);

            // Try immediate fetch. Always deliver through the warm channel so
            // the drain routes it by session id.
            match crate::linker::client::fetch_summary() {
                Some(result) if result.generation > 0 => {
                    let _ = tx.send(WarmEvent::WarmGraph {
                        session_id,
                        summary: Some(result.text),
                        generation: result.generation,
                    });
                }
                _ => {
                    // Generation is 0 or fetch failed — poll every 2s for up
                    // to 10s waiting for generation > 0.
                    for _ in 0..5 {
                        std::thread::sleep(std::time::Duration::from_secs(2));
                        if let Some(result) = crate::linker::client::fetch_summary_if_newer(0) {
                            if result.generation > 0 {
                                let _ = tx.send(WarmEvent::WarmGraph {
                                    session_id,
                                    summary: Some(result.text),
                                    generation: result.generation,
                                });
                                return;
                            }
                        }
                    }
                    // Timed out — send a no-op so the drain doesn't hang.
                    let _ = tx.send(WarmEvent::WarmGraph {
                        session_id,
                        summary: None,
                        generation: 0,
                    });
                }
            }
        });
    }
    // Capture the warming session's stable UUID (the SessionRuntime id, the same key the
    // drain routes on) so the WarmAwareness result lands on THIS session by id (C4),
    // even if another session replaces the shared `warm_rx` and is also Loading.
    let warming_id = state.rest.fg().id.clone();
    let config = state.rest.config.clone();
    // Workspace reindex is already async (background thread); fire it always,
    // independent of whether we show the loading splash.
    crate::tool::dircache::reindex(workdirs, state.rest.fg().dir_cache.clone());

    // Decide the warm work. Awareness runs only when the setting is on. It needs a
    // client AND a routable resolved route (an Anthropic-typed provider can't be
    // dispatched by the OpenAI-compatible client — native Anthropic is deferred).
    // "wanted but not routable" becomes a Skipped step (no task spawned).
    let want_awareness = settings.awareness_enabled;
    let aware_route = client.as_ref().and_then(|_| {
        if want_awareness {
            resolve_role_dispatch(&config, &settings, ModelRole::Awareness)
                .filter(|r| r.is_routable())
        } else {
            None
        }
    });

    // No awareness task to spawn → no splash; leave the mode as-is (Chat) so the
    // no-work case behaves exactly as before (no splash flash).
    if aware_route.is_none() {
        return;
    }

    // Resolve the Main route ONCE — it decides BOTH (a) whether the splash may show
    // and (b) the awareness-fallback route moved into the task below. koma-free Main is
    // KEYLESS and SLOW to warm, so a Loading splash on it strands the session in
    // `Mode::Loading` for the whole `WARM_AWARENESS_TIMEOUT_SECS` window — and there the
    // key dispatcher routes Esc to splash-skip, NEVER to Chat's double-Esc composer-clear
    // / rewind. So SUPPRESS the splash whenever the resolved Main is koma-free (regardless
    // of `show_splash`) and warm SILENTLY instead: blocking the UI in Loading is worse
    // than warming quietly. The awareness task + `warm_rx` drain below still run and fold
    // the summary in exactly as the background variant does. Non-koma-free (keyed / OAuth)
    // Mains are unchanged — they still get the splash when `show_splash`.
    let main_route = resolve_role_dispatch(&config, &settings, ModelRole::Main);
    let effective_splash = show_splash
        && !main_route
            .as_ref()
            .is_some_and(|r| r.api_type == ApiType::KomaFree);

    // Splash variant only: upgrade to the animated Loading splash while awareness
    // warms. The BACKGROUND variant (`show_splash = false`, first-run onboarding) AND a
    // koma-free Main (see above) SKIP this so the session stays in Chat — composer +
    // double-Esc live immediately — while the same awareness task below still runs and
    // folds its summary in via the drain.
    if effective_splash {
        *state.mode_mut() = Mode::Loading(LoadingState {
            started: std::time::Instant::now(),
            frame: 0,
            workspace: WarmStatus::Running,
            awareness: WarmStatus::Running,
        });
    }

    // Reuse the warm channel created above (shared with linker poll).
    let tx = warm_tx;

    // Awareness task: read the depth-1 docs + summarize on the resolved Awareness
    // route. Move the owned route + the cloned settings/workdir in; `summarize`
    // returns `None` on no docs / failure, which the drain renders as the
    // appropriate terminal step. Also resolve the Main route as a fallback: when
    // the Awareness model call itself fails (e.g. bad/typo'd model name) we retry
    // once on the trusted Main route before giving up.
    if let (Some(c), Some(route)) = (client.as_ref(), aware_route) {
        let c = Arc::clone(c);
        // `main_route` (resolved ONCE above, and reused here as the awareness fallback)
        // is moved into the task. `None` is safe — `summarize_with_fallback` skips the
        // retry when the routes are equal or Main is unavailable.
        handle.spawn(async move {
            // Bound the awareness call (WARM_AWARENESS_TIMEOUT_SECS): a hung/slow
            // upstream must NOT strand the session — with a splash it would sit in
            // Mode::Loading forever (swallowing double-Esc), and in the background it
            // would leak the task. Timeout OR any inner failure collapses to `None`
            // via `.ok().flatten()`, mirroring `spawn_awareness_recompute`; either way
            // the terminal WarmAwareness below still fires (so a splash flips
            // Loading→Chat, and the summary store is a harmless `None`).
            let summary = tokio::time::timeout(
                std::time::Duration::from_secs(WARM_AWARENESS_TIMEOUT_SECS),
                async {
                    match main_route {
                        Some(ref m) => {
                            crate::app::awareness::summarize_with_fallback(
                                &c,
                                &settings,
                                route.conn(),
                                &route.model_id,
                                route.provider(),
                                &workdir,
                                m.conn(),
                                &m.model_id,
                                m.provider(),
                            )
                            .await
                        }
                        None => {
                            crate::app::awareness::summarize(
                                &c,
                                &settings,
                                route.conn(),
                                &route.model_id,
                                route.provider(),
                                &workdir,
                            )
                            .await
                        }
                    }
                },
            )
            .await
            .ok()
            .flatten();
            // Tag the result with the warming session's id so the drain routes it to
            // exactly that session (C4), never to a different Loading session.
            let _ = tx.send(WarmEvent::WarmAwareness {
                session_id: warming_id,
                summary,
            });
        });
    }
}

/// Recompute the project-awareness summary for `session_id` OFF the event-loop
/// thread, delivering the result via the dedicated `awareness_rx`/`awareness_tx`
/// channel (see [`crate::app::state::AppStateRest::awareness_rx`]) instead of
/// `handle.block_on` — the summarize call is an unbounded network round-trip and
/// used to freeze the whole TUI while it ran (the `cd` and post-`/compact`
/// recomputes were the two remaining `block_on` sites; startup warming already
/// went through this pattern via `warm_session`/`WarmEvent::WarmAwareness`).
///
/// `route`/`main_route` are the caller's already-resolved Awareness/Main routes
/// (moved in, not borrowed, so the spawned task is `'static`); `client`/`settings`/
/// `workdir` are the same Send-safe snapshot the two call sites already prepared
/// before their old `block_on`. Lazily opens the channel pair on first call and
/// stashes both ends in `state.rest` (mirrors `warm_session`'s `warm_rx` setup,
/// but this pair is created once and kept — never replaced — since multiple
/// recomputes across different sessions/times must not strand each other).
///
/// The call is capped at 60s via `tokio::time::timeout`; a hung upstream times
/// out to `None` (via `.ok().flatten()`), matching `summarize`'s own "best-effort,
/// `None` on any failure" contract.
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_awareness_recompute(
    state: &mut AppState,
    handle: &tokio::runtime::Handle,
    session_id: String,
    client: Arc<OpenRouterClient>,
    settings: crate::model::settings::Settings,
    workdir: std::path::PathBuf,
    route: crate::app::resolve::Resolved,
    main_route: Option<crate::app::resolve::Resolved>,
) {
    let tx = match state.rest.awareness_tx.clone() {
        Some(tx) => tx,
        None => {
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            state.rest.awareness_rx = Some(rx);
            state.rest.awareness_tx = Some(tx.clone());
            tx
        }
    };
    handle.spawn(async move {
        let summary = tokio::time::timeout(std::time::Duration::from_secs(60), async {
            match main_route {
                Some(ref m) => {
                    crate::app::awareness::summarize_with_fallback(
                        &client,
                        &settings,
                        route.conn(),
                        &route.model_id,
                        route.provider(),
                        &workdir,
                        m.conn(),
                        &m.model_id,
                        m.provider(),
                    )
                    .await
                }
                None => {
                    crate::app::awareness::summarize(
                        &client,
                        &settings,
                        route.conn(),
                        &route.model_id,
                        route.provider(),
                        &workdir,
                    )
                    .await
                }
            }
        })
        .await
        .ok()
        .flatten();
        // Best-effort delivery: if the app has since dropped the receiver (closed
        // channel — shouldn't happen, the receiver is held for the app's lifetime
        // once created), the send is simply a no-op.
        let _ = tx.send((session_id, summary));
    });
}

/// Build a fresh per-session client.
///
/// The client is now KEYLESS — it carries no creds/model/provider/effort, only
/// `http` + a fresh `plan_word`. So this is needed ONLY at session boundaries
/// (startup, `/new`, picker-select, creds-confirm, cancel paths) to re-roll the
/// cache-stable `plan_word`; it must NOT be called on a mid-session cred/effort
/// change, since those are read per-call via `resolve_role`. The `&Session`
/// param is gone — building doesn't depend on session state anymore.
pub(crate) fn build_client() -> Arc<OpenRouterClient> {
    Arc::new(OpenRouterClient::new())
}
