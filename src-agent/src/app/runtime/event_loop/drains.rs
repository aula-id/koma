//! Private drain helpers for the central event loop.
//!
//! Functions here are called only from [`super::run_loop`]; they are `pub(super)`
//! so they cross the module boundary without leaking into the crate public API.

use std::io::{stdout, Write};
use std::sync::Arc;

use anyhow::Result;
use ratatui::crossterm::event::DisableMouseCapture;
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::crossterm::event::EnableMouseCapture;

use crate::app::state::AppState;
use crate::dto::chat::Role;
use crate::service::openrouter::OpenRouterClient;

use super::Term;

/// Leave the alternate screen + disable mouse capture, then print the full
/// conversation as plain text so the user can select/copy with the terminal's
/// native selection. Raw mode stays on (we read a single key to return), so
/// lines are terminated with `\r\n`.
pub(super) fn enter_select(rest: &crate::app::state::AppStateRest) -> Result<()> {
    execute!(stdout(), LeaveAlternateScreen, DisableMouseCapture)?;
    let mut out = stdout();
    if let Some(sess) = rest.fg().session.as_ref() {
        for m in sess.conversation.messages() {
            let label = match m.role {
                Role::System | Role::Tool => continue,
                Role::User => "you",
                Role::Assistant => "ai",
            };
            write!(out, "\r\n{label}:\r\n")?;
            for line in m.content.split('\n') {
                write!(out, "{line}\r\n")?;
            }
        }
    }
    write!(out, "\r\n-- copy with your mouse, then press any key to return --\r\n")?;
    out.flush()?;
    Ok(())
}

/// Re-enter the alternate screen + mouse capture and force a full repaint.
pub(super) fn exit_select(terminal: &mut Term) -> Result<()> {
    execute!(stdout(), EnterAlternateScreen, EnableMouseCapture)?;
    terminal.clear()?;
    Ok(())
}

/// Apply a finished compaction to the active session and finalize the UI.
///
/// This is the single apply path shared by both the immediate case (the model
/// already took >= the minimum animation time) and the deferred case (a fast
/// compaction held back by [`super::MIN_COMPACT_ANIM`]). It:
/// - rebuilds the conversation (`apply_compaction` + `rebuild_system`) and saves,
/// - refreshes the project-awareness summary (best-effort, gated by the setting),
/// - invalidates the transcript cache so the same-length REPLACE doesn't leave a
///   stale prefix (the summary is the new first block),
/// - scrolls to the TOP so the user sees the fresh summary,
/// - surfaces the summary text as a neutral (info) toast under the finish, and
/// - clears the waiting/animation state.
pub(super) fn apply_compaction_result(
    state: &mut AppState,
    client: &Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
    summary: String,
    kept_tail: Vec<crate::dto::chat::ChatMessage>,
) {
    // Some summariser models (gpt-oss / harmony) emit inline tool-call markup
    // (`<tool_call>…</tool_call>`, `<function=…>`, `<parameter=…>`) inside their
    // text. Strip it here so the raw XML never lands in the stored summary
    // message, the wire history, or the toast — the same scrub the streaming /
    // final-answer paths already apply to assistant content.
    let summary = crate::dto::chat::strip_tool_call_tags(&summary);

    if let Some(sess) = state.rest.fg_mut().session.as_mut() {
        sess.conversation
            .apply_compaction(summary.clone(), kept_tail);
        sess.rebuild_system();
        let _ = sess.save();
    }
    // Refresh the project-awareness summary post-compaction: the project is often
    // better understood after a compact, and this also satisfies the "applies on
    // compaction" requirement. Best-effort; gated by `awareness_enabled` inside
    // `summarize`. Clone the inputs out first (including `config` for the role
    // resolution) so the `block_on` doesn't hold a borrow of `state.rest`.
    let aware_inputs = match (client.as_ref(), state.rest.fg().session.as_ref()) {
        (Some(c), Some(sess)) if sess.settings.awareness_enabled => Some((
            Arc::clone(c),
            state.rest.config.clone(),
            sess.settings.clone(),
            sess.workdir(),
        )),
        _ => None,
    };
    if let Some((c, config, settings, workdir)) = aware_inputs {
        // Resolve the Awareness role (endpoint + key + model + upstream-route slug)
        // for the summary call; Awareness always resolves, but guard defensively.
        // Also resolve the Main route as a fallback: when the Awareness model call
        // itself fails (e.g. bad/typo'd model name) we retry once on the trusted
        // Main route before giving up.
        if let Some(r) = crate::app::resolve::resolve_role(
            &config,
            &settings,
            crate::model::app_config::ModelRole::Awareness,
        ) {
            let main_route = crate::app::resolve::resolve_role(
                &config,
                &settings,
                crate::model::app_config::ModelRole::Main,
            );
            let s = match main_route {
                Some(ref m) => handle.block_on(crate::app::awareness::summarize_with_fallback(
                    &c,
                    &settings,
                    r.conn(),
                    &r.model_id,
                    r.provider(),
                    &workdir,
                    m.conn(),
                    &m.model_id,
                    m.provider(),
                )),
                None => handle.block_on(crate::app::awareness::summarize(
                    &c,
                    &settings,
                    r.conn(),
                    &r.model_id,
                    r.provider(),
                    &workdir,
                )),
            };
            state.rest.fg_mut().awareness_summary = s;
        }
    }

    // The transcript cache only rebuilds on a length SHRINK; compaction can be a
    // same-length REPLACE, which would leave a stale prefix. Force a full rebuild
    // so the new summary (first Assistant block) + kept tail render correctly.
    state.rest.transcript_cache.borrow_mut().blocks.clear();
    // Jump to the top of the transcript so the freshly-written summary is what the
    // user sees once the animation clears (instead of the kept tail at the bottom).
    // Transcript view state lives on the foreground session now.
    {
        let fg = state.rest.fg_mut();
        fg.follow = false;
        fg.scroll = 0;
    }

    // Surface the generated summary "under the finish animation" as a neutral,
    // multi-line info toast (capped so a long summary stays contained).
    state
        .rest
        .set_toast_info(format!("compacted ✓\n{}", cap_summary(&summary, 400)));

    state.rest.fg_mut().waiting = false;
    state.rest.status = "ready".into();
    // Animation is done: stop the per-tick redraw + drop any deferral bookkeeping.
    state.rest.compact_anim_start = None;
    state.rest.compact_apply_at = None;
    state.rest.compact_pending = None;
}

/// Trim and cap a summary for toast display: collapse leading/trailing
/// whitespace, then keep at most `max` characters, appending an ellipsis when cut.
pub(super) fn cap_summary(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max).collect();
        out.push('…');
        out
    }
}
