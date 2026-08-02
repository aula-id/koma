//! View – `/model` session model switcher overlay.
//!
//! A popup anchored above the composer (chat stays visible), matching the
//! `/bash`, `/todo`, `$` sub-agents, and settings menu overlay pattern.
//! Layout: bordered block with title, option/help list, note, keybinding hint.

use crate::app::mode::{ModelCmdState, ModelCmdSub};
use crate::model::app_config::ModelRole;
use crate::view::theme::Palette;
use ratatui::{
    layout::{Constraint, Direction, Layout, Margin},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Paragraph},
    Frame,
};

/// Render the model command overlay anchored above `input_chunk` on top of
/// `transcript_chunk`, following the same geometry as bash/todo/subagents.
pub fn render_overlay(
    frame: &mut Frame,
    state: &ModelCmdState,
    palette: &Palette,
    input_chunk: ratatui::layout::Rect,
    transcript_chunk: ratatui::layout::Rect,
) {
    let title_str = match &state.sub {
        ModelCmdSub::Help { .. } => " model — help ".to_string(),
        ModelCmdSub::RolePick { role } => match role {
            ModelRole::Main => " model — main ".to_string(),
            ModelRole::Awareness => " model — awareness ".to_string(),
            ModelRole::Planner => " model — planner ".to_string(),
            ModelRole::Compactor => " model — compactor ".to_string(),
            ModelRole::Safeguard => " model — safeguard ".to_string(),
        },
        ModelCmdSub::AgentList => " model — agents ".to_string(),
        ModelCmdSub::AgentPick { agent_name, .. } => {
            format!(" model — agent: {agent_name} ")
        }
    };

    // Content rows: option/help list, capped at 12 with scroll.
    let content_rows = match &state.sub {
        ModelCmdSub::Help { lines } => lines.len(),
        _ => {
            if state.options.is_empty() {
                1 // empty placeholder
            } else {
                state.options.len()
            }
        }
    };
    let list_rows = content_rows.min(12) as u16;

    // Desired height: list + 2 (bordered block top/bottom) + 1 note + 1 hint.
    let has_note = !state.note.is_empty();
    let desired = list_rows + 2 + if has_note { 1 } else { 0 } + 1; // hint always

    // Anchor above the input bar, growing upward into transcript space.
    let avail = input_chunk.y.saturating_sub(transcript_chunk.y);
    let h = desired.min(avail.max(3));
    let y = input_chunk.y.saturating_sub(h);
    let rect = ratatui::layout::Rect {
        x: input_chunk.x,
        y,
        width: input_chunk.width,
        height: h,
    };

    let block = Block::bordered()
        .border_style(Style::default().fg(palette.dim))
        .title(Span::styled(
            title_str,
            Style::default().fg(palette.dim),
        ));
    let inner = block.inner(rect);
    crate::view::clear_and_fill(frame, rect, palette.bg);
    frame.render_widget(block, rect);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    // Layout: list (fills remaining), optional note (1), hint (1).
    let mut constraints = vec![Constraint::Min(1)]; // option/help list
    if has_note && h > list_rows + 2 + 1 {
        // room for at least: list + border + hint + note
        constraints.push(Constraint::Length(1)); // note
    }
    constraints.push(Constraint::Length(1)); // hint

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    // --- Content area (option list / help) ---
    let content_area = chunks[0].inner(Margin {
        horizontal: 1,
        vertical: 0,
    });

    match &state.sub {
        ModelCmdSub::Help { lines } => {
            let styled_lines: Vec<Line> = lines
                .iter()
                .map(|l| {
                    Line::from(Span::styled(
                        format!(" {l} "),
                        Style::default().fg(palette.accent),
                    ))
                })
                .collect();
            frame.render_widget(Paragraph::new(styled_lines), content_area);
        }
        _ => {
            if state.options.is_empty() {
                frame.render_widget(
                    Paragraph::new(Span::styled(
                        " (no options available) ",
                        Style::default().fg(palette.dim),
                    )),
                    content_area,
                );
            } else {
                let lines: Vec<Line> = state
                    .options
                    .iter()
                    .enumerate()
                    .map(|(i, (uuid, label))| {
                        let display = format!(" {label} ");
                        if i == state.cursor {
                            // Pad selected row to full inner width to avoid
                            // ratatui highlight bleed (known project issue).
                            let pad_w = content_area.width as usize;
                            let text_w = display.len();
                            let padded = if text_w < pad_w {
                                format!("{display:<pad_w$}")
                            } else {
                                display
                            };
                            let hl = Style::default()
                                .fg(palette.sel_fg)
                                .bg(palette.sel_bg);
                            Line::from(Span::styled(padded, hl))
                        } else {
                            let style = if uuid.is_none() {
                                // Inherit row — dim.
                                Style::default().fg(palette.dim)
                            } else {
                                Style::default().fg(palette.accent)
                            };
                            Line::from(Span::styled(display, style))
                        }
                    })
                    .collect();

                // Scroll so cursor stays visible.
                let list_height = content_area.height as usize;
                let sel = state.cursor.min(state.options.len().saturating_sub(1));
                let scroll_offset = if list_height > 0 && sel >= list_height {
                    (sel - list_height + 1) as u16
                } else {
                    0
                };
                frame.render_widget(
                    Paragraph::new(lines).scroll((scroll_offset, 0)),
                    content_area,
                );
            }
        }
    }

    // --- Note (dim) — offset index shifts when note is present ---
    let note_idx = if has_note && chunks.len() > 2 { 1 } else { 0 };
    if has_note {
        let note_area = chunks[note_idx].inner(Margin {
            horizontal: 1,
            vertical: 0,
        });
        frame.render_widget(
            Paragraph::new(state.note.as_str())
                .style(Style::default().fg(palette.dim)),
            note_area,
        );
    }

    // --- Keybinding hint ---
    let hint_idx = chunks.len() - 1;
    let hint_area = chunks[hint_idx].inner(Margin {
        horizontal: 1,
        vertical: 0,
    });
    let hint = match &state.sub {
        ModelCmdSub::Help { .. } => "Esc close",
        ModelCmdSub::AgentList => "↑↓ select · Enter pick model · Esc cancel",
        _ => "↑↓ select · Enter apply · Esc cancel",
    };
    frame.render_widget(
        Paragraph::new(hint).style(Style::default().fg(palette.dim)),
        hint_area,
    );
}
