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
//! 3. The three keyed choices, each on its own line, the key accented.
//! 4. A one-line honesty note about Phase-1 detach (no daemon yet).
//!
//! No selection cursor: the choices are bound to distinct keys (k / d / Esc),
//! handled in [`crate::controller::input::handle_quit_confirm`].

use ratatui::{
    layout::{Constraint, Direction, Layout, Margin},
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

    // Each choice: an accented key glyph + a dim description. The accent colour
    // marks the actionable key; the rest stays neutral.
    let key = |c: &'static str| Span::styled(
        c,
        Style::default()
            .fg(palette.accent)
            .add_modifier(Modifier::BOLD),
    );
    let desc = |t: &'static str| Span::styled(t, Style::default().fg(palette.fg));

    let lines: Vec<Line> = vec![
        Line::from(Span::styled(
            warn,
            Style::default().fg(palette.fg).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            key("[k] "),
            desc("kill all & quit  — abort every session's in-flight work, then exit"),
        ]),
        Line::from(vec![
            key("[d] "),
            desc("detach & quit    — leave conversations on disk (resumable), then exit"),
        ]),
        Line::from(vec![
            key("[esc] "),
            desc("cancel           — back to chat, keep everything running"),
        ]),
        Line::from(""),
        // Honesty note: Phase 1 has no daemon, so detach can't keep cooking
        // headless — the work dies with the process. Be explicit about it.
        Line::from(Span::styled(
            "note: detach leaves sessions resumable but does NOT keep them \
             running headless — that arrives with the daemon (Phase 2).",
            Style::default().fg(palette.dim),
        )),
    ];
    frame.render_widget(Paragraph::new(lines), inner);

    // --- Keybinding hint ---
    let hint = "k kill all · d detach · Esc cancel";
    let instructions = Paragraph::new(hint).style(Style::default().fg(palette.dim));
    frame.render_widget(instructions, chunks[2]);
}
