//! View – full-screen, searchable command/keybinding reference + launcher
//! (Help mode).
//!
//! Replaces the old floating `/help` overlay (which clipped — it had no scroll).
//! Layout (top to bottom), matching the house border convention used by
//! `/mcp` + the `--resume` picker:
//!
//! 1. Header: ` help ` on a `Borders::BOTTOM` rule (dim).
//! 2. Search line: the live `query` with a block cursor.
//! 3. Filtered list: one row per entry — the key in accent + the description in
//!    fg, the selected row highlighted (`sel_fg`/`sel_bg`). The list WINDOWS
//!    (MAX_VIS + a slice around `selected`) so it scrolls and the selection
//!    stays visible — the old overlay's no-scroll clipping is not repeated.
//! 4. Footer: full-width inverse hint bar (same as `/mcp`).
//!
//! Filtering/selection state lives in [`crate::app::mode::HelpState`]; keystroke
//! handling in [`crate::controller::input::help`].

use ratatui::{
    layout::{Constraint, Direction, Layout, Margin},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::app::mode::{HelpKind, HelpState};
use crate::view::theme::Palette;

/// Width the key column is padded to (so descriptions align in a column).
const KEY_W: usize = 16;

/// Hardcoded "newer version available" green (NOT themeable — matches the header
/// badge in `view::chat::header`).
const GREEN: Color = Color::Rgb(57, 255, 20);

/// Render the help reference for `st` using the given colour `palette`.
///
/// Colours flow through `palette`, except the fixed `GREEN` "update available"
/// accent in the "Updating koma" block (it mirrors the non-themeable header badge).
pub fn draw(frame: &mut Frame, st: &HelpState, palette: &Palette) {
    // Height of the "Updating koma" block: label + current/available + run-command,
    // plus one extra line when the update carries a release message. A trailing
    // spacer line separates it from the search row.
    let has_msg = matches!(&st.update, Some((_, Some(_))));
    let update_h = if has_msg { 4 } else { 3 } + 1; // +1 spacer

    // Outer vertical zones: header | update block | search | list | footer.
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),                 // header text + BOTTOM border
            Constraint::Length(update_h as u16),   // "Updating koma" info block + spacer
            Constraint::Length(2),                 // search line + spacer
            Constraint::Min(0),                    // filtered list
            Constraint::Length(1),                 // footer hint
        ])
        .split(frame.area());

    // --- Header ---
    let header_block = Block::new()
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(palette.dim));
    let header_inner = header_block.inner(outer[0]);
    frame.render_widget(header_block, outer[0]);
    frame.render_widget(
        Paragraph::new(Span::styled("help", Style::default().fg(palette.dim))),
        header_inner.inner(Margin {
            horizontal: 2,
            vertical: 0,
        }),
    );

    // --- "Updating koma" info block (dim, non-interactive — sits above search). ---
    let update_inner = outer[1].inner(Margin {
        horizontal: 2,
        vertical: 0,
    });
    let dim = Style::default().fg(palette.dim);
    let mut update_lines: Vec<Line> = Vec::with_capacity(4);
    // Section label.
    update_lines.push(Line::from(Span::styled("updating koma", dim)));
    // Current (+ available [latest] when an update is known).
    let mut cur_spans = vec![Span::styled(
        format!("  current {}", st.current_version),
        dim,
    )];
    if let Some((latest, _)) = &st.update {
        cur_spans.push(Span::styled("  ·  available ", dim));
        cur_spans.push(Span::styled(format!("[{latest}]"), Style::default().fg(GREEN)));
    }
    update_lines.push(Line::from(cur_spans));
    // How to update.
    update_lines.push(Line::from(Span::styled(
        "  run  koma update  in your shell  (or  curl -fsSL https://koma.run/install.sh | sh )",
        dim,
    )));
    // Optional release message.
    if let Some((_, Some(msg))) = &st.update {
        update_lines.push(Line::from(Span::styled(format!("  {msg}"), dim)));
    }
    frame.render_widget(Paragraph::new(update_lines), update_inner);

    // --- Search line (live query + block cursor) ---
    let search_inner = outer[2].inner(Margin {
        horizontal: 2,
        vertical: 0,
    });
    let search_line = Line::from(vec![
        Span::styled("› ", Style::default().fg(palette.dim)),
        Span::styled(st.query.as_str(), Style::default().fg(palette.fg)),
        Span::styled("█", Style::default().fg(palette.accent)),
    ]);
    frame.render_widget(Paragraph::new(search_line), search_inner);

    // --- Filtered list (windowed so the selection stays visible) ---
    let list_inner = outer[3].inner(Margin {
        horizontal: 2,
        vertical: 0,
    });
    let max_vis = list_inner.height as usize;

    if st.filtered_idx.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "no matches",
                Style::default().fg(palette.dim),
            )),
            list_inner,
        );
    } else if max_vis > 0 {
        let sel = st.selected.min(st.filtered_idx.len() - 1);
        // Window start keeps `sel` visible (anchors to the bottom when scrolling
        // down past the viewport) — the same pattern as the file palette.
        let start = if sel < max_vis {
            0
        } else {
            sel + 1 - max_vis
        };
        let end = (start + max_vis).min(st.filtered_idx.len());

        let rows: Vec<Line> = st.filtered_idx[start..end]
            .iter()
            .enumerate()
            .map(|(vi, &ai)| {
                let i = start + vi;
                let entry = &st.all[ai];
                let key = format!(" {:<KEY_W$}", entry.key);
                if i == sel {
                    let hl = Style::default().fg(palette.sel_fg).bg(palette.sel_bg);
                    Line::from(vec![
                        Span::styled(key, hl),
                        Span::styled(format!("{} ", entry.desc), hl),
                    ])
                } else {
                    // Commands get the accent key (they're launchable); keybindings
                    // get a dimmer key so the two groups read apart in the flat list.
                    let key_style = match entry.kind {
                        HelpKind::Command => Style::default().fg(palette.accent),
                        HelpKind::Keybinding => Style::default().fg(palette.dim),
                    };
                    Line::from(vec![
                        Span::styled(key, key_style),
                        Span::styled(entry.desc.clone(), Style::default().fg(palette.fg)),
                    ])
                }
            })
            .collect();

        frame.render_widget(Paragraph::new(rows), list_inner);
    }

    // --- Footer: full-width inverse hint bar (matches /mcp). ---
    let footer_rect = outer[4];
    if footer_rect.width > 0 {
        let hint = "type to search · ↑/↓ select · Enter run · Esc close";
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
