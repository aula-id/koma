//! View – `/model` session model switcher overlay.
//!
//! A popup above the composer (chat stays visible), matching the `/bash`
//! overlay pattern. Layout:
//!
//! 1. Top+bottom rule title bar — title ` model switcher ` on the TOP rule.
//! 2. Flat option list (or help lines for Help submode) with cursor highlight.
//! 3. Note line (dim) — context-sensitive help/error.
//! 4. Keybinding hint line.

use crate::app::mode::{ModelCmdState, ModelCmdSub};
use crate::model::app_config::ModelRole;
use crate::view::theme::Palette;
use ratatui::{
    layout::{Constraint, Direction, Layout, Margin},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph},
    Frame,
};

/// Render the model command overlay inside the given `popup_area` (typically
/// `chunks[4]` = the input box area) over the `chat_area` (typically
/// `chunks[1]` = the transcript area).
pub fn render_overlay(
    frame: &mut Frame,
    state: &ModelCmdState,
    palette: &Palette,
    popup_area: ratatui::layout::Rect,
    _chat_area: ratatui::layout::Rect,
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

    // Calculate needed height: title (3) + options + note (1) + hint (1)
    let option_count = if state.options.is_empty() {
        match &state.sub {
            ModelCmdSub::Help { lines } => lines.len(),
            _ => 1,
        }
    } else {
        state.options.len()
    };
    let height = (option_count.min(12) + 5) as u16; // title + options + note + hint
    let height = height.min(popup_area.height);

    // Split popup_area to get a centered region.
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),                    // space above
            Constraint::Length(height),            // popup
            Constraint::Min(0),                    // space below
        ])
        .split(popup_area);

    let popup = chunks[1];

    let inner_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // title: top+bottom rules
            Constraint::Min(1),   // flat option list / help lines
            Constraint::Length(1), // note
            Constraint::Length(1), // hint
        ])
        .split(popup);

    // --- Title bar ---
    let title_block = Block::new()
        .borders(Borders::TOP | Borders::BOTTOM)
        .border_style(Style::default().fg(palette.dim))
        .title(Span::styled(
            title_str,
            Style::default().fg(palette.dim),
        ))
        .padding(Padding::horizontal(1));
    frame.render_widget(title_block, inner_chunks[0]);

    // --- Content area ---
    let inner = inner_chunks[1].inner(Margin {
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
            frame.render_widget(Paragraph::new(styled_lines), inner);
        }
        _ => {
            let lines: Vec<Line> = state
                .options
                .iter()
                .enumerate()
                .map(|(i, (uuid, label))| {
                    let display = if uuid.is_none() {
                        // Inherit row — dim
                        format!(" {label} ")
                    } else {
                        format!(" {label} ")
                    };
                    if i == state.cursor {
                        let hl =
                            Style::default().fg(palette.sel_fg).bg(palette.sel_bg);
                        Line::from(Span::styled(display, hl))
                    } else {
                        Line::from(Span::styled(
                            display,
                            Style::default().fg(palette.accent),
                        ))
                    }
                })
                .collect();

            // Scroll so cursor stays visible.
            let list_height = inner.height as usize;
            let sel = state.cursor.min(state.options.len().saturating_sub(1));
            let scroll_offset = if list_height > 0 && sel >= list_height {
                (sel - list_height + 1) as u16
            } else {
                0
            };
            frame.render_widget(
                Paragraph::new(lines).scroll((scroll_offset, 0)),
                inner,
            );
        }
    }

    // --- Note (dim) ---
    let note_area = inner_chunks[2].inner(Margin {
        horizontal: 1,
        vertical: 0,
    });
    frame.render_widget(
        Paragraph::new(state.note.as_str())
            .style(Style::default().fg(palette.dim)),
        note_area,
    );

    // --- Keybinding hint ---
    let hint_area = inner_chunks[3].inner(Margin {
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
