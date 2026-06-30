//! Compact command: `/compact` — summarise and truncate the conversation.

use std::sync::Arc;

use anyhow::Result;
use tokio::sync::mpsc;

use crate::app::state::AppState;
use crate::dto::chat::{ChatMessage, Role};
use crate::service::{openrouter::OpenRouterClient, StreamEvent};

/// Handle the `/compact` command: summarise old turns and trim context.
pub(super) fn handle_compact(
    state: &mut AppState,
    client: &mut Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) -> Result<()> {
    if state.rest.fg().waiting {
        state.rest.status = "busy — wait for response".into();
        return Ok(());
    }
    if client.is_none() || state.rest.fg().session.is_none() {
        state.rest.status = "no active session".into();
        return Ok(());
    }
    let (to_sum, kept_tail) = {
        let sess = state.rest.fg().session.as_ref().unwrap();
        let pn = sess.settings.compaction.preserve_n;
        sess.conversation.split_for_compaction(pn)
    };
    if to_sum.is_empty() {
        state.rest.status = "nothing to compact".into();
        return Ok(());
    }
    let mut req = vec![ChatMessage::new(
        Role::System,
        "You are compacting a conversation to free up context. Write a concise SUMMARY of the conversation above for your own future reference — NOT a reply to the user. Capture: what the user is building or asking for; key decisions, facts, and constraints established; the current state; specific files, code, names, and values that matter; and any open threads or next steps. Use short labeled sections or terse bullet points. Be factual. Do not greet, do not continue the task, do not address the user.",
    )];
    req.extend(to_sum);
    state.rest.fg_mut().waiting = true;
    state.rest.status = "compacting...".into();
    // Start THIS session's compaction animation clock (per-session, C4 — set on the
    // acting session, which in a client bracket is `fg()`). The renderer reads the
    // foreground session's value to draw the spinner/elapsed/bar; the event loop reads
    // it to redraw each tick and to enforce the minimum on-screen duration. Clear any
    // stale deferred-apply bookkeeping from a prior compaction on this session.
    {
        let fg = state.rest.fg_mut();
        fg.compact_anim_start = Some(std::time::Instant::now());
        fg.compact_apply_at = None;
        fg.compact_pending = None;
    }
    // Resolve the COMPACTOR role (falls back to Main — compaction rides the
    // main route today) into an owned `Resolved` BEFORE the spawn, so the
    // moved-into-task value carries no borrow of `state.rest`. Compactor
    // always resolves (Main legacy fallback), but guard defensively.
    let route = state.rest.fg().session.as_ref().and_then(|s| {
        crate::app::resolve::resolve_role(
            &state.rest.config,
            &s.settings,
            crate::model::app_config::ModelRole::Compactor,
        )
    });
    // Fresh channel for this request; the receiver lives in state so an
    // interrupt/new just drops it and the task's result is ignored.
    let (tx, rx) = mpsc::unbounded_channel();
    state.rest.fg_mut().active_rx = Some(rx);
    let c = Arc::clone(client.as_ref().unwrap());
    let jh = handle.spawn(async move {
        // Compaction sends on the resolved Compactor connection (endpoint +
        // key) with its model id + upstream-route slug; no effort (the
        // summary is mechanical).
        let result = match route {
            Some(r) => c.complete(r.conn(), &r.model_id, r.provider(), req).await,
            None => Err(anyhow::anyhow!("no active session")),
        };
        let event = match result {
            Ok(s) => StreamEvent::Compacted {
                summary: s,
                kept_tail,
            },
            Err(e) => StreamEvent::Error(e.to_string()),
        };
        let _ = tx.send(event);
    });
    state.rest.fg_mut().current_task = Some(jh.abort_handle());
    Ok(())
}
