//! View — `/bash` background-job panel overlay.
//!
//! Rendered as a bordered overlay anchored above the input box (mirroring the
//! sub-agents panel), NOT as a full-screen replacement. The chat transcript
//! remains visible behind the overlay. Two-pane layout (list | detail) inside
//! the box.

use ratatui::{
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

use crate::ipc::proto::BashJobView;
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

/// Build one sidebar row for job `j`. The id+status take the left; the elapsed
/// is a dim right-hand suffix. The selected row carries the inverse highlight; a
/// non-selected running row is accent, a finished one dim, killed/errored dim.
fn job_row<'a>(j: &BashJobView, selected: bool, width: usize, palette: &Palette) -> Line<'a> {
    let elapsed = format!("{}s", j.elapsed_secs);
    // Reserve room for a leading marker (2) + a space + the elapsed suffix.
    let label_w = width
        .saturating_sub(2) // "› " / "  " marker
        .saturating_sub(elapsed.chars().count() + 1) // " {elapsed}"
        .max(4);
    let label = truncate(&format!("bash-{}  {}", j.id, j.status), label_w);

    if selected {
        let hl = Style::default().fg(palette.sel_fg).bg(palette.sel_bg);
        Line::from(vec![
            Span::styled("› ", hl),
            Span::styled(format!("{label:<label_w$}"), hl),
            Span::styled(" ", Style::default()),
            Span::styled(elapsed, Style::default().fg(palette.dim)),
        ])
    } else {
        let name_style = if j.running {
            Style::default().fg(palette.accent)
        } else {
            Style::default().fg(palette.dim)
        };
        Line::from(vec![
            Span::styled("  ", Style::default().fg(palette.dim)),
            Span::styled(format!("{label:<label_w$}"), name_style),
            Span::styled(" ", Style::default()),
            Span::styled(elapsed, Style::default().fg(palette.dim)),
        ])
    }
}

/// Build detail lines for a single job — the `$ command` header, status line,
/// spacer, then output tail lines. Used by the right pane of the overlay.
fn detail_lines<'a>(j: &BashJobView, width: usize, palette: &Palette) -> Vec<Line<'a>> {
    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::from(vec![
        Span::styled("$ ", Style::default().fg(palette.dim)),
        Span::styled(
            truncate(&j.command, width.saturating_sub(2).max(4)),
            Style::default().fg(palette.accent),
        ),
    ]));

    let status_style = if j.running {
        Style::default().fg(palette.accent)
    } else {
        Style::default().fg(palette.fg)
    };
    lines.push(Line::from(vec![
        Span::styled("status: ", Style::default().fg(palette.dim)),
        Span::styled(j.status.clone(), status_style),
        Span::styled("   ·   ", Style::default().fg(palette.dim)),
        Span::styled(format!("{}s", j.elapsed_secs), Style::default().fg(palette.dim)),
    ]));

    lines.push(Line::from(""));
    let out_w = width.max(4);
    if j.output_tail.trim().is_empty() {
        lines.push(Line::from(Span::styled(
            "(no output yet)",
            Style::default().fg(palette.dim),
        )));
    } else {
        for raw in j.output_tail.lines() {
            lines.push(Line::from(Span::styled(
                truncate(raw, out_w),
                Style::default().fg(palette.fg),
            )));
        }
    }

    lines
}

/// Render the `/bash` panel as a bordered overlay anchored just above
/// `input_chunk`, drawn on top of the chat transcript. Mirrors the
/// sub-agents overlay layout (list LEFT + detail RIGHT).
pub fn render_bash_overlay(
    frame: &mut Frame,
    input_chunk: Rect,
    transcript_chunk: Rect,
    jobs: &[BashJobView],
    selected: usize,
    palette: &Palette,
) {
    // Box sizing: up to ~12 rows, clamped to the space above the input.
    let avail = input_chunk.y.saturating_sub(transcript_chunk.y);
    let h = 12u16.min(avail.max(3));
    let y = input_chunk.y.saturating_sub(h);
    let rect = Rect { x: input_chunk.x, y, width: input_chunk.width, height: h };

    let block = Block::bordered()
        .border_style(Style::default().fg(palette.dim))
        .title(Span::styled(" bash ", Style::default().fg(palette.dim)));
    let inner = block.inner(rect);
    frame.render_widget(Clear, rect);
    frame.render_widget(block, rect);

    if inner.width == 0 || inner.height == 0 {
        // The bordered box itself is the whole signal.
        return;
    }

    if jobs.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "(no background jobs)",
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

    // LEFT: one row per job, selected row highlighted.
    let list_block = Block::new()
        .borders(Borders::RIGHT)
        .border_style(Style::default().fg(palette.dim));
    let list_inner = list_block.inner(cols[0]);
    frame.render_widget(list_block, cols[0]);

    let sel = selected.min(jobs.len().saturating_sub(1));
    let list_w = list_inner.width as usize;
    let list_lines: Vec<Line> = jobs
        .iter()
        .enumerate()
        .map(|(i, j)| job_row(j, i == sel, list_w, palette))
        .collect();
    frame.render_widget(Paragraph::new(list_lines), list_inner);

    // RIGHT: selected job detail.
    let right = cols[1].inner(Margin { horizontal: 1, vertical: 0 });
    if right.width == 0 || right.height == 0 {
        return;
    }
    frame.render_widget(
        Paragraph::new(detail_lines(&jobs[sel], right.width as usize, palette)),
        right,
    );
}
