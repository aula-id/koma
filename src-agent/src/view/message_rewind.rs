//! View – message-rewind picker (MessageRewind mode).
//!
//! Rendered as a bordered FLOATING OVERLAY anchored just above the input box
//! (mirroring the `/bash` panel, [`crate::view::bash::render_bash_overlay`]), NOT
//! as a full-screen replacement — the chat transcript stays visible behind it.
//!
//! The overlay lists the conversation's prior user messages in CHRONOLOGICAL order
//! (oldest at the top, newest at the BOTTOM, pre-selected), so the user can pick one
//! to rewind to and edit. Inside the box (top to bottom):
//!
//! 1. Message list — one truncated preview line per entry, oldest→newest. The
//!    selected row is highlighted with `palette.sel_fg` on `palette.sel_bg`. The
//!    list bottom-anchors its scroll to keep the selection visible.
//! 2. One-line keybinding hint.
//!
//! Selection state lives in [`crate::app::mode::RewindState`]. Keystroke handling
//! lives in [`crate::controller::input::handle_rewind`].

use ratatui::{
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Paragraph},
    Frame,
};
use crate::app::mode::RewindState;
use crate::app::state::AppStateRest;
use crate::view::theme::Palette;

/// Collapse a message to a single line and truncate it to at most `max` Unicode
/// scalar values, appending `…` if cut. Newlines/tabs become spaces so a
/// multi-line message stays on one row.
fn preview(s: &str, max: usize) -> String {
    let flat: String = s
        .chars()
        .map(|c| if c == '\n' || c == '\t' || c == '\r' { ' ' } else { c })
        .collect();
    let chars: Vec<char> = flat.chars().collect();
    if chars.len() <= max {
        flat
    } else if max == 0 {
        String::new()
    } else {
        // Reserve one char for the ellipsis.
        let mut out: String = chars[..max.saturating_sub(1)].iter().collect();
        out.push('…');
        out
    }
}

/// Render the message-rewind picker as a bordered overlay anchored just above
/// `input_chunk`, drawn on top of the chat transcript. Mirrors the `/bash` overlay's
/// sizing (up to ~12 rows, clamped to the space above the input, width = input width).
///
/// `input_chunk` / `transcript_chunk` are the same chat layout rects the bash overlay
/// receives (`layout_chunks[3]` and `layout_chunks[1]`): the box is anchored to the
/// input's top edge and clamped so it never overruns the transcript above.
pub fn draw(
    frame: &mut Frame,
    input_chunk: Rect,
    transcript_chunk: Rect,
    rest: &AppStateRest,
    rw: &RewindState,
    palette: &Palette,
) {
    // Box sizing: up to ~12 rows, clamped to the space above the input (matches bash).
    let avail = input_chunk.y.saturating_sub(transcript_chunk.y);
    let h = 12u16.min(avail.max(3));
    let y = input_chunk.y.saturating_sub(h);
    let rect = Rect { x: input_chunk.x, y, width: input_chunk.width, height: h };

    let block = Block::bordered()
        .border_style(Style::default().fg(palette.dim))
        .title(Span::styled(
            " edit a previous message ",
            Style::default().fg(palette.dim),
        ));
    let inner = block.inner(rect);
    crate::view::clear_and_fill(frame, rect, palette.panel);
    frame.render_widget(block, rect);

    if inner.width == 0 || inner.height == 0 {
        // The bordered box itself is the whole signal.
        return;
    }

    // Split the box interior into the message list (fills) + a one-line hint at the
    // bottom. When the box is a single row tall, the list collapses to nothing and
    // only the hint shows.
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(inner);

    // --- Message list (flat, chronological: oldest→newest, newest at the bottom) ---
    let list = rows[0].inner(Margin { horizontal: 1, vertical: 0 });
    if list.width > 0 && list.height > 0 {
        let list_w = list.width as usize;
        let mut lines: Vec<Line> = Vec::with_capacity(rw.entries.len());
        for (i, entry) in rw.entries.iter().enumerate() {
            // Whole inner width is available for the preview (minus a 1-char gutter).
            let text = preview(&entry.content, list_w.saturating_sub(1).max(1));
            let style = if i == rw.selected {
                Style::default().fg(palette.sel_fg).bg(palette.sel_bg)
            } else {
                Style::default().fg(palette.fg)
            };
            lines.push(Line::styled(text, style));
        }

        // Scrolloff window (persisted offset on rest — RewindState is rebuilt per
        // client frame). One row per entry, so window start == scroll offset.
        let list_height = list.height as usize;
        let sel = rw.selected.min(rw.entries.len().saturating_sub(1));
        let (start, _) = crate::view::scroll::scroll_window(
            &rest.rewind_offset,
            sel,
            rw.entries.len(),
            list_height,
        );
        frame.render_widget(Paragraph::new(lines).scroll((start as u16, 0)), list);
    }

    // --- Keybinding hint ---
    let hint = rows[1].inner(Margin { horizontal: 1, vertical: 0 });
    if hint.width > 0 && hint.height > 0 {
        frame.render_widget(
            Paragraph::new("↑↓ select · Enter rewind · Esc cancel")
                .style(Style::default().fg(palette.dim)),
            hint,
        );
    }
}
