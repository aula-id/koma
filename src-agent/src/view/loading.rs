//! View – startup loading splash (Loading mode).
//!
//! A btop-style, borderless, full-frame centered splash shown while a
//! returning-into-Chat session warms asynchronously (the project awareness
//! summary runs as a background task; see the non-blocking `runtime::warm_session`
//! refactor — the model catalogue is no longer fetched here, it loads on demand
//! per endpoint in the omnisearch). Purely presentational: the step statuses live
//! in [`app::mode::LoadingState`], the spinner `frame` is advanced by the event
//! loop each tick, and `Esc` (skip) is handled in
//! [`controller::input::handle_loading`].
//!
//! Layout (top → bottom), centered:
//!
//! ```text
//!                         koma
//!
//!                  ⠙  indexing workspace
//!                  ⠹  reading project docs
//!
//!              warming up · 1.4s   ·   esc to skip
//! ```
//!
//! Markers: a braille spinner while `Running`, `●` (accent) when `Done` with a
//! dim detail, and a dim `·` for Pending / Skipped / Failed (with a trailing dim
//! word for the latter two). No emoji — braille + `●` / `·` are plain glyphs.

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::app::mode::{LoadingState, WarmStatus};
use crate::view::theme::Palette;

/// Braille spinner cycle (10 frames). Indexed by `frame % 10`, advanced each
/// draw tick by the event loop so the glyph rotates.
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// The two step rows, in display order, with their fixed labels.
const STEP_LABELS: [&str; 2] = ["indexing workspace", "reading project docs"];

/// Build one centered step row: `<marker>  <label>[  <detail>]`.
///
/// - `Running` → the current braille spinner frame, in `palette.accent`.
/// - `Done`    → `●` in `palette.accent`, plus the carried detail in `palette.dim`.
/// - `Pending` → a dim `·` (nothing else).
/// - `Skipped` → a dim `·` + dim ` skipped`.
/// - `Failed`  → a dim `·` + dim ` failed`.
fn step_line<'a>(
    status: &'a WarmStatus,
    label: &'a str,
    frame: u64,
    palette: &Palette,
) -> Line<'a> {
    let accent = Style::default().fg(palette.accent).bg(palette.bg);
    let dim = Style::default().fg(palette.dim).bg(palette.bg);

    // Marker + an optional trailing detail span (Done detail / skipped|failed word).
    let (marker, marker_style, detail): (String, Style, Option<Span>) = match status {
        WarmStatus::Running => (SPINNER[(frame % 10) as usize].to_string(), accent, None),
        WarmStatus::Done(d) => {
            let detail = if d.is_empty() {
                None
            } else {
                Some(Span::styled(format!("  {d}"), dim))
            };
            ("●".to_string(), accent, detail)
        }
        WarmStatus::Pending => ("·".to_string(), dim, None),
        WarmStatus::Skipped => ("·".to_string(), dim, Some(Span::styled("  skipped", dim))),
        WarmStatus::Failed => ("·".to_string(), dim, Some(Span::styled("  failed", dim))),
    };

    // Label is dim while not Done/Running-distinct; keep it simple and readable:
    // the marker carries the colour, the label stays in the primary fg.
    let mut spans = vec![
        Span::styled(marker, marker_style),
        Span::styled("  ", dim),
        Span::styled(label, Style::default().fg(palette.fg).bg(palette.bg)),
    ];
    if let Some(d) = detail {
        spans.push(d);
    }
    Line::from(spans)
}

/// Draw a centered "reopening" splash with an animated braille spinner, driven by
/// `spinner_frame` (advances one braille glyph per call, 10-frame cycle). Used by the client
/// while it silently restarts a stale session-daemon (`attach_session`), so the ~1s
/// blocking restart shows motion instead of a frozen frame. Standalone (no daemon
/// snapshot needed) — mirrors this module's `draw` layout but with a single line.
pub fn draw_reopening(frame: &mut Frame, spinner_frame: u64, palette: &Palette) {
    let area = frame.area();
    crate::view::clear_and_fill(frame, area, palette.bg);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(40), // spacer → line lands a bit above center
            Constraint::Length(1),      // "koma" title
            Constraint::Length(1),      // gap
            Constraint::Length(1),      // spinner + label
            Constraint::Min(0),         // rest
        ])
        .split(area);

    let title = Paragraph::new(Line::from(Span::styled(
        "koma",
        Style::default().fg(palette.accent).bg(palette.bg),
    )))
    .style(Style::default().bg(palette.bg))
    .alignment(Alignment::Center);
    frame.render_widget(title, chunks[1]);

    let glyph = SPINNER[(spinner_frame % 10) as usize];
    let line = Line::from(vec![
        Span::styled(glyph, Style::default().fg(palette.accent).bg(palette.bg)),
        Span::styled(
            "  reopening",
            Style::default().fg(palette.fg).bg(palette.bg),
        ),
    ]);
    frame.render_widget(
        Paragraph::new(line)
            .style(Style::default().bg(palette.bg))
            .alignment(Alignment::Center),
        chunks[3],
    );
}

/// Render the loading splash for `state` using the given colour `palette`.
pub fn draw(frame: &mut Frame, state: &LoadingState, palette: &Palette) {
    let area = frame.area();
    crate::view::clear_and_fill(frame, area, palette.bg);

    // Vertical layout: an upper-third spacer pushes the title down to ~1/3, the
    // step block sits in the middle, and the footer pins to the bottom. The
    // fixed-height title + steps + footer are separated by flexible spacers so
    // the whole thing stays vertically centered-ish (btop splash feel).
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(30), // top spacer → title lands in the upper third
            Constraint::Length(1),      // title
            Constraint::Length(1),      // gap
            Constraint::Length(2),      // the two step rows
            Constraint::Min(1),         // flexible spacer
            Constraint::Length(1),      // footer
            Constraint::Length(1),      // bottom margin
        ])
        .split(area);

    // --- Title: "koma" in accent, centered ---
    let title = Paragraph::new(Line::from(Span::styled(
        "koma",
        Style::default().fg(palette.accent).bg(palette.bg),
    )))
    .style(Style::default().bg(palette.bg))
    .alignment(Alignment::Center);
    frame.render_widget(title, chunks[1]);

    // --- Step rows: a centered block of two lines ---
    let steps: Vec<Line> = STEP_LABELS
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let status = match i {
                0 => &state.workspace,
                _ => &state.awareness,
            };
            step_line(status, label, state.frame, palette)
        })
        .collect();
    frame.render_widget(
        Paragraph::new(steps)
            .style(Style::default().bg(palette.bg))
            .alignment(Alignment::Center),
        chunks[3],
    );

    // --- Footer: dim "warming up · {elapsed:.1}s   ·   esc to skip" ---
    let elapsed = state.started.elapsed().as_secs_f64();
    let footer = Paragraph::new(Line::from(Span::styled(
        format!("warming up · {elapsed:.1}s   ·   esc to skip"),
        Style::default().fg(palette.dim).bg(palette.bg),
    )))
    .style(Style::default().bg(palette.bg))
    .alignment(Alignment::Center);
    frame.render_widget(footer, chunks[5]);
}
