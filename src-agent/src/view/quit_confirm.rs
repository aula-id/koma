//! View – quit-confirm overlay (`Mode::QuitConfirm`).
//!
//! Shown always when the user asks to quit — regardless of whether any session
//! has work in flight. Flat, boxless layout (top+bottom rules only, matching the
//! session-hub + session-picker views + the repo border convention) — top to bottom:
//!
//! 1. Top+bottom rule title bar — ` quit ` on the TOP rule.
//! 2. An adaptive header line:
//!    - working > 0: "N session(s) still cooking — keep them running or kill all?"
//!    - working == 0: "Keep your N session(s) for next time, or close them?"
//! 3. The three keyed choices, each on its own line, the key rendered as a small
//!    filled "chip" so the row reads as a clickable button.
//! 4. A one-line note clarifying that detach persists conversations but does not
//!    keep work running after exit.
//!
//! No selection cursor: the choices are bound to distinct keys (k / d / Esc),
//! handled in [`crate::controller::input::handle_quit_confirm`]. Each option row
//! is ALSO clickable — its on-screen [`ratatui::layout::Rect`] is recorded into
//! [`QuitConfirmState::button_rects`] here so the event loop can hit-test a
//! left-click and dispatch the matching action.

use ratatui::{
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph},
    Frame,
};
use crate::app::mode::QuitConfirmState;
use crate::view::theme::Palette;

/// Render the quit-confirm overlay for `s` using the given colour `palette`.
pub fn draw(frame: &mut Frame, s: &QuitConfirmState, palette: &Palette) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // title: top+bottom rules
            Constraint::Min(1),    // warning + choices + note
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

    // --- Body: header + choices + honesty note ---
    let inner = chunks[1].inner(Margin { horizontal: 1, vertical: 0 });

    // Adaptive header: wording differs based on whether any work is in flight.
    let warn = if s.working > 0 {
        let plural = if s.working == 1 { "session" } else { "sessions" };
        format!(
            "{} {plural} still cooking — keep them running or kill all?",
            s.working
        )
    } else {
        let plural = if s.total == 1 { "session" } else { "sessions" };
        format!("Keep your {} {plural} for next time, or close them?", s.total)
    };

    // Each choice renders as a small filled "chip" (the key, reversed onto the
    // accent colour) followed by a neutral description — so the whole row reads
    // as a clickable button. `sel_fg` is the palette's on-accent foreground
    // (true-black / true-white), legible even under BOLD; it matches the footer +
    // selection inverse-text treatment used elsewhere.
    let chip = |c: &'static str| Span::styled(
        c,
        Style::default()
            .bg(palette.accent)
            .fg(palette.sel_fg)
            .add_modifier(Modifier::BOLD),
    );
    let desc = |t: &'static str| Span::styled(t, Style::default().fg(palette.fg));
    // A single space gap between the chip and its label.
    let gap = || Span::raw(" ");

    let lines: Vec<Line> = vec![
        Line::from(Span::styled(
            warn,
            Style::default().fg(palette.fg).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            chip(" k "),
            gap(),
            desc("kill all & quit  — abort every session's in-flight work, then exit"),
        ]),
        Line::from(vec![
            chip(" d "),
            gap(),
            desc("detach & quit    — leave conversations on disk (resumable), then exit"),
        ]),
        Line::from(vec![
            chip(" esc "),
            gap(),
            desc("cancel           — back to chat, keep everything running"),
        ]),
        Line::from(""),
        // Note: detach persists each conversation so it can be resumed later, but
        // there is no headless background mode — the work stops when koma exits.
        Line::from(Span::styled(
            "note: detach saves each conversation to disk to resume later — \
             in-flight work still stops when koma exits.",
            Style::default().fg(palette.dim),
        )),
    ];
    frame.render_widget(Paragraph::new(lines), inner);

    // Record the three option rows as full-width click targets. The Paragraph
    // lines are laid out top-down (header=0, blank=1, k=2, d=3, esc=4), so each
    // button is a 1-row band at `inner.y + ROW`. Full-row width is a forgiving
    // hit target. Guard tiny terminals: if `inner` can't fit all five rows we
    // leave the rects empty (Rect::ZERO) so nothing is clickable rather than
    // pointing clicks at off-screen / overlapping rows. Order: kill, detach,
    // cancel — matching `button_rects`' documented index order.
    let rects = if inner.width > 0 && inner.height >= 5 {
        let row = |r: u16| Rect {
            x: inner.x,
            y: inner.y + r,
            width: inner.width,
            height: 1,
        };
        [row(2), row(3), row(4)]
    } else {
        [Rect::ZERO; 3]
    };
    s.button_rects.set(rects);

    // --- Keybinding hint ---
    let hint = "k kill all · d detach · Esc cancel · click to choose";
    let instructions = Paragraph::new(hint).style(Style::default().fg(palette.dim));
    frame.render_widget(instructions, chunks[2]);
}
