//! Header row and model-name label rendering.

use super::helpers::truncate_chars;
use crate::app::state::AppStateRest;
use crate::view::theme::Palette;
use ratatui::{
    layout::{Alignment, Margin, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph},
    Frame,
};

/// Render the header line ("koma" + mode indicator) into `chunk`.
///
/// Mode colours follow the palette's semantic roles: Normal = success (green),
/// Auto = warn (yellow), Plan = info (calm cyan — read-only, nothing to fear),
/// Yolo = error (LOUD red — harness bypassed, make it unmistakable).
pub(super) fn render_header(
    frame: &mut Frame,
    chunk: Rect,
    rest: &AppStateRest,
    palette: &Palette,
) {
    let (mode_icon, mode_label, mode_color) = match rest.agent_mode {
        crate::app::state::AgentMode::Normal => ("●", "normal".to_string(), palette.success),
        crate::app::state::AgentMode::Auto => ("»", "auto".to_string(), palette.warn),
        crate::app::state::AgentMode::Plan => ("●", "planning".to_string(), palette.info),
        crate::app::state::AgentMode::Yolo => ("!", "yooloo".to_string(), palette.error),
        crate::app::state::AgentMode::Sdlc => {
            if let Some(ref phase) = rest.sdlc_phase {
                ("◆", format!("sdlc:{phase}"), palette.info)
            } else {
                ("◆", "sdlc".to_string(), palette.info)
            }
        }
    };
    // Build the right-side text ("● normal" or "» auto") so we can
    // measure it and pad the gap between brand and mode.
    let mode_str = format!("{mode_icon} {mode_label}");
    // header_inner width = frame width minus 2 (border) minus 4 (horizontal padding 2+2)
    let header_inner_w = frame.area().width.saturating_sub(2 + 4) as usize;
    let brand = "koma";

    // Version badge — uses semantic palette roles:
    // success when up-to-date, error current + success "[latest]" when an update is known.
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
        badge_spans.push(Span::styled(cur, Style::default().fg(palette.error)));
        badge_spans.push(Span::raw(" "));
        badge_spans.push(Span::styled(latest, Style::default().fg(palette.success)));
    } else {
        badge_w += cur.chars().count();
        badge_spans.push(Span::styled(cur, Style::default().fg(palette.success)));
    }

    // Gap = available width minus the FULL badge width minus mode string chars; floor at 1.
    let gap = header_inner_w
        .saturating_sub(badge_w + mode_str.chars().count())
        .max(1);
    let mut header_spans = badge_spans;
    header_spans.push(Span::raw(" ".repeat(gap)));
    if matches!(
        rest.agent_mode,
        crate::app::state::AgentMode::Plan | crate::app::state::AgentMode::Sdlc
    ) {
        // "planning" gets a bold shimmer sweep instead of a flat colour — see
        // `plan_shimmer_spans` below. Icon stays a plain bold dot of the same cyan.
        header_spans.push(Span::styled(
            mode_icon,
            Style::default().fg(mode_color).add_modifier(Modifier::BOLD),
        ));
        header_spans.push(Span::raw(" "));
        let elapsed_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        header_spans.extend(plan_shimmer_spans(&mode_label, elapsed_ms, palette));
    } else {
        header_spans.push(Span::styled(mode_icon, Style::default().fg(mode_color)));
        header_spans.push(Span::raw(" "));
        header_spans.push(Span::styled(mode_label, Style::default().fg(mode_color)));
    }
    let header_line = Line::from(header_spans);
    let header_block = Block::new()
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(palette.dim))
        .padding(Padding::horizontal(2));
    let header_inner = header_block.inner(chunk);
    frame.render_widget(header_block, chunk);
    frame.render_widget(Paragraph::new(header_line), header_inner);
}

/// Build the shimmering "planning" label spans for the Plan-mode header entry.
///
/// The whole word is bold, base-coloured in the calm plan cyan. A small bright
/// highlight (one peak char + a dimmer shoulder on each side where the text
/// allows it) sweeps left-to-right across the letters, then spends a short
/// `GAP`-wide pause off the right edge — during which the word sits flat at
/// the base colour — before wrapping back to the start. That pause is what
/// turns the sweep into a rhythmic pulse instead of a nonstop strobe.
///
/// Time-driven only (no stored counter, mirrors `comet_spans` in
/// `super::helpers`): the phase is derived straight from wall-clock
/// milliseconds, so it advances on every redraw with nothing to reset. Per-char
/// spans keep the colour mapping trivial and multibyte-safe (`chars()`).
fn plan_shimmer_spans(text: &str, elapsed_ms: u128, palette: &Palette) -> Vec<Span<'static>> {
    let base = palette.info;
    let shoulder = crate::view::theme::lighten(palette.info, 0.33);
    let peak = crate::view::theme::lighten(palette.info, 0.66);
    const FRAME_MS: u128 = 90; // sweep advance cadence
    const GAP: usize = 3; // pause length after the highlight exits, before it wraps

    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    if n == 0 {
        return Vec::new();
    }
    let period = n + GAP;
    let pos = ((elapsed_ms / FRAME_MS) as usize) % period.max(1);

    let mut spans: Vec<Span<'static>> = Vec::with_capacity(n);
    for (i, ch) in chars.iter().enumerate() {
        // `pos >= n` means the highlight is in its off-screen gap pause: render
        // flat base colour. Otherwise the char at `pos` is the bright peak, and
        // its immediate neighbours (when in bounds) are the dimmer shoulders.
        let color = if pos >= n {
            base
        } else if i == pos {
            peak
        } else if i + 1 == pos || i == pos + 1 {
            shoulder
        } else {
            base
        };
        spans.push(Span::styled(
            ch.to_string(),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ));
    }
    spans
}

/// Render the model-name row: right-aligned, dim, sits directly above the input
/// top border so it reads as a label for it. Truncated to the inner width
/// (2-col padding each side) when the model string is absurdly long.
pub(super) fn render_model_row(
    frame: &mut Frame,
    chunk: Rect,
    _rest: &AppStateRest,
    resolved_model: &str,
    palette: &Palette,
) {
    let row_inner_w = chunk.width.saturating_sub(4) as usize; // 2+2 padding
    let display_model = if resolved_model.is_empty() {
        // Prefer a blank label over the dead serde default (`openai/gpt-4o-mini`)
        // when the live resolver returned nothing. Surfacing DEFAULT_MODEL here
        // made it look like Main had been swapped to gpt after an OAuth/provider
        // drift, even though the user never configured that model.
        ""
    } else {
        resolved_model
    };
    let model_label = truncate_chars(display_model, row_inner_w);
    let model_row = Paragraph::new(Span::styled(model_label, Style::default().fg(palette.dim)))
        .alignment(Alignment::Right);
    let model_area = chunk.inner(Margin {
        horizontal: 2,
        vertical: 0,
    });
    frame.render_widget(model_row, model_area);
}
