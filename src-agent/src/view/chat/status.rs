//! Status bar: left-side animated comet label + right-side token/cost readout.

use super::helpers::{comet_spans, fmt_count};
use crate::app::state::{AppStateRest, SessionRuntime};
use crate::view::theme::Palette;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

/// Render the status bar into `chunk`.
///
/// While the app is WORKING (`work_since` is set), the label animates: a
/// travelling accent "comet" sweeps across the phase word with a dim ` · {secs}s`
/// elapsed counter. Idle (`ready`) and the `approve …? [y/n]` prompt render
/// statically — a single plain dim span, no comet, no timer.
///
/// Latest context usage and cumulative output/cost are right-aligned.
pub(super) fn render_status(
    frame: &mut Frame,
    chunk: Rect,
    rest: &AppStateRest,
    palette: &Palette,
) {
    let status_area = chunk.inner(Margin {
        horizontal: 2,
        vertical: 0,
    });
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
    let readout = usage_readout(fg, palette);
    match &readout {
        Some(r) => {
            // Include the dim cache bracket and aggregate marker in the width.
            let w = u16::try_from(r.width() + 1).unwrap_or(u16::MAX);
            let cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Min(0), Constraint::Length(w)])
                .split(status_area);
            // Per-span styles (the comet's colours, or the static dim) own the look;
            // no paragraph-level base style so it doesn't flatten the comet head.
            frame.render_widget(Paragraph::new(status_line), cols[0]);
            frame.render_widget(
                Paragraph::new(r.clone()).alignment(Alignment::Right),
                cols[1],
            );
        }
        None => {
            frame.render_widget(Paragraph::new(status_line), status_area);
        }
    }
}

/// The estimate and limit come from the same shaped request. Cached tokens are
/// a subset of the input count; output and spend remain cumulative.
fn usage_readout(fg: &SessionRuntime, palette: &Palette) -> Option<Line<'static>> {
    let input = fg.context_usage.map_or(fg.tokens_in, |u| u.prompt_tokens);
    if fg.context_usage.is_none() && input == 0 && fg.tokens_out == 0 && fg.cost == 0.0 {
        return None;
    }
    let context = match fg.context_usage.filter(|u| u.effective_window > 0) {
        Some(usage) => format!(
            "{}{:.0}%",
            if usage.estimated { "~" } else { "" },
            usage.prompt_tokens as f64 / usage.effective_window as f64 * 100.0
        ),
        // A restored session/older daemon may have counts without the request's
        // context limit. Avoid guessing a denominator for those historical counts.
        None => "—%".into(),
    };
    let accent = Style::default().fg(palette.accent);
    let dim = Style::default().fg(palette.dim);
    let mut spans = vec![Span::styled(
        format!("{context} ↑{}", fmt_count(input)),
        accent,
    )];
    if fg.tokens_cached > 0 {
        spans.push(Span::styled(
            format!("[{}]", fmt_count(fg.tokens_cached)),
            dim,
        ));
    }
    spans.push(Span::styled(
        format!(" ↓{} ${:.4}", fmt_count(fg.tokens_out), fg.cost),
        accent,
    ));
    // [!] retains its existing meaning: aggregate spend includes sub-agents.
    spans.push(Span::styled(" [!]", dim));
    Some(Line::from(spans))
}
