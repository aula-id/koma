//! View – first-run connection CHOOSER (`Mode::Onboard`).
//!
//! The very first screen a brand-new install sees: a 3-way pick of HOW to connect
//! (koma free / provider / custom) before any credentials are asked for. Minimalist
//! and top-down — no full box, matching the border-style convention and the
//! [`crate::view::key_input`] wizard it precedes.
//!
//! Layout: a single left-aligned block, horizontally centred in the frame and
//! placed in the upper portion (≈25 % down), rendered row-by-row. Every row shares
//! the same left edge; the SELECTED choice is prefixed `> ` in accent and its label
//! is drawn in accent, while the other rows carry a 2-space indent and a dim-to-fg
//! label. Purely presentational — cursor movement / selection live in
//! [`crate::app::mode::OnboardState`] and [`crate::controller::input::handle_onboard`].

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Padding, Paragraph, Wrap},
    Frame,
};

use crate::app::mode::OnboardState;
use crate::view::theme::Palette;

/// Warning/yellow tone for the "not permanent" callout box. `Palette` has no
/// dedicated warning role (its `accent` is user-configurable and may itself be
/// any colour, including yellow), so — matching the house convention used for
/// other warning callouts (e.g. the tool-approval box in
/// `view::chat::overlays::render_tool_approval` and `YOLO_RED` in
/// `view::settings::mod`) — this is a fixed raw colour rather than a palette lookup.
const WARN: Color = Color::Rgb(255, 180, 60);

/// Total width (chars) of the content block. Clamped to the available area.
const BLOCK_W: u16 = 64;
/// Width (chars) of the label column within a choice row (label + trailing pad),
/// so every description starts at the same column.
const LABEL_W: usize = 14;

/// The three connection choices: `(label, description)`, in cursor order
/// (0 = koma free, 1 = provider, 2 = custom). NO brand names in the copy.
const CHOICES: [(&str, &str); 3] = [
    ("koma free", "start now, no key - free models hosted by koma"),
    ("provider", "sign in to a provider account"),
    ("custom", "your own endpoint + API key"),
];

/// Compute the left-edge x coordinate that centres `block_w` inside `frame_w`.
fn block_x(frame_w: u16, block_w: u16) -> u16 {
    frame_w.saturating_sub(block_w) / 2
}

/// Render the connection chooser for `state` using the given colour `palette`.
pub fn draw(frame: &mut Frame, state: &OnboardState, palette: &Palette) {
    let area = frame.area();

    // Top spacer pushes the block to ≈25 % down; the body is a Min region that
    // holds every row top-down.
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(25), // top spacer → block starts at ~25 %
            Constraint::Min(1),         // block body
        ])
        .split(area);

    let block_w = BLOCK_W.min(area.width);
    let bx = block_x(area.width, block_w);
    let body_y = chunks[1].y;
    let body_h = chunks[1].height;

    // Render one line into the block at row offset `row`; a no-op when the row
    // would fall outside the body region.
    let render_line = |frame: &mut Frame, line: Line, r: u16| {
        if r >= body_h {
            return;
        }
        let rect = Rect {
            x: bx,
            y: body_y + r,
            width: block_w,
            height: 1,
        };
        frame.render_widget(Paragraph::new(line), rect);
    };

    // A dim 2-space-indented line (title uses accent; prompt/footer use dim).
    let indented = |text: &str, style: Style| -> Line<'static> {
        Line::from(vec![
            Span::raw("  "),
            Span::styled(text.to_string(), style),
        ])
    };

    let mut row: u16 = 0;

    // Title.
    render_line(frame, indented("koma", Style::default().fg(palette.accent)), row);
    row += 3; // title + two blank rows

    // Prompt.
    render_line(frame, indented("how do you want to connect?", Style::default().fg(palette.dim)), row);
    row += 2; // prompt + one blank row

    // Choices.
    for (i, (label, desc)) in CHOICES.iter().enumerate() {
        let selected = state.cursor == i;
        let (prefix, label_style) = if selected {
            ("> ", Style::default().fg(palette.accent))
        } else {
            ("  ", Style::default().fg(palette.fg))
        };
        let line = Line::from(vec![
            Span::styled(prefix, Style::default().fg(palette.accent)),
            Span::styled(format!("{label:<LABEL_W$}"), label_style),
            Span::styled((*desc).to_string(), Style::default().fg(palette.dim)),
        ]);
        render_line(frame, line, row);
        row += 1;
    }
    row += 2; // two blank rows before the box

    // Callout: reassure the user this pick isn't permanent — small yellow
    // bordered box, same left edge as everything else in the block (not
    // full-width across the screen).
    const CALLOUT_H: u16 = 4; // top border + 2 content rows + bottom border
    if row + CALLOUT_H <= body_h {
        let callout_rect = Rect {
            x: bx,
            y: body_y + row,
            width: block_w,
            height: CALLOUT_H,
        };
        let block = Block::bordered()
            .border_style(Style::default().fg(WARN))
            .padding(Padding::horizontal(1));
        let inner = block.inner(callout_rect);
        frame.render_widget(block, callout_rect);
        let lines = vec![
            Line::from(Span::styled(
                "you can change this anytime in /settings",
                Style::default().fg(WARN),
            )),
            Line::from(Span::styled(
                "or type /free to switch to the free tier later",
                Style::default().fg(WARN),
            )),
        ];
        frame.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: true }),
            inner,
        );
        row += CALLOUT_H;
    }
    row += 1; // blank row before the footer

    // Footer key hints.
    render_line(
        frame,
        indented(
            "up/down move \u{00b7} enter select \u{00b7} q quit",
            Style::default().fg(palette.dim),
        ),
        row,
    );
}
