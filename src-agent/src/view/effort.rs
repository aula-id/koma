//! View – `/effort` reasoning-effort picker (Effort mode).
//!
//! A small overlay listing the thinking-effort options the current model
//! supports. Layout (top to bottom):
//!
//! 1. Top+bottom rule title bar — title ` reasoning effort ` on the TOP rule.
//! 2. Flat (borderless) option list — the selected row highlighted with
//!    `palette.sel_fg` on `palette.sel_bg`, the rest in accent.
//! 3. The capability note (dim) plus a one-line keybinding hint.
//!
//! Selection state lives in [`app::mode::EffortPickerState`]; keystroke handling
//! lives in [`controller::input::handle_effort`]. The contained box mirrors the
//! `@`/command palette look (the allowed bordered exception to the otherwise
//! borderless, top-down UI).

use ratatui::{
    layout::{Constraint, Direction, Layout, Margin},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph},
    Frame,
};
use crate::app::mode::EffortPickerState;
use crate::app::state::AppStateRest;
use crate::view::theme::Palette;

/// Render the effort picker for `picker` using the given colour `palette`.
pub fn draw(frame: &mut Frame, rest: &AppStateRest, picker: &EffortPickerState, palette: &Palette) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // title: top+bottom rules
            Constraint::Min(1),    // flat option list
            Constraint::Length(1), // capability note
            Constraint::Length(1), // keybinding hint line
        ])
        .split(frame.area());

    // --- Title bar ---
    // Top+bottom rules only — title " reasoning effort " sits on the TOP rule.
    let title_block = Block::new()
        .borders(Borders::TOP | Borders::BOTTOM)
        .border_style(Style::default().fg(palette.dim))
        .title(Span::styled(
            " reasoning effort ",
            Style::default().fg(palette.dim),
        ))
        .padding(Padding::horizontal(1));
    frame.render_widget(title_block, chunks[0]);

    // --- Option list (flat, no borders) ---
    let inner = chunks[1].inner(Margin { horizontal: 1, vertical: 0 });
    let lines: Vec<Line> = picker
        .options
        .iter()
        .enumerate()
        .map(|(i, opt)| {
            if i == picker.selected {
                let hl = Style::default().fg(palette.sel_fg).bg(palette.sel_bg);
                Line::from(Span::styled(format!(" {opt} "), hl))
            } else {
                Line::from(Span::styled(
                    format!(" {opt} "),
                    Style::default().fg(palette.accent),
                ))
            }
        })
        .collect();

    // Scroll so the selected row stays visible within the inner height
    // (scrolloff-walk within the window; one row per option → start == offset).
    let list_height = inner.height as usize;
    let sel = picker.selected.min(picker.options.len().saturating_sub(1));
    let (start, _) =
        crate::view::scroll::scroll_window(&rest.effort_offset, sel, picker.options.len(), list_height);
    frame.render_widget(Paragraph::new(lines).scroll((start as u16, 0)), inner);

    // --- Capability note (dim) ---
    let note_area = chunks[2].inner(Margin { horizontal: 1, vertical: 0 });
    frame.render_widget(
        Paragraph::new(picker.note.as_str()).style(Style::default().fg(palette.dim)),
        note_area,
    );

    // --- Keybinding hint ---
    let hint_area = chunks[3].inner(Margin { horizontal: 1, vertical: 0 });
    let hint = "↑↓ select · Enter apply · Esc cancel · Ctrl+C quit";
    frame.render_widget(
        Paragraph::new(hint).style(Style::default().fg(palette.dim)),
        hint_area,
    );
}
