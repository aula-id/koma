//! Input box rendering: multiline editor with caret, compact animation, and
//! the session-name tab on the top border.

use ratatui::{
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph, Wrap},
    Frame,
};
use crate::app::state::AppStateRest;
use crate::view::theme::Palette;
use super::helpers::render_compact_anim;

/// Prefix width in columns: "[$] " on line 1, "    " on continuations.
const PREFIX_W: usize = 4;

/// Compute how many inner rows the input box needs.
///
/// Used by the caller to reserve the correct height in the layout split before
/// any widgets are rendered. Capped at 50% of the terminal height so the input
/// can never eat the transcript.
///
/// The input `Block` uses `Borders::TOP | Borders::BOTTOM` only (no side borders)
/// plus `Padding::horizontal(2)`, so inner width = frame_width - 4 (2+2 padding).
/// This matches `render_editor`'s `area.width` from `input_block.inner(chunk)`.
pub(super) fn input_row_count(rest: &AppStateRest, frame_width: u16, frame_height: u16) -> usize {
    let inner_w = (frame_width.saturating_sub(4) as usize).max(1);
    let mut input_rows = 0usize;
    for line in rest.fg().input.split('\n') {
        let prefixed = line.chars().count() + PREFIX_W;
        input_rows += 1usize.max(prefixed.div_ceil(inner_w));
    }
    if rest.compact_anim_start.is_some() {
        return 2;
    }
    let max_inner = ((frame_height / 2).saturating_sub(2) as usize).max(1);
    input_rows.clamp(1, max_inner)
}

/// Render the input block (borders + either the compact animation or the
/// multiline editor) into `chunk`.
pub(super) fn render_input(frame: &mut Frame, chunk: Rect, rest: &AppStateRest, palette: &Palette) {
    let session_tab: Option<Span<'static>> = rest.fg().session.as_ref().map(|s| {
        Span::styled(
            format!(" {} ", s.name.clone()),
            Style::default().fg(palette.accent),
        )
    });
    let input_block = if let Some(tab) = session_tab {
        Block::new()
            .borders(Borders::TOP | Borders::BOTTOM)
            .border_style(Style::default().fg(palette.dim))
            .title(tab)
            .title_alignment(Alignment::Right)
            .padding(Padding::horizontal(2))
    } else {
        Block::new()
            .borders(Borders::TOP | Borders::BOTTOM)
            .border_style(Style::default().fg(palette.dim))
            .padding(Padding::horizontal(2))
    };
    let input_inner = input_block.inner(chunk);
    frame.render_widget(input_block, chunk);
    if let Some(start) = rest.compact_anim_start {
        render_compact_anim(frame, input_inner, start, palette);
    } else {
        render_editor(frame, input_inner, rest, palette);
    }
}

/// Render the multiline editor (the normal `[$] {input}` state) into `area`.
///
/// Logical lines split on '\n'; the first carries the accent prompt, every
/// continuation a 4-col indent so it hangs under the prompt. A non-blinking
/// block caret is painted AT `rest.cursor` (a char index into the whole input,
/// counting the '\n's), so mid-text edits show the caret in place rather than
/// always at the end.
///
/// When the content exceeds the available height, the editor scrolls vertically
/// to keep the caret visible (scroll-to-caret, bottom-anchored like a phone
/// keyboard).
fn render_editor(frame: &mut Frame, area: Rect, rest: &AppStateRest, palette: &Palette) {
    let inner_w = (area.width as usize).max(1);
    let cursor = rest.fg().cursor;
    let logicals: Vec<&str> = rest.fg().input.split('\n').collect();
    let last_idx = logicals.len().saturating_sub(1);

    // Total wrapped visual lines (uncapped) and caret's wrapped row.
    let mut total_vis: usize = 0;
    let mut caret_vis: usize = 0;
    let mut caret_found = false;

    let mut input_lines: Vec<Line> = Vec::new();

    // Running char offset accumulator: avoids O(L^2) re-walks from line 0.
    let mut char_start = 0usize;

    for (i, logical) in logicals.iter().enumerate() {
        let line_chars = logical.chars().count();
        let on_this_line = if i == last_idx {
            cursor >= char_start && cursor <= char_start + line_chars
        } else {
            cursor >= char_start && cursor < char_start + line_chars + 1
        };

        // Build spans for this logical line.
        let prefix: Span = if i == 0 {
            Span::styled("[$] ", Style::default().fg(palette.accent))
        } else {
            Span::raw("    ")
        };
        let mut spans: Vec<Span> = vec![prefix];

        if on_this_line {
            let col = (cursor - char_start).min(line_chars);
            let before: String = logical.chars().take(col).collect();
            let at: String = logical.chars().nth(col).map(String::from).unwrap_or_default();
            let after: String = logical.chars().skip(col + 1).collect();
            if !before.is_empty() {
                spans.push(Span::raw(before));
            }
            if at.is_empty() {
                spans.push(Span::styled("\u{2588}", Style::default().fg(palette.accent)));
            } else {
                spans.push(Span::styled(
                    at,
                    Style::default().add_modifier(Modifier::REVERSED),
                ));
            }
            if !after.is_empty() {
                spans.push(Span::raw(after));
            }
        } else {
            spans.push(Span::raw(*logical));
        }

        // Wrapped visual rows for this logical line (prefix + content).
        let full_w = PREFIX_W + line_chars;
        let rows = 1usize.max(full_w.div_ceil(inner_w));

        if on_this_line {
            // Caret's column within the full prefixed line.
            let caret_col = PREFIX_W + (cursor - char_start).min(line_chars);
            // Visual rows consumed before this line + caret's row within it
            // (floor division: caret_col is a 0-based column index, not a count).
            caret_vis = total_vis + caret_col / inner_w;
            caret_found = true;
        }

        total_vis += rows;
        input_lines.push(Line::from(spans));

        // Advance accumulator: +1 for the '\n' separator between logical lines.
        char_start += line_chars + 1;
    }

    if !caret_found {
        caret_vis = total_vis;
    }

    // Scroll-to-caret: bottom-anchored. The caret stays on the last visible
    // row when possible, like a phone keyboard.
    let viewport_h = area.height as usize;
    let scroll = if total_vis > viewport_h {
        let desired = caret_vis.saturating_sub(viewport_h.saturating_sub(1));
        let max_scroll = total_vis.saturating_sub(viewport_h);
        (desired.min(max_scroll)) as u16
    } else {
        0
    };

    frame.render_widget(
        Paragraph::new(input_lines)
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0)),
        area,
    );
}
