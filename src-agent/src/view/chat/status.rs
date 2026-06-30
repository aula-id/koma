//! Status bar: left-side animated comet label + right-side token/cost readout.

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use crate::app::state::AppStateRest;
use crate::view::theme::Palette;
use super::helpers::{comet_spans, fmt_count};

/// Render the status bar into `chunk`.
///
/// While the app is WORKING (`work_since` is set), the label animates: a
/// travelling accent "comet" sweeps across the phase word with a dim ` · {secs}s`
/// elapsed counter. Idle (`ready`) and the `approve …? [y/n]` prompt render
/// statically — a single plain dim span, no comet, no timer.
///
/// The cumulative token/cost readout is right-aligned when non-zero.
pub(super) fn render_status(frame: &mut Frame, chunk: Rect, rest: &AppStateRest, palette: &Palette) {
    let status_area = chunk.inner(Margin { horizontal: 2, vertical: 0 });
    // Status line is per-session (C6): render the FOREGROUND session's status.
    let status = &rest.fg().status;
    let status_line: Line<'static> = match rest.work_since {
        Some(since) => {
            let elapsed_ms = since.elapsed().as_millis();
            let mut spans = comet_spans(status, elapsed_ms, palette);
            // Dim elapsed counter, e.g. `thinking · 3s`. Whole seconds so it ticks
            // calmly (the comet supplies the fast motion).
            spans.push(Span::styled(
                format!(" · {}s", elapsed_ms / 1000),
                Style::default().fg(palette.dim),
            ));
            Line::from(spans)
        }
        None => Line::from(Span::styled(
            status.clone(),
            Style::default().fg(palette.dim),
        )),
    };
    // Read the FOREGROUND session's own counters — each tab shows only its own
    // ↑/↓/$, never the sum across sessions.
    let fg = rest.fg();
    let readout = if fg.tokens_in > 0 || fg.tokens_out > 0 || fg.cost > 0.0 {
        // Show the cached-prompt-token count right after the input arrow when the
        // last response hit the prompt cache (`cached:N`), so the saving is
        // visible; omitted entirely on a cold prefix to keep the readout quiet.
        let cached = if fg.tokens_cached > 0 {
            format!(" cached:{}", fmt_count(fg.tokens_cached))
        } else {
            String::new()
        };
        Some(format!(
            "↑{}{} ↓{}  ${:.4}",
            fmt_count(fg.tokens_in),
            cached,
            fmt_count(fg.tokens_out),
            fg.cost
        ))
    } else {
        None
    };
    match &readout {
        Some(r) => {
            // `↑ ↓ $` and digits are each one display column, so a char count is
            // the exact width; +1 keeps a gap from the status text, +4 accounts for
            // the ` [!]` aggregate marker appended after the cost figure.
            let w = u16::try_from(r.chars().count() + 1 + 4).unwrap_or(u16::MAX);
            let cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Min(0), Constraint::Length(w)])
                .split(status_area);
            // Per-span styles (the comet's colours, or the static dim) own the look;
            // no paragraph-level base style so it doesn't flatten the comet head.
            frame.render_widget(Paragraph::new(status_line), cols[0]);
            // Two spans: accent for the counters, dim for the [!] aggregate marker.
            // [!] signals that the cost figure is an aggregate (includes sub-agent
            // spend); the detail will be visible in a future /usage screen.
            let readout_line = Line::from(vec![
                Span::styled(r.as_str(), Style::default().fg(palette.accent)),
                Span::styled(" [!]", Style::default().fg(palette.dim)),
            ]);
            frame.render_widget(
                Paragraph::new(readout_line).alignment(Alignment::Right),
                cols[1],
            );
        }
        None => {
            frame.render_widget(Paragraph::new(status_line), status_area);
        }
    }
}
