//! Stream task management: start, abort, and manage the async streaming task.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use crate::app::state::{AppState, AppStateRest};
use crate::dto::chat::{ChatMessage, Role};
use crate::service::openrouter::OpenRouterClient;

/// Abort the in-flight task and stop listening to it: aborts the task handle,
/// drops the active receiver (so any late events from the task vanish), and
/// clears the waiting flag.
pub(crate) fn abort_current(rest: &mut AppStateRest) {
    let rt = rest.fg_mut();
    if let Some(h) = rt.current_task.take() {
        h.abort();
    }
    rt.active_rx = None;
    rt.waiting = false;
    // Tear down any in-flight compaction animation / deferred apply so an
    // interrupt (Esc) or `/new` mid-compact doesn't leave the spinner stuck (and
    // forcing a per-tick redraw) forever.
    rest.compact_anim_start = None;
    rest.compact_apply_at = None;
    rest.compact_pending = None;
}

/// Spawn a streaming task for `history`. Opens a fresh channel, stashes the
/// receiver in state, and hands the sender to the task — so this request's
/// events are isolated from any previous one (no generation tagging needed).
pub(crate) fn start_stream_task(
    mut history: Vec<ChatMessage>,
    state: &mut AppState,
    sess_idx: usize,
    client: &Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) {
    // Assemble the System message so the prompt-caching breakpoint covers only the
    // STABLE head (which is byte-identical across the session, so the cache hits):
    //
    //   [ stable base system prompt (already in history[0]) ]
    // + [ plan-word steer (same word every request, chosen once per client) ]
    // + CACHE_SPLIT_MARK                                     <- cache breakpoint here
    // + [ "\n\n# Project files (top level)" listing ]       (volatile: changes with files)
    // + [ "\n\n# Project summary" awareness block ]          (volatile: project-dependent)
    //
    // The plan-word steer + the mark go in FIRST, before the volatile tail, so the
    // head ends at the mark and the listing/awareness land after it. `to_wire`
    // splits on the mark and attaches `cache_control` to the head only; the tail
    // rides as a second, uncached content part. Injecting here (BEFORE `to_wire`)
    // keeps the steer inside the cached block. The tail may be empty (no listing /
    // no summary) — `to_wire` handles that by emitting a single cached part.
    if let Some(first) = history.first_mut() {
        if first.role == Role::System {
            // Plan-word steer: lead the FIRST plan with the session's whimsical
            // word instead of "Plan:". `plan_word` is chosen once per client, so
            // the SAME word every request keeps the cached head byte-stable.
            if let Some(c) = client.as_ref() {
                let word = c.plan_word();
                first.content.push_str(&format!(
                    "\n\nWhen you write your plan for this task, lead with the single word \"{word}:\" (a whimsical lead-in) instead of \"Plan:\"."
                ));
            }
            // Boundary between the stable cached head (everything above) and the
            // volatile tail (everything below). Inserted unconditionally so the
            // split point always exists, even when the tail ends up empty.
            first.content.push_str(crate::dto::chat::CACHE_SPLIT_MARK);
            // Volatile tail begins here — project layout + awareness summary. Sent
            // every request (so they survive compaction too) but kept AFTER the
            // cache breakpoint so file changes never bust the cached prefix.
            if let Ok(cache) = state.rest.sessions[sess_idx].dir_cache.read() {
                let mut listing = cache.children(".", 0);
                // When multi-workspace, also list entries from other workspaces.
                if cache.is_multi() {
                    for i in 1.. {
                        let more = cache.children(".", i);
                        if more.is_empty() { break; }
                        listing.extend(more);
                    }
                }
                if !listing.is_empty() {
                    first.content.push_str("\n\n# Project files (top level)\n");
                    first.content.push_str(&listing.join("\n"));
                }
            }
            if let Some(summary) = state.rest.sessions[sess_idx].awareness_summary.as_deref() {
                if !summary.is_empty() {
                    first.content.push_str("\n\n# Project summary\n");
                    first.content.push_str(summary);
                }
            }
        }
    }
    // Short-send reshape inputs, snapshotted out of `state` BEFORE the spawn so
    // the task holds no borrow of `state`. Cloning the session dir + settings +
    // latest user message lets `shortsend::shape` run its fold/router off the UI
    // thread (the task already shows the "waiting" state, so the UI never freezes
    // on these secondary-model calls). `None` when there's no session — the task
    // then sends the injected history unchanged.
    //
    // DUAL RAIL: `shape` only transforms this API-bound `history` Vec (built from
    // `sess.conversation.history()` by the caller). It reads `messages.sqlite` and
    // returns a NEW Vec; it does not touch `sess.conversation`, `messages.json`,
    // or the rendered transcript — display is entirely unaffected.
    //
    // The OLD per-send "is the history near the window?" gate moves HERE (out of
    // shape) so it can read the live cache-warmth + sticky engage state, which only
    // exists on `state`. We compute the engage decision (a bool) + the token budget
    // (`usable`) into locals FIRST — all the `state.rest` reads happen up front so
    // they don't borrow-conflict with the per-session snapshot or the two writes
    // below. Everything here is a no-op (`summarizing` stays false, the task sends
    // the history unchanged) when there's no active session.
    //
    // The per-session snapshot the reshape task needs: (dir, settings, latest user
    // message, resolved Awareness route). Cloned out of the session up front so the
    // spawned task holds no borrow of `state`, and so `settings` is available to
    // size the window + read `sliding_cache` below without re-borrowing the session.
    //
    // `shape`'s fold + snippet-router ride the AWARENESS role; resolve it HERE
    // (before the spawn) into an owned `Resolved` so the moved-into-task value
    // carries no borrow of `state.rest.config`. `None` (an unresolved Awareness
    // role) makes `shape` skip the fold/router (existing summary still applies).
    let reshape: Option<(
        std::path::PathBuf,
        crate::model::settings::Settings,
        String,
        Option<crate::app::resolve::Resolved>,
    )> = state.rest.sessions[sess_idx].session.as_ref().map(|sess| {
        let user_intent = sess.conversation.last_user_content().unwrap_or_default();
        // Call-boundary gate for the SECONDARY fold/router calls: an Anthropic-typed
        // Awareness route can't be dispatched (native Anthropic is deferred), so
        // downgrade it to `None`. `shape` already treats `None` as "skip the fold +
        // snippet-router" gracefully (existing summary still applies) — no summary /
        // no recall, never a crash.
        let aware = crate::app::resolve::resolve_role(
            &state.rest.config,
            &sess.settings,
            crate::model::app_config::ModelRole::Awareness,
        )
        .filter(|r| r.is_routable());
        (sess.path.clone(), sess.settings.clone(), user_intent, aware)
    });

    // Resolve the MAIN role for the actual send: its connection (endpoint + key),
    // model id, upstream-route slug, and effort. EFFORT ISOLATION: effort flows
    // ONLY here, into the streaming path. Resolved BEFORE the spawn into an owned
    // `Resolved` so the moved-into-task value carries no borrow of `state.rest`.
    // Main always resolves (legacy fallback), but keep it `Option` and treat a
    // `None` as "no session" below.
    let main = state.rest.sessions[sess_idx].session.as_ref().and_then(|sess| {
        crate::app::resolve::resolve_role(
            &state.rest.config,
            &sess.settings,
            crate::model::app_config::ModelRole::Main,
        )
    });

    // 1. Window: the model's context-window size in tokens, from the cached
    //    catalogue. WINDOW-SIZING FIX: size against the RESOLVED Main model id
    //    (what we actually send), NOT the legacy `settings.model` — a per-session
    //    or config Main override must size the short-send window correctly. 128k is
    //    a safe fallback (the min-window policy is 100k+).
    let window = main
        .as_ref()
        .and_then(|m| {
            state
                .rest
                .models_cache
                .as_deref()
                .and_then(|models| {
                    crate::service::openrouter::context_length_for(models, &m.model_id)
                })
        })
        .unwrap_or(128_000);
    // Image-attachment send context: the session dir (source of record for image
    // bytes), whether the resolved Main model can read images, and its id (named
    // in the strip-warning). Built BEFORE the spawn so the task holds no borrow of
    // `state`. Capability: when the catalogue is populated, honour
    // `model_takes_images`; when it's never been fetched (`None`), DEFAULT TO
    // CAPABLE so a cold cache never wrongly strips an image. `None` (no session /
    // no resolved Main) means the task sends without image handling.
    let image_ctx: Option<crate::dto::openrouter::ImageWireCtx> = match (
        state.rest.sessions[sess_idx].session.as_ref(),
        main.as_ref(),
    ) {
        (Some(sess), Some(m)) => {
            let takes = match state.rest.models_cache.as_deref() {
                Some(models) => {
                    crate::service::openrouter::model_takes_images(models, &m.model_id)
                }
                None => true, // catalogue not fetched yet → assume capable, never strip
            };
            Some(crate::dto::openrouter::ImageWireCtx {
                session_dir: sess.path.clone(),
                model_takes_images: takes,
            })
        }
        _ => None,
    };
    // 2. Usable budget: the window minus the fixed system/tools/memory overhead,
    //    floored so the percentages below never go degenerate on a tiny window.
    let usable = window
        .saturating_sub(super::super::shortsend::BASE_OVERHEAD)
        .max(8_000);
    // 3. Conversation size estimate (~4 chars/token over content + tool args).
    let conv_tokens = super::super::shortsend::estimate_conv_tokens(&history);
    // 4. Cache warmth: a warm cache (provider supports caching, the cache holds
    //    tokens, and the last send was recent enough that it hasn't gone cold)
    //    lets the conversation grow far larger before we summarize. The cold
    //    window is longer when the provider runs a sliding/refreshing cache.
    let sliding_cache = reshape
        .as_ref()
        .is_some_and(|(_, settings, _, _)| settings.sliding_cache);
    let gap = state.rest.sessions[sess_idx].last_send_at.map(|t| t.elapsed());
    let cold_window = if sliding_cache {
        Duration::from_secs(300)
    } else {
        Duration::from_secs(120)
    };
    let cache_warm = state.rest.sessions[sess_idx].provider_caches
        && state.rest.sessions[sess_idx].tokens_cached > 0
        && gap.is_some_and(|g| g < cold_window);
    let engage_pct = if cache_warm {
        super::super::shortsend::ENGAGE_WARM_PCT
    } else {
        super::super::shortsend::ENGAGE_COLD_PCT
    };
    // 5. Sticky engage hysteresis: cross the (warmth-dependent) engage threshold to
    //    turn summarizing ON; only fall back below DISENGAGE_PCT to turn it OFF.
    //    The dead-zone between the two prevents flapping on/off each turn.
    let enter = conv_tokens > engage_pct * usable / 100;
    let exit = conv_tokens < super::super::shortsend::DISENGAGE_PCT * usable / 100;
    if !state.rest.sessions[sess_idx].summarizing && enter {
        state.rest.sessions[sess_idx].summarizing = true;
    } else if state.rest.sessions[sess_idx].summarizing && exit {
        state.rest.sessions[sess_idx].summarizing = false;
    }
    let summarizing = state.rest.sessions[sess_idx].summarizing;
    // 6. Stamp the send instant so the NEXT turn can measure cache warmth from the
    //    gap since this send.
    state.rest.sessions[sess_idx].last_send_at = Some(Instant::now());
    let (tx, rx) = mpsc::unbounded_channel();
    state.rest.sessions[sess_idx].active_rx = Some(rx);
    let c = Arc::clone(client.as_ref().unwrap());
    let jh = handle.spawn(async move {
        // Reshape the wire payload just before POSTing. `shape` preserves the
        // system message at index 0 (with the project-files/awareness injection
        // applied above, plus — when engaged — the condensed-history summary
        // appended to its uncached tail), so the model still receives the real
        // system prompt. It fails open — any error returns the original history —
        // so this can never break the send. `summarizing` is the upstream engage
        // decision; `usable` is the token budget the fold's band sizing uses.
        let history = match reshape {
            Some((session_dir, settings, user_intent, route)) => {
                super::super::shortsend::shape(
                    history,
                    &session_dir,
                    &c,
                    &settings,
                    route,
                    &user_intent,
                    summarizing,
                    usable,
                )
                .await
            }
            None => history,
        };
        // Send on the resolved MAIN route: its connection (endpoint + key), model
        // id, upstream-route slug, and effort. The owned `Resolved` was moved into
        // this task; borrow it for the call. A `None` (no session) can't reach here
        // — the client only exists when Main resolves — but guard defensively.
        if let Some(m) = main {
            // Call-boundary gate (FAIL LOUD): the OpenAI-compatible client must
            // never POST its body to an Anthropic-typed provider — that endpoint
            // speaks a different wire protocol (native Anthropic is deferred), so
            // the request would 400/404 with an opaque error. Surface a clear
            // error on the stream channel and DON'T dispatch; the drain folds it
            // into the status line + toast exactly like any stream failure.
            if !m.is_routable() {
                let _ = tx.send(crate::service::StreamEvent::Error(
                    "Anthropic-compatible providers are not wired yet".to_string(),
                ));
            } else {
                let _ = c
                    .stream_complete(
                        m.conn(),
                        &m.model_id,
                        m.provider(),
                        &m.effort,
                        history,
                        &crate::tool::main_tool_names(),
                        image_ctx,
                        tx,
                    )
                    .await;
            }
        }
    });
    state.rest.sessions[sess_idx].current_task = Some(jh.abort_handle());
}
