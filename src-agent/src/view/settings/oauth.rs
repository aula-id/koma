//! View — OAuth connect-flow overlay sub-renderers (picker, wait, paste, failed).
//! The idle connection list for the `/settings` OAuth page is in
//! `pages/oauth.rs`; this module only holds the transient flow overlays that
//! are reused by `view::onboard_provider`.

use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
    Frame,
};

use crate::view::theme::Palette;

/// Provider picker overlay: an 8-option list, cursor-highlighted.
///
/// `pub(crate)` so the guided provider onboarding wizard reuses it for its Login step.
pub(crate) fn draw_picker(frame: &mut Frame, cursor: usize, palette: &Palette, area: Rect) {
    const OPTIONS: [&str; 8] = [
        "Codex",
        "Kilo Code",
        "koma.run",
        "xAI",
        "Claude",
        "Command Code",
        "Codex (paste token)",
        "Command Code (paste key)",
    ];
    let full_w = area.width as usize;
    crate::view::clear_and_fill(frame, area, palette.bg);
    let lines: Vec<Line> = OPTIONS
        .iter()
        .enumerate()
        .map(|(i, label)| {
            if i == cursor {
                let text = format!("› {label}");
                Line::from(Span::styled(
                    format!("{text:<full_w$}"),
                    Style::default().fg(palette.sel_fg).bg(palette.sel_bg),
                ))
            } else {
                Line::from(vec![
                    Span::styled("  ", Style::default().fg(palette.accent)),
                    Span::styled(*label, Style::default().fg(palette.fg)),
                ])
            }
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
}

/// A one-line status/spinner message plus an optional URL/code line beneath it —
/// shared shape for `Starting`/`CodexWait`/`KiloWait`. `copied` shows a dim
/// confirmation line under the URL after a successful `c` (copy-url) press.
///
/// `pub(crate)` so the guided provider onboarding wizard reuses it for its Login step.
pub(crate) fn draw_message(
    frame: &mut Frame,
    palette: &Palette,
    area: Rect,
    headline: &str,
    url: Option<&str>,
    copied: bool,
) {
    let mut lines = vec![Line::from(Span::styled(
        headline.to_string(),
        Style::default().fg(palette.accent),
    ))];
    if let Some(u) = url {
        if !u.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                u.to_string(),
                Style::default().fg(palette.dim),
            )));
            if copied {
                lines.push(Line::from(Span::styled(
                    "url copied to clipboard",
                    Style::default().fg(palette.dim),
                )));
            }
        }
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

/// The manual paste-token screen: a single input line with a trailing cursor.
///
/// `pub(crate)` so the guided provider onboarding wizard reuses it for its Login step.
pub(crate) fn draw_paste(frame: &mut Frame, input: &str, palette: &Palette, area: Rect) {
    let line = Line::from(vec![
        Span::styled("token: ", Style::default().fg(palette.dim)),
        Span::styled(input.to_string(), Style::default().fg(palette.fg)),
        Span::styled("█", Style::default().fg(palette.accent)),
    ]);
    frame.render_widget(Paragraph::new(vec![line]).wrap(Wrap { trim: false }), area);
}

/// Failure screen: the error line in red plus a dismiss hint.
///
/// `pub(crate)` so the guided provider onboarding wizard reuses it for its Login step.
pub(crate) fn draw_failed(frame: &mut Frame, msg: &str, palette: &Palette, area: Rect) {
    let lines = vec![
        Line::from(Span::styled(msg.to_string(), Style::default().fg(palette.error))),
        Line::from(""),
        Line::from(Span::styled(
            "enter/esc dismiss",
            Style::default().fg(palette.dim),
        )),
    ];
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}
