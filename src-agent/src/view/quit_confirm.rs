//! View – quit-confirm overlay (`Mode::QuitConfirm`).
//!
//! Shown always when the user asks to quit — regardless of whether any session
//! has work in flight. Flat, boxless layout (top+bottom rules only, matching the
//! session-hub + session-picker views + the repo border convention) — top to bottom:
//!
//! 1. Top+bottom rule title bar — ` quit ` on the TOP rule.
//! 2. A clean question line ("Do you want to quit?"); when work is in flight a
//!    dim sub-line warns that in-flight work will be lost.
//! 3. A navigable horizontal BUTTON ROW: `[close window (quit)]  [detach]  [cancel]`.
//!    The focused button (index `s.selected`) is highlighted; the others are
//!    subdued. Each button is laid out as a chip and its on-screen
//!    [`ratatui::layout::Rect`] is recorded into [`QuitConfirmState::button_rects`]
//!    in index order so the event loop can hit-test a left-click.
//! 4. A one-line description of the FOCUSED button.
//!
//! Navigation (Left/Right, Tab/Shift+Tab, Enter) plus the direct k / d / Esc
//! shortcuts are handled in [`crate::controller::input::handle_quit_confirm`].
//!
//! When in the **Exiting** phase (the user activated quit or detach), the entire
//! overlay is replaced with a centered braille spinner and "Exiting…" text. No
//! buttons are rendered and no hit-boxes are recorded.

use crate::app::mode::QuitConfirmState;
use crate::view::theme::Palette;
use ratatui::{
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph},
    Frame,
};

/// The three button labels, left→right, in `button_rects`/`selected` index order
/// (`0` = close window (quit), `1` = detach, `2` = cancel). The chip is the label wrapped
/// in literal brackets with inner padding (`[ close window (quit) ]`) — koma button style — so the chip width is
/// `label.len() + 4`, matching the click-rect math below.
const LABELS: [&str; 3] = ["quit", "detach", "cancel"];

/// One-line description for each button, same index order as [`LABELS`].
/// A window IS its own single-session daemon (daemon-per-session), so `close window (quit)`
/// KILLS that daemon (the session ends, kept on disk → reloadable from the swapper's
/// history), while `detach` leaves the daemon RUNNING and its session COOKING headless
/// (→ resumable live from the swapper's cooking pane).
const DESCS: [&str; 3] = [
    "Quit session and stop current progress",
    "Minimize — as usual agent keep cooking",
    "Back to chat",
];

/// Gap (in columns) rendered between adjacent buttons in the row.
const GAP: u16 = 3;

/// Width (in columns) of the centered content column. Chosen to comfortably
/// fit the button row (≈41 cols) with breathing room, and to look balanced on
/// common terminal widths (80–120 cols).
const CONTENT_WIDTH: u16 = 54;

/// Braille spinner cycle (10 frames), indexed by wall-clock milliseconds / 80.
/// Uses the same 10-glyph cycle as [`crate::view::loading`] and
/// [`crate::view::todo`].
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Time-based braille spinner glyph. Uses wall-clock time so even a single frame
/// shows a meaningful glyph (unlike frame-counter spinners that would be frozen).
fn spinner_glyph() -> &'static str {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    SPINNER[((now_ms / 80) as usize) % SPINNER.len()]
}

/// Compute a centered `Rect` of the given `w` × `h` inside `area`,
/// clamped so it never exceeds the available space. Used exclusively by
/// the quit-confirm body to float the question + buttons dead-center.
fn centered_rect(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect {
        x,
        y,
        width: w,
        height: h,
    }
}

/// Render the quit-confirm overlay for `s` using the given colour `palette`.
///
/// In the `Exiting` phase, renders a centered braille spinner with "Exiting…"
/// instead of the question/button layout.
pub fn draw(frame: &mut Frame, s: &QuitConfirmState, palette: &Palette) {
    if s.is_exiting() {
        return draw_exiting(frame, s, palette);
    }
    draw_choice(frame, s, palette);
}

/// Render the Exiting phase: centered braille spinner + "Exiting…" text.
fn draw_exiting(frame: &mut Frame, s: &QuitConfirmState, palette: &Palette) {
    let area = frame.area();
    crate::view::clear_and_fill(frame, area, palette.bg);

    // Zero out button hit-boxes so stale rects from the Choice phase can't
    // be hit-tested against the new layout.
    s.button_rects.set([Rect::ZERO; 3]);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(35), // top spacer
            Constraint::Length(1),      // title bar
            Constraint::Length(2),      // gap
            Constraint::Length(1),      // spinner + "Exiting…"
            Constraint::Length(1),      // description
            Constraint::Min(0),         // rest
        ])
        .split(area);

    // --- Title bar: " exiting " (mirrors the " quit " title style) ---
    let title_block = Block::new()
        .borders(Borders::TOP | Borders::BOTTOM)
        .border_style(Style::default().fg(palette.dim))
        .title(Span::styled(" exiting ", Style::default().fg(palette.dim)))
        .padding(Padding::horizontal(1));
    frame.render_widget(title_block, chunks[1]);

    // --- Spinner + "Exiting…" ---
    let glyph = spinner_glyph();
    let spinner_line = Line::from(vec![
        Span::styled(format!("{glyph}  "), Style::default().fg(palette.accent)),
        Span::styled(
            "Exiting…",
            Style::default().fg(palette.fg).add_modifier(Modifier::BOLD),
        ),
    ]);
    frame.render_widget(
        Paragraph::new(spinner_line).alignment(ratatui::layout::Alignment::Center),
        chunks[3],
    );

    // --- Description ---
    let subtitle = if s.working > 0 {
        "Stopping session and cleaning up…"
    } else {
        "Cleaning up…"
    };
    let desc = Paragraph::new(Line::from(Span::styled(
        subtitle,
        Style::default().fg(palette.dim),
    )))
    .alignment(ratatui::layout::Alignment::Center);
    frame.render_widget(desc, chunks[4]);
}

/// Render the Choice phase: question + button row + description (original layout).
fn draw_choice(frame: &mut Frame, s: &QuitConfirmState, palette: &Palette) {
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
    // Build the lines first, then center based on actual count.

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
        let label = format!("[ {} ]", LABELS[idx]);
        let style = if idx == sel {
            Style::default()
                .bg(palette.accent)
                .fg(palette.sel_fg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(palette.accent)
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
        let plural = if s.working == 1 {
            "session"
        } else {
            "sessions"
        };
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

    // Center the content column both horizontally and vertically within the
    // body area so the dialog floats dead-center on screen. Height is derived
    // from the actual number of lines built above.
    let body = centered_rect(chunks[1], CONTENT_WIDTH, lines.len() as u16);
    let inner = body.inner(Margin {
        horizontal: 1,
        vertical: 0,
    });
    frame.render_widget(Paragraph::new(lines), inner);

    // On-screen width of a button chip: label plus the `[` and `]` bracket chars
    // and two inner spaces (` label `), matching the `[ label ]` chip rendered above.
    let chip_w = |idx: usize| LABELS[idx].len() as u16 + 4;

    // Record each button's on-screen Rect as a chip-width horizontal segment on
    // the button row, in index order (0 = close window (quit), 1 = detach, 2 = cancel)
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
