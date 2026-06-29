//! View — `/bash` background-job panel (Bash mode).
//!
//! Two-pane master/detail layout, mirroring `/agents`: a narrow sidebar LISTs
//! every background bash job (with a status tag + elapsed); the detail pane on
//! the right shows the SELECTED job read-only — its full command, status, elapsed,
//! and a live output tail. READ-ONLY + kill: there is no editing, no sub-modes.
//!
//! Border convention (strict, matches project rules + `/agents`):
//! - Header: `Borders::BOTTOM` only.
//! - List/detail divider: `Borders::RIGHT` on the list pane.
//! - Footer: full-width inverse hint bar.
//!
//! ```text
//!  bash
//! ─────────────────────────────────────────────────────────
//! │ bash-1  running   2s │  $ cargo build --release
//! │ bash-2  exit 0    9s │  status: running   ·   2s
//! │ bash-3  killed   14s │
//!                       │  Compiling agent v0.1.0
//!                       │  …
//!
//!  ↑/↓ pick · k kill · Esc close
//! ```
//!
//! All cursor state lives in [`crate::app::mode::BashState`]; key handling lives
//! in [`crate::controller::input::handle_bash`].

use ratatui::{
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::app::mode::BashState;
use crate::ipc::proto::BashJobView;
use crate::view::theme::Palette;

/// List (sidebar) column width in terminal columns (includes the RIGHT border).
const SIDEBAR_W: u16 = 28;

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

/// Render the `/bash` panel for `st` using the given colour `palette`.
pub fn draw_bash(frame: &mut Frame, st: &BashState, palette: &Palette) {
    // Outer vertical zones: header | body | footer.
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // header text + BOTTOM border
            Constraint::Min(0),    // list + detail
            Constraint::Length(1), // footer key hints
        ])
        .split(frame.area());

    // --- Header ---
    let header_block = Block::new()
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(palette.dim));
    let header_inner = header_block.inner(outer[0]);
    frame.render_widget(header_block, outer[0]);
    frame.render_widget(
        Paragraph::new(Span::styled("bash", Style::default().fg(palette.dim))),
        header_inner.inner(Margin { horizontal: 2, vertical: 0 }),
    );

    // --- Body: list sidebar + detail pane ---
    let body_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(SIDEBAR_W), // list with RIGHT border as divider
            Constraint::Min(0),            // detail pane
        ])
        .split(outer[1]);

    draw_list(frame, st, palette, body_cols[0]);
    draw_detail(frame, st, palette, body_cols[1]);

    // --- Footer: full-width inverse status bar ---
    let footer_rect = outer[2];
    if footer_rect.width > 0 {
        let hint = "↑/↓ pick · k kill · Esc close";
        let bar_style = Style::default()
            .fg(palette.sel_fg)
            .bg(palette.sel_bg)
            .add_modifier(Modifier::BOLD);
        let padded = format!(
            " {:<width$}",
            hint,
            width = footer_rect.width.saturating_sub(1) as usize
        );
        frame.render_widget(
            Paragraph::new(Line::from(Span::raw(padded))).style(bar_style),
            footer_rect,
        );
    }
}

/// Render the LIST pane: one row per job (`bash-{id}  {status}` + dim elapsed),
/// RIGHT border as the divider.
fn draw_list(frame: &mut Frame, st: &BashState, palette: &Palette, area: Rect) {
    let block = Block::new()
        .borders(Borders::RIGHT)
        .border_style(Style::default().fg(palette.dim));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let content = inner.inner(Margin { horizontal: 1, vertical: 1 });

    let lines: Vec<Line> = if st.jobs.is_empty() {
        vec![Line::from(Span::styled(
            "(no jobs)",
            Style::default().fg(palette.dim),
        ))]
    } else {
        st.jobs
            .iter()
            .enumerate()
            .map(|(i, j)| job_row(j, i == st.selected, content.width as usize, palette))
            .collect()
    };
    frame.render_widget(Paragraph::new(lines), content);
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
        // Focused selection: accent-block highlight across the row (sel_fg/sel_bg),
        // matching the agents/command-palette convention.
        let hl = Style::default().fg(palette.sel_fg).bg(palette.sel_bg);
        Line::from(vec![
            Span::styled("› ", hl),
            Span::styled(format!("{label:<label_w$}"), hl),
            Span::styled(" ", Style::default()),
            Span::styled(elapsed, Style::default().fg(palette.dim)),
        ])
    } else {
        // Non-selected: running rows pop in accent, terminal rows stay dim.
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

/// Render the DETAIL pane: the selected job's header (full command + status +
/// elapsed) then its output tail rendered as lines. Empty list → "no background
/// jobs".
fn draw_detail(frame: &mut Frame, st: &BashState, palette: &Palette, area: Rect) {
    let inner = area.inner(Margin { horizontal: 2, vertical: 1 });

    let Some(j) = st.current() else {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "no background jobs",
                Style::default().fg(palette.dim),
            ))),
            inner,
        );
        return;
    };

    let width = inner.width as usize;
    let mut lines: Vec<Line> = Vec::new();

    // Command header — full command, accent, prefixed `$ ` like a shell prompt.
    lines.push(Line::from(vec![
        Span::styled("$ ", Style::default().fg(palette.dim)),
        Span::styled(
            truncate(&j.command, width.saturating_sub(2).max(4)),
            Style::default().fg(palette.accent),
        ),
    ]));

    // Status + elapsed line — status colour matches the row state.
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

    // Spacer, then the captured output tail (one Line per source line).
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

    frame.render_widget(Paragraph::new(lines), inner);
}
