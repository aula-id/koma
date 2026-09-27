//! Compact command: `/compact` — summarise and truncate the conversation.

use std::sync::Arc;

use anyhow::Result;
use tokio::sync::mpsc;

use crate::app::state::AppState;
use crate::dto::chat::{ChatMessage, Role};
use crate::service::{openrouter::OpenRouterClient, StreamEvent};

/// Handle the `/compact` command: summarise old turns and trim context.
///
/// `preserve_n_override` lets a caller force how many recent turns are kept
/// verbatim: `/compact` passes `None` (use the session's configured
/// `compaction.preserve_n`), while the plan-approval compaction passes `Some(0)`
/// to collapse the ENTIRE exploratory history to the summary (the approved plan
/// is seeded separately in `apply_compaction_result`).
///
/// `pub(crate)` so the deferred/idle drain can fire it once the post-approval
/// turn settles. It still operates on the FOREGROUND session, so the drain call
/// site gates on `idx == foreground` to keep the two in sync.
pub(crate) fn handle_compact(
    state: &mut AppState,
    client: &mut Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
    preserve_n_override: Option<usize>,
) -> Result<()> {
    if state.rest.fg().waiting {
        state.rest.fg_mut().status = "busy — wait for response".into();
        state.rest.fg_mut().pending_plan_seed = false;
        state.rest.fg_mut().pending_plan_seed_body = None;
        state.rest.fg_mut().pending_mission_seed = None;
        return Ok(());
    }
    if client.is_none() || state.rest.fg().session.is_none() {
        state.rest.fg_mut().status = "no active session".into();
        state.rest.fg_mut().pending_plan_seed = false;
        state.rest.fg_mut().pending_plan_seed_body = None;
        state.rest.fg_mut().pending_mission_seed = None;
        return Ok(());
    }
    let Some(sess) = state.rest.fg().session.as_ref() else {
        crate::model::store::append_global_error_log("compact", "BUG: fg session missing");
        state.rest.fg_mut().pending_plan_seed = false;
        state.rest.fg_mut().pending_plan_seed_body = None;
        state.rest.fg_mut().pending_mission_seed = None;
        return Ok(());
    };
    let pn = preserve_n_override.unwrap_or(sess.settings.compaction.preserve_n);
    let (to_sum, kept_tail) = sess.conversation.split_for_compaction(pn);
    if to_sum.is_empty() {
        state.rest.fg_mut().status = "nothing to compact".into();
        state.rest.fg_mut().pending_plan_seed = false;
        state.rest.fg_mut().pending_plan_seed_body = None;
        state.rest.fg_mut().pending_mission_seed = None;
        return Ok(());
    }
    let mut req = vec![ChatMessage::new(
        Role::System,
        "You are compacting a conversation to free up context. Write a concise SUMMARY of the conversation above for your own future reference — NOT a reply to the user. Capture: what the user is building or asking for; key decisions, facts, and constraints established; the current state; specific files, code, names, and values that matter; and any open threads or next steps. Use short labeled sections or terse bullet points. Be factual. Do not greet, do not continue the task, do not address the user.",
    )];
    req.extend(to_sum);
    state.rest.fg_mut().waiting = true;
    state.rest.fg_mut().status = "compacting...".into();
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
        crate::app::resolve::resolve_role_dispatch(
            &state.rest.config,
            &s.settings,
            crate::model::app_config::ModelRole::Compactor,
        )
    });
    // Fresh channel for this request; the receiver lives in state so an
    // interrupt/new just drops it and the task's result is ignored.
    let (tx, rx) = mpsc::unbounded_channel();
    state.rest.fg_mut().active_rx = Some(rx);
    let Some(c) = client.as_ref().cloned() else {
        crate::model::store::append_global_error_log("compact", "BUG: client missing");
        state.rest.fg_mut().pending_plan_seed = false;
        state.rest.fg_mut().pending_plan_seed_body = None;
        state.rest.fg_mut().pending_mission_seed = None;
        return Ok(());
    };
    let jh = handle.spawn(async move {
        // Compaction sends on the resolved Compactor connection (endpoint +
        // key) with its model id + upstream-route slug; no effort (the
        // summary is mechanical).
        let result = match route {
            Some(r) => match tokio::time::timeout(
                std::time::Duration::from_secs(120),
                c.complete(r.conn(), &r.model_id, r.provider(), req),
            )
            .await
            {
                Ok(result) => result,
                Err(_) => Err(anyhow::anyhow!("compactor timeout")),
            },
            None => Err(anyhow::anyhow!("no active session")),
        };
        let result = result.and_then(|summary| {
            crate::model::conversation::clean_compaction_summary(&summary)
                .ok_or_else(|| anyhow::anyhow!("compactor returned an empty summary"))
        });
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::harness::regression_test::{fixture, response};
    use crate::model::{
        app_config::ModelRole, conversation::Conversation, session::Session, settings::Settings,
    };

    #[tokio::test]
    async fn null_compactor_response_emits_error_without_changing_history_or_trying_main() {
        let (mut config, requests) =
            fixture(vec![(response(serde_json::Value::Null, "stop"), 0)], true).await;
        config.models[0].roles = vec![ModelRole::Compactor];
        let mut state = AppState::new(crate::app::mode::Mode::Chat);
        state.rest.config = config;
        let conversation = Conversation::from_messages(vec![
            ChatMessage::new(Role::System, "system"),
            ChatMessage::new(Role::User, "preserve this conversation"),
        ]);
        let before = conversation.messages().to_vec();
        let path = std::env::temp_dir().join(format!("koma-compact-null-{}", uuid::Uuid::new_v4()));
        state.rest.fg_mut().session = Some(Session::new(
            "compact-null".into(),
            path.clone(),
            "pwd".into(),
            Settings::default(),
            conversation,
        ));
        let mut client = Some(Arc::new(OpenRouterClient::new()));
        handle_compact(
            &mut state,
            &mut client,
            &tokio::runtime::Handle::current(),
            Some(0),
        )
        .unwrap();
        let event = state
            .rest
            .fg_mut()
            .active_rx
            .as_mut()
            .unwrap()
            .recv()
            .await
            .unwrap();
        assert!(matches!(event, StreamEvent::Error(ref error) if error.contains("empty summary")));
        assert_eq!(
            state
                .rest
                .fg()
                .session
                .as_ref()
                .unwrap()
                .conversation
                .messages(),
            before
        );
        assert!(!path.exists());
        assert_eq!(requests.await.unwrap().len(), 1);
    }
}
