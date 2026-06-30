//! View – quit-confirm overlay (`Mode::QuitConfirm`).
//!
//! Shown always when the user asks to quit — regardless of whether any session
//! has work in flight. Flat, boxless layout (top+bottom rules only, matching the
//! session-hub + session-picker views + the repo border convention) — top to bottom:
//!
//! 1. Top+bottom rule title bar — ` quit ` on the TOP rule.
//! 2. A clean question line ("Do you want to quit?"); when work is in flight a
//!    dim sub-line warns that in-flight work will be lost.
//! 3. A navigable horizontal BUTTON ROW: `[quit & kill]  [minimize]  [cancel]`.
//!    The focused button (index `s.selected`) is highlighted; the others are
//!    subdued. Each button is laid out as a chip and its on-screen
//!    [`ratatui::layout::Rect`] is recorded into [`QuitConfirmState::button_rects`]
//!    in index order so the event loop can hit-test a left-click.
//! 4. A one-line description of the FOCUSED button.
//!
//! Navigation (Left/Right, Tab/Shift+Tab, Enter) plus the direct k / d / Esc
//! shortcuts are handled in [`crate::controller::input::handle_quit_confirm`].

use ratatui::{
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph},
    Frame,
};
use crate::app::mode::QuitConfirmState;
use crate::view::theme::Palette;

/// The three button labels, left→right, in `button_rects`/`selected` index order
/// (`0` = quit & kill, `1` = minimize, `2` = cancel). The chip is the label wrapped
/// in literal brackets (`[quit & kill]`) — koma button style — so the chip width is
/// `label.len() + 2`, matching the click-rect math below.
const LABELS: [&str; 3] = ["quit & kill", "minimize", "cancel"];

/// One-line description for each button, same index order as [`LABELS`].
const DESCS: [&str; 3] = [
    "Abort every session's in-flight work, then exit koma.",
    "Save each conversation to disk to resume later, then exit koma. \
     In-flight work still stops on exit.",
    "Back to chat — keep everything running.",
];

/// Gap (in columns) rendered between adjacent buttons in the row.
const GAP: u16 = 3;

/// Render the quit-confirm overlay for `s` using the given colour `palette`.
pub fn draw(frame: &mut Frame, s: &QuitConfirmState, palette: &Palette) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // title: top+bottom rules
            Constraint::Min(1),    // question + button row + description
            Constraint::Length(1), // keybinding hint line
        ])
        .split(frame.area());

    // --- Title bar ---
    // Top+bottom rules only — title sits on the TOP rule, dim style.
    let title_block = Block::new()
        .borders(Borders::TOP | Borders::BOTTOM)
        .border_style(Style::default().fg(palette.dim))
        .title(Span::styled(" quit ", Style::default().fg(palette.dim)))
        .padding(Padding::horizontal(1));
    let title_inner = title_block.inner(chunks[0]);
    let subtitle = if s.working > 0 {
        "a quit was requested while work is still in flight"
    } else {
        "a quit was requested"
    };
    let note = Line::from(Span::styled(subtitle, Style::default().fg(palette.dim)));
    frame.render_widget(title_block, chunks[0]);
    frame.render_widget(Paragraph::new(note), title_inner);

    // --- Body: question + button row + focused-button description ---
    let inner = chunks[1].inner(Margin { horizontal: 1, vertical: 0 });

    // Clamp the focused index defensively so an out-of-range value (shouldn't
    // happen) never panics on array indexing below.
    let sel = s.selected.min(2);

    // Build the chip Span for a button: the label wrapped in literal brackets
    // (`[like this]`, koma button style), rendered highlighted when focused (reversed
    // onto the accent colour, BOLD — the brackets stay visible as part of the chip
    // text) or subdued (dim) otherwise. `sel_fg` is the on-accent foreground
    // (true-black/white), legible under BOLD — matching the footer + selection
    // inverse treatment.
    let chip = |idx: usize| {
        let label = format!("[{}]", LABELS[idx]);
        let style = if idx == sel {
            Style::default()
                .bg(palette.accent)
                .fg(palette.sel_fg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(palette.dim)
        };
        Span::styled(label, style)
    };

    // The button row, laid out left→right with `GAP` columns between chips.
    let mut row_spans: Vec<Span> = Vec::with_capacity(5);
    for idx in 0..3 {
        if idx > 0 {
            row_spans.push(Span::raw(" ".repeat(GAP as usize)));
        }
        row_spans.push(chip(idx));
    }

    // Body rows, top-down. The question is always row 0; the optional working
    // sub-line shifts everything below it down by one, so we track the button
    // row's index as we push lines (used for the click-rect y below).
    let mut lines: Vec<Line> = Vec::with_capacity(6);
    lines.push(Line::from(Span::styled(
        "Do you want to quit?",
        Style::default().fg(palette.fg).add_modifier(Modifier::BOLD),
    )));
    if s.working > 0 {
        let plural = if s.working == 1 { "session" } else { "sessions" };
        lines.push(Line::from(Span::styled(
            format!(
                "{} {plural} still working — in-flight work will be lost.",
                s.working
            ),
            Style::default().fg(palette.dim),
        )));
    }
    lines.push(Line::from("")); // blank before the button row
    let button_row = lines.len() as u16; // index of the next pushed line
    lines.push(Line::from(row_spans));
    lines.push(Line::from("")); // blank after the button row
    lines.push(Line::from(Span::styled(
        DESCS[sel],
        Style::default().fg(palette.dim),
    )));
    frame.render_widget(Paragraph::new(lines), inner);

    // On-screen width of a button chip: label plus the `[` and `]` bracket chars,
    // matching the `[label]` chip rendered above.
    let chip_w = |idx: usize| LABELS[idx].len() as u16 + 2;

    // Record each button's on-screen Rect as a chip-width horizontal segment on
    // the button row, in index order (0 = quit & kill, 1 = minimize, 2 = cancel)
    // so click hit-testing matches `button_rects`' documented order. Walk the row
    // accumulating chip widths + gaps from `inner.x`, mirroring the render above.
    // Guard tiny terminals: if the row is off-screen (not enough height) or the
    // full row width can't fit, leave the rects empty (Rect::ZERO) so nothing is
    // clickable rather than pointing clicks at the wrong place.
    let total_w: u16 = chip_w(0) + chip_w(1) + chip_w(2) + GAP * 2;
    let rects = if inner.width >= total_w && inner.height > button_row {
        let mut rects = [Rect::ZERO; 3];
        let mut x = inner.x;
        for (idx, rect) in rects.iter_mut().enumerate() {
            let w = chip_w(idx);
            *rect = Rect {
                x,
                y: inner.y + button_row,
                width: w,
                height: 1,
            };
            x = x.saturating_add(w).saturating_add(GAP);
        }
        rects
    } else {
        [Rect::ZERO; 3]
    };
    s.button_rects.set(rects);

    // --- Keybinding hint ---
    let hint = "←/→ move · Enter select · k/d/Esc shortcut · click";
    let instructions = Paragraph::new(hint).style(Style::default().fg(palette.dim));
    frame.render_widget(instructions, chunks[2]);
}
