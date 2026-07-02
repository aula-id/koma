//! View — `/todo` task-panel overlay.
//!
//! Rendered as a bordered overlay anchored above the input box (mirroring the
//! `/bash` panel), NOT as a full-screen replacement. The chat transcript
//! remains visible behind the overlay. Two-pane layout (list | detail) inside
//! the box.

use ratatui::{
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use crate::app::mode::todo::{TodoItem, TodoStatus};
use crate::view::theme::Palette;

/// Truncate `s` to at most `max` chars, appending `…` if cut.
fn truncate(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        s.to_string()
    } else {
        let cut = max.saturating_sub(1);
        chars[..cut].iter().collect::<String>() + "…"
    }
}

fn spinner_glyph() -> &'static str {
    const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    SPINNER[((now_ms / 80) as usize) % SPINNER.len()]
}

/// Status symbol for a todo item row.
fn status_symbol(s: &TodoStatus) -> &'static str {
    match s {
        TodoStatus::Pending => "○",
        TodoStatus::InProgress => "◐",
        TodoStatus::Completed => "●",
        TodoStatus::Cancelled => "⊘",
    }
}

/// Animated status symbol — spinner for InProgress, same as row for others.
fn status_symbol_animated(s: &TodoStatus) -> String {
    match s {
        TodoStatus::InProgress => spinner_glyph().to_string(),
        other => status_symbol(other).to_string(),
    }
}

/// Build one sidebar row for a todo item. The status symbol + content fill the
/// full width. The selected row carries the inverse highlight.
fn todo_row<'a>(item: &TodoItem, selected: bool, width: usize, palette: &Palette) -> Line<'a> {
    let sym = status_symbol_animated(&item.status);
    let label = truncate(&item.content, width.saturating_sub(sym.chars().count() + 1));

    if selected {
        let hl = Style::default().fg(palette.sel_fg).bg(palette.sel_bg);
        Line::from(vec![
            Span::styled(format!("{sym} "), hl),
            Span::styled(label, hl),
        ])
    } else {
        let name_style = match item.status {
            TodoStatus::Completed | TodoStatus::Cancelled => Style::default().fg(palette.dim),
            _ => Style::default().fg(palette.fg),
        };
        Line::from(vec![
            Span::styled(format!("{sym} "), Style::default().fg(palette.dim)),
            Span::styled(label, name_style),
        ])
    }
}

/// Build detail lines for a single todo item — status+priority at top,
/// awaiting/state text, then content as description. Used by the right pane.
fn detail_lines<'a>(item: &TodoItem, palette: &Palette) -> Vec<Line<'a>> {
    let mut lines: Vec<Line> = Vec::new();

    // Status + priority at top
    let status_style = match item.status {
        TodoStatus::Completed | TodoStatus::Cancelled => Style::default().fg(palette.dim),
        TodoStatus::InProgress => Style::default().fg(palette.accent),
        _ => Style::default().fg(palette.fg),
    };
    lines.push(Line::from(vec![
        Span::styled("status: ", Style::default().fg(palette.dim)),
        Span::styled(item.status.display().to_string(), status_style),
        Span::styled("   ·   ", Style::default().fg(palette.dim)),
        Span::styled("priority: ", Style::default().fg(palette.dim)),
        Span::styled(item.priority.label().to_string(), Style::default().fg(palette.fg)),
    ]));

    // State hint text
    match item.status {
        TodoStatus::Pending => {
            lines.push(Line::from(Span::styled(
                "(awaiting model or user action)",
                Style::default().fg(palette.dim),
            )));
        }
        TodoStatus::InProgress => {
            lines.push(Line::from(Span::styled(
                "(currently being worked on)",
                Style::default().fg(palette.accent),
            )));
        }
        TodoStatus::Completed => {
            lines.push(Line::from(Span::styled(
                "(done)",
                Style::default().fg(palette.dim),
            )));
        }
        TodoStatus::Cancelled => {
            lines.push(Line::from(Span::styled(
                "(cancelled)",
                Style::default().fg(palette.dim),
            )));
        }
    }

    lines.push(Line::from(""));

    // Content as description — each word is a separate span for natural wrapping.
    lines.push(Line::from(vec![
        Span::styled("content:", Style::default().fg(palette.dim)),
    ]));
    let content_spans: Vec<Span> = item
        .content
        .split_whitespace()
        .enumerate()
        .flat_map(|(i, word)| {
            let mut spans = vec![Span::styled(word.to_string(), Style::default().fg(palette.fg))];
            // Add a space between words, but not after the last word.
            if i + 1 < item.content.split_whitespace().count() {
                spans.push(Span::raw(" "));
            }
            spans
        })
        .collect();
    if content_spans.is_empty() {
        lines.push(Line::from(""));
    } else {
        lines.push(Line::from(content_spans));
    }

    lines
}

/// Render the `/todo` panel as a bordered overlay anchored just above
/// `input_chunk`, drawn on top of the chat transcript. Mirrors the
/// `/bash` overlay layout (list LEFT + detail RIGHT).
pub fn render_todo_overlay(
    frame: &mut Frame,
    input_chunk: Rect,
    transcript_chunk: Rect,
    items: &[TodoItem],
    selected: usize,
    completed_count: usize,
    palette: &Palette,
) {
    // Box sizing: up to ~12 rows, clamped to the space above the input.
    let avail = input_chunk.y.saturating_sub(transcript_chunk.y);
    let h = 12u16.min(avail.max(3));
    let y = input_chunk.y.saturating_sub(h);
    let rect = Rect { x: input_chunk.x, y, width: input_chunk.width, height: h };

    let total = items.len();
    let title = format!(" todo ({}/{}) ", completed_count, total);
    let block = Block::bordered()
        .border_style(Style::default().fg(palette.dim))
        .title(Span::styled(title, Style::default().fg(palette.dim)));
    let inner = block.inner(rect);
    frame.render_widget(Clear, rect);
    frame.render_widget(block, rect);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    if items.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "(no todos — model can add with todowrite tool)",
                Style::default().fg(palette.dim),
            )),
            inner.inner(Margin { horizontal: 1, vertical: 0 }),
        );
        return;
    }

    // Two-pane split: narrow left list (RIGHT border divider) + wide right detail.
    const LIST_W: u16 = 24;
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(LIST_W), Constraint::Min(0)])
        .split(inner);

    // LEFT: one row per item, selected row highlighted.
    let list_block = Block::new()
        .borders(Borders::RIGHT)
        .border_style(Style::default().fg(palette.dim));
    let list_inner = list_block.inner(cols[0]);
    frame.render_widget(list_block, cols[0]);

    let sel = selected.min(items.len().saturating_sub(1));
    let list_w = list_inner.width as usize;
    let list_h = list_inner.height as usize;
    let list_lines: Vec<Line> = items
        .iter()
        .enumerate()
        .map(|(i, item)| todo_row(item, i == sel, list_w, palette))
        .collect();
    // Scroll so the selected row is always visible.
    let scroll = if list_h > 0 {
        let top = sel.saturating_sub(list_h.saturating_sub(1));
        top as u16
    } else {
        0
    };
    frame.render_widget(Paragraph::new(list_lines).scroll((scroll, 0)), list_inner);

    // RIGHT: selected item detail.
    let right = cols[1].inner(Margin { horizontal: 1, vertical: 0 });
    if right.width == 0 || right.height == 0 {
        return;
    }
    frame.render_widget(
        Paragraph::new(detail_lines(&items[sel], palette)).wrap(Wrap { trim: true }),
        right,
    );
}
