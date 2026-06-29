//! Header row and model-name label rendering.

use ratatui::{
    layout::{Alignment, Margin, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph},
    Frame,
};
use crate::app::state::AppStateRest;
use crate::config::DEFAULT_MODEL;
use crate::view::theme::Palette;
use super::helpers::truncate_chars;

/// Render the header line ("koma" + mode indicator) into `chunk`.
///
/// Mode colours are fixed regardless of theme: Normal = green, Auto = yellow.
pub(super) fn render_header(frame: &mut Frame, chunk: Rect, rest: &AppStateRest, palette: &Palette) {
    let (mode_icon, mode_label, mode_color) = match rest.agent_mode {
        crate::app::state::AgentMode::Normal => ("●", "normal", Color::Rgb(80, 220, 80)),
        crate::app::state::AgentMode::Auto   => ("»", "auto",   Color::Rgb(255, 210, 60)),
    };
    // Build the right-side text ("● normal" or "» auto") so we can
    // measure it and pad the gap between brand and mode.
    let mode_str = format!("{mode_icon} {mode_label}");
    // header_inner width = frame width minus 2 (border) minus 4 (horizontal padding 2+2)
    let header_inner_w = frame.area().width.saturating_sub(2 + 4) as usize;
    let brand = "koma";

    // Version badge — HARDCODED colours (NOT palette.accent, which is themeable):
    // green when up-to-date, pink current + green "[latest]" when an update is known.
    const GREEN: Color = Color::Rgb(57, 255, 20);
    const PINK: Color = Color::Rgb(255, 105, 180);
    let cur = crate::model::store::current_version();
    let update = rest
        .latest_version
        .as_ref()
        .filter(|v| crate::app::version::is_newer(&v.version, cur));

    // Build the badge spans (brand stays dim; only the version part is coloured) and
    // measure their TOTAL char width so the gap keeps the mode icon/label right-aligned.
    let mut badge_spans = vec![
        Span::styled(brand, Style::default().fg(palette.dim)),
        Span::raw(" "),
    ];
    let mut badge_w = brand.chars().count() + 1; // brand + leading space before version
    if let Some(v) = update {
        let latest = format!("[{}]", v.version);
        badge_w += cur.chars().count() + 1 + latest.chars().count(); // cur + space + "[latest]"
        badge_spans.push(Span::styled(cur, Style::default().fg(PINK)));
        badge_spans.push(Span::raw(" "));
        badge_spans.push(Span::styled(latest, Style::default().fg(GREEN)));
    } else {
        badge_w += cur.chars().count();
        badge_spans.push(Span::styled(cur, Style::default().fg(GREEN)));
    }

    // Gap = available width minus the FULL badge width minus mode string chars; floor at 1.
    let gap = header_inner_w
        .saturating_sub(badge_w + mode_str.chars().count())
        .max(1);
    let mut header_spans = badge_spans;
    header_spans.push(Span::raw(" ".repeat(gap)));
    header_spans.push(Span::styled(mode_icon, Style::default().fg(mode_color)));
    header_spans.push(Span::raw(" "));
    header_spans.push(Span::styled(mode_label, Style::default().fg(mode_color)));
    let header_line = Line::from(header_spans);
    let header_block = Block::new()
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(palette.dim))
        .padding(Padding::horizontal(2));
    let header_inner = header_block.inner(chunk);
    frame.render_widget(header_block, chunk);
    frame.render_widget(Paragraph::new(header_line), header_inner);
}

/// Render the model-name row: right-aligned, dim, sits directly above the input
/// top border so it reads as a label for it. Truncated to the inner width
/// (2-col padding each side) when the model string is absurdly long.
pub(super) fn render_model_row(
    frame: &mut Frame,
    chunk: Rect,
    rest: &AppStateRest,
    resolved_model: &str,
    palette: &Palette,
) {
    let row_inner_w = chunk.width.saturating_sub(4) as usize; // 2+2 padding
    let display_model = if resolved_model.is_empty() {
        match rest.fg().session.as_ref() {
            Some(s) => s.settings.model.as_str(),
            None    => DEFAULT_MODEL,
        }
    } else {
        resolved_model
    };
    let model_label = truncate_chars(display_model, row_inner_w);
    let model_row = Paragraph::new(Span::styled(model_label, Style::default().fg(palette.dim)))
        .alignment(Alignment::Right);
    let model_area = chunk.inner(Margin { horizontal: 2, vertical: 0 });
    frame.render_widget(model_row, model_area);
}
