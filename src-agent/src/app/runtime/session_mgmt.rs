use std::sync::Arc;

use crate::app::mode::{LoadingState, Mode, WarmStatus};
use crate::app::resolve::resolve_role;
use crate::app::state::AppState;
use crate::model::app_config::ModelRole;
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

/// Warm a newly-activated session to match a cold terminal launch: kick off a
/// background reindex of its workspace and (best-effort) compute the project
/// awareness summary. Safe to call whenever a session becomes the active one
/// (startup, /new, picker-select, creds-confirm). No-op if no session.
///
/// NON-BLOCKING: the awareness network call used to run via `handle.block_on` on
/// the UI thread BEFORE the event loop started, so a slow network froze the app on
/// a black screen. It is now SPAWNED as a background task (mirroring the endpoints
/// fetch), and — when there is awareness work to do — this switches the app into
/// [`Mode::Loading`], an animated splash the event loop renders while the task
/// runs. The task sends a [`WarmEvent::WarmAwareness`] on `warm_rx`, drained in
/// `run_loop` to populate the summary and advance the splash; once the awareness
/// step is terminal the loop enters Chat. This function returns immediately (it
/// only spawns), so startup never blocks.
///
/// The model catalogue is NO LONGER fetched here: it loads ON DEMAND, per
/// endpoint, the first time a model omnisearch needs it (see
/// `AppStateRest::request_catalogue` + the debounced tick in `event_loop`). So
/// when awareness is disabled or unroutable, there is no warm work and the mode is
/// left as-is (Chat) — no splash flash.
pub(crate) fn warm_session(
    state: &mut AppState,
    client: &Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
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
            resolve_role(&config, &settings, ModelRole::Awareness).filter(|r| r.is_routable())
        } else {
            None
        }
    });

    // No awareness task to spawn → no splash; leave the mode as-is (Chat) so the
    // no-work case behaves exactly as before (no splash flash).
    if aware_route.is_none() {
        return;
    }

    *state.mode_mut() = Mode::Loading(LoadingState {
        started: std::time::Instant::now(),
        frame: 0,
        workspace: WarmStatus::Running,
        awareness: WarmStatus::Running,
    });

    // One channel for the warm task; the receiver lives in state and is drained in
    // run_loop. Dropping it (e.g. app close) makes the sends no-ops, same contract
    // as the streaming / endpoints channels.
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    state.rest.warm_rx = Some(rx);

    // Awareness task: read the depth-1 docs + summarize on the resolved Awareness
    // route. Move the owned route + the cloned settings/workdir in; `summarize`
    // returns `None` on no docs / failure, which the drain renders as the
    // appropriate terminal step. Also resolve the Main route as a fallback: when
    // the Awareness model call itself fails (e.g. bad/typo'd model name) we retry
    // once on the trusted Main route before giving up.
    if let (Some(c), Some(route)) = (client.as_ref(), aware_route) {
        let c = Arc::clone(c);
        // Resolve the Main route for fallback; cheap (no I/O). `None` is safe —
        // `summarize_with_fallback` skips the retry when the routes are equal or
        // Main is unavailable.
        let main_route = resolve_role(&config, &settings, ModelRole::Main);
        handle.spawn(async move {
            let summary = match main_route {
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
            };
            // Tag the result with the warming session's id so the drain routes it to
            // exactly that session (C4), never to a different Loading session.
            let _ = tx.send(WarmEvent::WarmAwareness {
                session_id: warming_id,
                summary,
            });
        });
    }
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
