//! Sub-agents panel rendering (the `$` overlay) and the helper functions for
//! status tag/line formatting used both here and in the panel.

use ratatui::{
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};
use crate::app::state::AppStateRest;
use crate::view::theme::Palette;
use super::helpers::truncate_chars;
use super::transcript::assemble_messages;

/// Short status tag/glyph for a sub-agent, shown in the panel's left list.
pub(super) fn subagent_tag(status: &crate::app::subagent::SubAgentStatus) -> &'static str {
    use crate::app::subagent::SubAgentStatus;
    match status {
        SubAgentStatus::Running => "running",
        SubAgentStatus::Done(_) => "done",
        SubAgentStatus::Killed => "killed",
        SubAgentStatus::Error(_) => "error",
    }
}

/// Status line for a sub-agent, shown at the top of the panel's right pane.
pub(super) fn subagent_status_line(status: &crate::app::subagent::SubAgentStatus) -> String {
    use crate::app::subagent::SubAgentStatus;
    match status {
        SubAgentStatus::Running => "running…".to_string(),
        SubAgentStatus::Done(answer) => format!("done · {}", truncate_chars(answer, 60)),
        SubAgentStatus::Killed => "killed".to_string(),
        SubAgentStatus::Error(e) => format!("error · {}", truncate_chars(e, 60)),
    }
}

/// Render the sub-agents panel overlay (opened with `$`) into a popup anchored
/// just above `input_chunk`.
///
/// A bordered popup above the input (same rect math as the help overlay), split
/// into a narrow left list of active sub-agents (RIGHT border, like the settings
/// sidebar) and a wide right pane showing the selected one's live progress.
/// Modal: keys are routed to the sub-agent handler in the input controller.
/// Ctrl+X kills the selection.
pub(super) fn render_subagents_panel(
    frame: &mut Frame,
    input_chunk: Rect,
    transcript_chunk: Rect,
    rest: &AppStateRest,
    palette: &Palette,
) {
    // Box sizing: up to ~12 rows, clamped to the space above the input.
    let avail = input_chunk.y.saturating_sub(transcript_chunk.y);
    let h = 12u16.min(avail.max(3));
    let y = input_chunk.y.saturating_sub(h);
    let rect = Rect { x: input_chunk.x, y, width: input_chunk.width, height: h };

    let block = Block::bordered()
        .border_style(Style::default().fg(palette.dim))
        .title(Span::styled(" sub-agents ", Style::default().fg(palette.dim)));
    let inner = block.inner(rect);
    frame.render_widget(Clear, rect);
    frame.render_widget(block, rect);

    if inner.width == 0 || inner.height == 0 {
        // Nothing fits — the bordered box itself is the whole signal.
    } else if rest.fg().subagents.is_empty() && rest.fg().pending_subagents.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "(no active sub-agents)",
                Style::default().fg(palette.dim),
            ))
            .style(Style::default()),
            inner.inner(Margin { horizontal: 1, vertical: 0 }),
        );
    } else {
        // Two-pane split: a ~24-col left list (RIGHT border divider, like the
        // settings sidebar) + a wide right progress pane.
        const LIST_W: u16 = 24;
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(LIST_W), Constraint::Min(0)])
            .split(inner);

        // LEFT: one row per sub-agent — "#id name tag" + truncated label. The
        // row at `subagent_sel` gets the sel_fg/sel_bg highlight.
        let list_block = Block::new()
            .borders(Borders::RIGHT)
            .border_style(Style::default().fg(palette.dim));
        let list_inner = list_block.inner(cols[0]);
        frame.render_widget(list_block, cols[0]);

        let sel = rest.subagent_sel.min(rest.fg().subagents.len().saturating_sub(1));
        let list_w = list_inner.width as usize;
        let mut list_lines: Vec<Line> = rest
            .fg()
            .subagents
            .iter()
            .enumerate()
            .map(|(i, sa)| {
                let tag = subagent_tag(&sa.status);
                let head = format!("#{} {} {}", sa.id, sa.agent_name, tag);
                let label = truncate_chars(&sa.label, list_w.saturating_sub(head.chars().count() + 1).max(1));
                let text = format!("{head} {label}");
                if i == sel {
                    Line::from(Span::styled(
                        format!("{:<width$}", truncate_chars(&text, list_w), width = list_w),
                        Style::default().fg(palette.sel_fg).bg(palette.sel_bg),
                    ))
                } else {
                    Line::from(vec![
                        Span::styled(format!("#{} ", sa.id), Style::default().fg(palette.accent)),
                        Span::styled(
                            truncate_chars(
                                &format!("{} {} {}", sa.agent_name, tag, sa.label),
                                list_w.saturating_sub(2 + sa.id.to_string().chars().count()).max(1),
                            ),
                            Style::default().fg(palette.fg),
                        ),
                    ])
                }
            })
            .collect();
        // QUEUED delegations (not yet running) listed AFTER the live/done rows,
        // tagged "pending" and rendered fully dim so they read as not-yet-active.
        // They are not selectable here (no messages yet) — S3 owns that; for now
        // they only show the id + agent + truncated prompt.
        for p in &rest.fg().pending_subagents {
            let body = format!("{} pending {}", p.agent_name, p.prompt);
            list_lines.push(Line::from(vec![
                Span::styled(format!("#{} ", p.id), Style::default().fg(palette.dim)),
                Span::styled(
                    truncate_chars(
                        &body,
                        list_w.saturating_sub(2 + p.id.to_string().chars().count()).max(1),
                    ),
                    Style::default().fg(palette.dim),
                ),
            ]));
        }
        frame.render_widget(Paragraph::new(list_lines), list_inner);

        // RIGHT: the selected sub-agent's status line + the trailing transcript
        // lines that fit. Inset 1 col on the left so it doesn't hug the divider.
        // When ONLY pending entries exist (no spawned sub-agent yet) there is
        // nothing to select, so show a neutral note instead of indexing an empty
        // list.
        let right = cols[1].inner(Margin { horizontal: 1, vertical: 0 });
        if right.width > 0 && right.height > 0 && rest.fg().subagents.is_empty() {
            frame.render_widget(
                Paragraph::new(Span::styled(
                    "(sub-agents queued — waiting for a free slot)",
                    Style::default().fg(palette.dim),
                )),
                right,
            );
        } else if right.width > 0 && right.height > 0 {
            let sa = &rest.fg().subagents[sel];
            let mut rows: Vec<Line> = Vec::new();
            rows.push(Line::from(Span::styled(
                subagent_status_line(&sa.status),
                Style::default().fg(palette.accent),
            )));
            // Last transcript lines that fit (after the status row).
            let budget = (right.height as usize).saturating_sub(1);
            if budget > 0 {
                let start = sa.transcript.len().saturating_sub(budget);
                for line in &sa.transcript[start..] {
                    rows.push(Line::from(Span::styled(
                        truncate_chars(line, right.width as usize),
                        Style::default().fg(palette.dim),
                    )));
                }
            }
            frame.render_widget(Paragraph::new(rows), right);
        }
    }
}

/// Render the FULL-SCREEN sub-agent viewer over the whole frame (opened with
/// Enter on a spawned `$`-panel row). Short-circuits the normal chat draw, like
/// the nano prompt editor.
///
/// Layout (minimalist, matching the app's header convention):
///
/// ```text
///  sub-agent #3 explore (running…)
/// ─────────────────────────────────────────  ← dim BOTTOM rule
///   ★ find the auth middleware                ← the sub-agent's conversation,
///   ● I located it in src/mw/auth.rs …          rendered EXACTLY like the main
///   …                                           chat (markdown, tool lines, …)
///
///  up/down scroll · Esc back                  ← dim footer
/// ```
///
/// The body reuses the main-chat transcript renderer ([`assemble_messages`]), so
/// the sub-agent view looks identical to the main chat. While the agent is
/// `Running` the view auto-follows the bottom so live progress shows; once it
/// stops, Up/Down/PgUp scroll freely (clamped to the content). The renderer stays
/// pure: it derives the top line from `agent_viewer_scroll` + the Running follow,
/// and publishes the max scroll into the shared `last_max_scroll` cell (reused
/// here since the viewer and the main transcript are never drawn together) so the
/// key handler can clamp + detect bottom.
pub(super) fn render_agent_viewer(
    frame: &mut Frame,
    rest: &AppStateRest,
    idx: usize,
    palette: &Palette,
) {
    let area = frame.area();
    let Some(sa) = rest.fg().subagents.get(idx) else {
        return; // index went stale — nothing to show (handler resets next key).
    };

    // Header (title + BOTTOM rule) | body | footer.
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // title line + BOTTOM border
            Constraint::Min(0),    // transcript body
            Constraint::Length(1), // footer hint
        ])
        .split(area);

    // --- Header: "sub-agent #id name (status)" (dim) with a BOTTOM rule. ---
    let header_block = Block::new()
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(palette.dim));
    let header_inner = header_block.inner(outer[0]);
    frame.render_widget(header_block, outer[0]);
    let title = format!(
        "sub-agent #{} {} ({})",
        sa.id,
        sa.agent_name,
        subagent_tag(&sa.status),
    );
    frame.render_widget(
        Paragraph::new(Span::styled(title, Style::default().fg(palette.dim))),
        header_inner.inner(Margin { horizontal: 2, vertical: 0 }),
    );

    // --- Footer hint (dim). ---
    frame.render_widget(
        Paragraph::new(Span::styled(
            "up/down scroll \u{b7} Esc back",
            Style::default().fg(palette.dim),
        )),
        outer[2].inner(Margin { horizontal: 2, vertical: 0 }),
    );

    // --- Body: same 2-col horizontal margin + wrap width as the main chat, so
    // the reused renderer wraps identically. ---
    let body = outer[1].inner(Margin { horizontal: 2, vertical: 0 });
    if body.width == 0 || body.height == 0 {
        return;
    }
    let wrap_w = (body.width as usize).saturating_sub(2).max(1);

    // Render the sub-agent's structured conversation through the SHARED main-chat
    // transcript path (markdown bodies, reasoning/thinking blocks, ⚙/✓ tool lines).
    let lines = assemble_messages(&sa.messages, palette, wrap_w);

    // Scroll model: while the agent is RUNNING auto-follow the bottom so live
    // progress shows; otherwise honour the stored offset, clamped. Publish the max
    // so the key handler can clamp + detect bottom (same cell-publish pattern the
    // main transcript uses for `last_max_scroll`).
    let total = u16::try_from(lines.len()).unwrap_or(u16::MAX);
    let max_scroll = total.saturating_sub(body.height);
    rest.last_max_scroll.set(max_scroll);
    let top = if rest.agent_viewer_follow {
        max_scroll
    } else {
        rest.agent_viewer_scroll.min(max_scroll)
    };
    frame.render_widget(Paragraph::new(lines).scroll((top, 0)), body);
}
