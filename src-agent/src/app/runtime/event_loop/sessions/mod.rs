//! Render-agnostic per-session servicing for the central event loop.
//!
//! [`service_all_sessions`] advances every session's in-flight turn each tick
//! by delegating to the three submodules: streaming, subagents, and deferred.
#![allow(unused_imports)]
#![allow(dead_code)]

mod streaming;
mod subagents;
mod deferred;

use std::sync::Arc;

use crate::app::state::AppState;
use crate::service::openrouter::OpenRouterClient;

/// Service every session once: drain its async results and advance its turn
/// state. Returns `true` if anything changed (any event was applied / any turn
/// stepped), so the caller can flag a redraw. Render-agnostic: it never touches
/// the terminal, input, or any foreground-only / global drain.
///
/// For each session index `idx` (so a background session keeps streaming + running
/// tools + advancing its sub-agents while a different session is on screen) it does,
/// in the SAME relative order the old inline drains used for the foreground session:
///   1. drain `sessions[idx].active_rx` (stream tokens/usage/tool-calls/done/error/compacted),
///   2. drain `sessions[idx]`'s sub-agents (collect-then-apply, terminal delivery, queued starts),
///   3. drain `sessions[idx].tool_task_rx` (deferred heavy-tool results),
///   4. fire the resume gate when both deferred lanes for `idx` are empty.
pub(super) fn service_all_sessions(
    state: &mut AppState,
    client: &Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) -> bool {
    let mut dirty = false;
    for idx in 0..state.rest.sessions.len() {
        dirty |= service_session(state, idx, client, handle);
    }
    dirty
}

/// Service a single session `idx`. Split out so the borrow patterns read the
/// same as the old foreground-only code with `idx` swapped in for `foreground`.
fn service_session(
    state: &mut AppState,
    idx: usize,
    client: &Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) -> bool {
    // TOMBSTONE skip (daemon stage 10): a CLOSED session is inert — `close()` already
    // aborted its stream + sub-agents, dropped every receiver, and released its lock.
    // Do NOT drain it, advance its turn, fire its background-finish nudge, or touch
    // `was_working`; just leave the slot in place (removing it would shift indices and
    // cross-wire the OTHER sessions' in-flight async). Nothing changed, so report
    // not-dirty.
    if state.rest.sessions[idx].closed {
        return false;
    }

    let mut dirty = false;

    // 1. Drain stream events + PC verdict channel.
    dirty |= streaming::drain_stream(state, idx, client, handle);

    // 2. Drain sub-agent channels (collect-then-apply, terminal delivery, queued starts).
    dirty |= subagents::drain_subagents(state, idx, client, handle);

    // 3. Drain deferred tool-task + shell lanes, then fire resume gate.
    dirty |= deferred::drain_deferred_and_resume(state, idx, client, handle);

    // 4. Background-finish nudge + was_working update.
    dirty |= deferred::nudge_background_finish(state, idx);

    // 5. Cross-instance memory sync: poll MEMORY.md mtime and rebuild the
    //    system prompt if another koma instance updated the shared store.
    dirty |= poll_memory_change(state, idx);

    dirty
}

/// Poll the mtime of the shared `MEMORY.md` and, if it changed since the last
/// snapshot, rebuild the system prompt so this session sees the updated memory
/// index. Fires an info toast to notify the user. Cheap: one `stat()` syscall
/// per tick, no full re-parse unless mtime actually changed.
fn poll_memory_change(state: &mut AppState, idx: usize) -> bool {
    let pwd_hash = match state.rest.sessions[idx]
        .session
        .as_ref()
        .map(|s| s.pwd_hash.clone())
    {
        Some(h) => h,
        None => return false,
    };
    let mem_dir = match crate::model::store::memory_dir(&pwd_hash) {
        Ok(d) => d,
        Err(_) => return false,
    };
    let current_mtime = crate::model::memory::memory_mtime(&mem_dir);
    let prev_mtime = state.rest.sessions[idx].last_memory_mtime;

    // Update the snapshot regardless (tracks both own writes and external).
    state.rest.sessions[idx].last_memory_mtime = current_mtime;

    // Detect change: mtime differs from previous snapshot, or this is the
    // first poll (prev is None) and the file exists — seed without toast.
    match (prev_mtime, current_mtime) {
        (None, Some(_)) => {
            // First poll: just seed. No toast — this is session init, not an
            // external change.
            if let Some(sess) = state.rest.sessions[idx].session.as_mut() {
                sess.rebuild_system();
            }
            false
        }
        (Some(prev), Some(curr)) if prev != curr => {
            // External write detected — rebuild and notify.
            if let Some(sess) = state.rest.sessions[idx].session.as_mut() {
                sess.rebuild_system();
            }
            state.rest.sessions[idx].set_toast_info("Memory updated by another session".into());
            true
        }
        _ => false,
    }
}
